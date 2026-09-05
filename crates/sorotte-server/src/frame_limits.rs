//! Recipient framing admission. A roster remains a complete replacement;
//! optional discovery signatures may be omitted under pressure. Growth is
//! checked against a read-only projected session before authority changes.
use super::*;

const FRAME_ENVELOPE_RESERVE: usize = 512;
// A combined barrier snapshot may contain prepare and recovery logical IDs,
// plus request IDs in prepare, policy, nested status, and recovery. Reserve
// their maximum JSON escaping separately from the per-participant schemas.
const COORDINATION_FIXED_RESERVE: usize = 2 * PLAYBACK_BARRIER_MAX_LOGICAL_MEDIA_ID_CHARS * 6
    + 8 * PLAYBACK_BARRIER_MAX_REQUEST_ID_BYTES * 6
    + 4096;

impl ServerClientCapabilities {
    pub(crate) fn frame_limit(&self) -> usize {
        if self.large_protocol_frames_v1 {
            SOROTTE_MAX_PROTOCOL_LINE_BYTES
        } else if self.readiness_v2
            || self.participant_status_v1
            || self.playback_barrier_v1
            || self.media_match
        {
            // Prior Rust releases advertise these extensions but do not
            // negotiate the larger common transport contract.
            DEFAULT_MAX_PROTOCOL_LINE_BYTES
        } else {
            LEGACY_MAX_PROTOCOL_LINE_BYTES
        }
    }
}

pub(crate) fn frame_capacity_error() -> ServerRuntimeError {
    ServerRuntimeError::Protocol(ProtocolError::ServerError {
        message: "Room state exceeds a recipient's protocol frame capacity".to_owned(),
    })
}

pub(crate) fn empty_room_identity(index: usize) -> String {
    // Legacy clients recognize all-whitespace names as dummy entries. Binary
    // whitespace preserves distinct identities without quadratic padding.
    if index == 1 {
        return " ".to_owned();
    }
    let mut identity = String::from(" ");
    for bit in format!("{index:b}").bytes() {
        identity.push(if bit == b'0' { ' ' } else { '\t' });
    }
    identity
}

fn omit_discovery_signatures(rooms: &mut BTreeMap<String, BTreeMap<String, ListUserEntry>>) {
    for entry in rooms.values_mut().flat_map(|room| room.values_mut()) {
        if let Some(file) = entry.file.as_mut().and_then(Value::as_object_mut) {
            file.remove("mediaMatch");
        }
    }
}

impl ServerRuntime {
    pub(crate) fn frame_limit_for_capabilities(
        &self,
        capabilities: &ServerClientCapabilities,
    ) -> usize {
        capabilities
            .frame_limit()
            .min(self.resource_limits.queued_bytes_per_peer.saturating_sub(2))
    }

    pub(crate) fn known_rooms_for_frame_projection(&self) -> BTreeSet<String> {
        self.room_controllers
            .keys()
            .chain(self.room_playlists.keys())
            .chain(self.room_playback_states.keys())
            .cloned()
            .collect()
    }

    pub(crate) fn check_projected_playlist_frames(
        &self,
        client_id: &str,
        candidate: &ServerSession,
        changes: &BTreeMap<String, Vec<String>>,
    ) -> Result<(), ServerRuntimeError> {
        let mut bytes = 0usize;
        for (_, peer) in self
            .sessions
            .iter()
            .filter(|(id, _)| id.as_str() != client_id)
            .map(|(id, peer)| (id.as_str(), peer))
            .chain(std::iter::once((client_id, candidate)))
        {
            let Some(files) = changes.get(&peer.room) else {
                continue;
            };
            let message = ProtocolMessage::set(
                SetPayload::new().with_playlist_change(
                    playlist_change_with_plex_sidecar(
                        files.clone(),
                        peer.capabilities.plex_playlist_uris,
                    )
                    .with_playlist_epoch(u64::MAX)
                    .with_user(&candidate.username),
                ),
            );
            if !message_fits_line_limit(
                &message,
                self.frame_limit_for_capabilities(&peer.capabilities)
                    .saturating_sub(FRAME_ENVELOPE_RESERVE),
            )? {
                return Err(frame_capacity_error());
            }
            bytes = bytes.saturating_add(encode_message_line(&message)?.len().saturating_add(2));
            if bytes > self.resource_limits.queued_bytes_total / 4 {
                return Err(frame_capacity_error());
            }
        }
        Ok(())
    }

    pub(crate) fn check_fanout_allocation(
        &self,
        message: &ProtocolMessage,
        recipients: usize,
    ) -> Result<(), ServerRuntimeError> {
        if !message_fits_line_limit(
            message,
            MAX_PROTOCOL_LINE_BYTES
                .min(self.resource_limits.queued_bytes_per_peer.saturating_sub(2)),
        )? {
            return Err(frame_capacity_error());
        }
        let bytes = encode_message_line(message)?.len().saturating_add(2);
        if bytes
            .checked_mul(recipients)
            .is_none_or(|bytes| bytes > self.resource_limits.queued_bytes_total)
        {
            return Err(frame_capacity_error());
        }
        Ok(())
    }
    pub(crate) fn recipient_frame_limit(&self, client_id: &str) -> usize {
        let negotiated = self
            .sessions
            .get(client_id)
            .map(|session| session.capabilities.frame_limit())
            .unwrap_or(LEGACY_MAX_PROTOCOL_LINE_BYTES);
        negotiated.min(self.resource_limits.queued_bytes_per_peer.saturating_sub(2))
    }

    pub(crate) fn compact_list_for_limit(
        &self,
        client_id: &str,
        rooms: &mut BTreeMap<String, BTreeMap<String, ListUserEntry>>,
    ) {
        let message = ProtocolMessage::list(ListPayload::rooms(rooms.clone()));
        let limit = self
            .recipient_frame_limit(client_id)
            .min(self.resource_limits.queued_bytes_total / 4 / self.sessions.len().max(1));
        if !message_fits_line_limit(&message, limit).unwrap_or(false) {
            omit_discovery_signatures(rooms);
        }
    }

    pub(crate) fn check_session_frame_projection(
        &self,
        client_id: &str,
        candidate: &ServerSession,
    ) -> Result<(), ServerRuntimeError> {
        let known_rooms = self.known_rooms_for_frame_projection();
        self.check_frame_projection_with_config(
            client_id,
            candidate,
            self.isolate_rooms,
            &known_rooms,
        )
    }

    pub(crate) fn check_frame_projection_with_config(
        &self,
        client_id: &str,
        candidate: &ServerSession,
        isolate_rooms: bool,
        known_rooms: &BTreeSet<String>,
    ) -> Result<(), ServerRuntimeError> {
        let projected: Vec<_> = self
            .sessions
            .iter()
            .filter(|(id, _)| id.as_str() != client_id)
            .map(|(id, session)| (id.as_str(), session))
            .chain(std::iter::once((client_id, candidate)))
            .collect();
        if let Some(file) = &candidate.file {
            let update = user_file_update_message(
                &candidate.username,
                &candidate.room,
                file.to_wire_value(true),
            );
            let recipients = projected
                .iter()
                .filter(|(_, peer)| !isolate_rooms || peer.room == candidate.room)
                .count();
            self.check_fanout_allocation(&update, recipients)?;
        }
        let mut checked_views = BTreeSet::new();
        let mut projected_fanout_bytes = 0usize;
        let mut coordination_fanout_bytes = 0usize;
        for (recipient_id, recipient) in &projected {
            let limit = self
                .frame_limit_for_capabilities(&recipient.capabilities)
                .saturating_sub(FRAME_ENVELOPE_RESERVE);
            if (!isolate_rooms || recipient.room == candidate.room)
                && let Some(file) = &candidate.file
            {
                let update = user_file_update_message(
                    &candidate.username,
                    &candidate.room,
                    file.to_wire_value(
                        recipient.capabilities.media_match && *recipient_id != client_id,
                    ),
                );
                if !message_fits_line_limit(
                    &update,
                    self.frame_limit_for_capabilities(&recipient.capabilities),
                )? {
                    return Err(frame_capacity_error());
                }
            }
            let files = self.room_playlist_state(&recipient.room).files;
            let playlist = ProtocolMessage::set(
                SetPayload::new().with_playlist_change(
                    playlist_change_with_plex_sidecar(
                        files,
                        recipient.capabilities.plex_playlist_uris,
                    )
                    .with_playlist_epoch(u64::MAX)
                    .with_user(&candidate.username),
                ),
            );
            if !message_fits_line_limit(&playlist, limit)? {
                return Err(frame_capacity_error());
            }
            // Reserve representation for every future state of the closed
            // readiness/barrier schemas, not only the small initial state.
            // 2048 covers fixed keys/enums, maximal numeric representations,
            // and the 128-byte operation ID escaped at six bytes per byte.
            // Six encoded names cover key, username and controller metadata.
            if recipient.capabilities.readiness_v2 || recipient.capabilities.playback_barrier_v1 {
                let mut cohort: BTreeSet<&str> = projected
                    .iter()
                    .filter(|(_, peer)| peer.room == recipient.room)
                    .map(|(_, peer)| peer.username.as_str())
                    .collect();
                // A live generation retains disconnected participants. New
                // arrivals must still be able to read that retained snapshot.
                if let Some(barrier) = self.room_playback_barriers.get(&recipient.room) {
                    cohort.extend(
                        barrier
                            .participants
                            .values()
                            .map(|peer| peer.username.as_str()),
                    );
                    cohort.extend(barrier.excluded_legacy_clients.iter().map(String::as_str));
                }
                let max_name = cohort
                    .iter()
                    .map(|name| serde_json::to_string(name).map(|name| name.len()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(ProtocolError::from)?
                    .into_iter()
                    .max()
                    .unwrap_or(0);
                let per_member = 2048usize.saturating_add(max_name.saturating_mul(6));
                let coordination_bytes = cohort
                    .len()
                    .saturating_mul(per_member)
                    .saturating_add(COORDINATION_FIXED_RESERVE + max_name);
                if coordination_bytes > limit {
                    return Err(frame_capacity_error());
                }
                coordination_fanout_bytes =
                    coordination_fanout_bytes.saturating_add(coordination_bytes);
                if coordination_fanout_bytes > self.resource_limits.queued_bytes_total / 4 {
                    return Err(frame_capacity_error());
                }
            }
            let scope = if isolate_rooms {
                recipient.room.as_str()
            } else {
                ""
            };
            if !checked_views.insert((
                scope,
                limit,
                recipient.capabilities.is_gui_user(),
                recipient.capabilities.media_match,
            )) {
                continue;
            }
            let mut rooms: BTreeMap<String, BTreeMap<String, ListUserEntry>> = BTreeMap::new();
            for (_, source) in &projected {
                if isolate_rooms && recipient.room != source.room {
                    continue;
                }
                rooms.entry(source.room.clone()).or_default().insert(
                    source.username.clone(),
                    ListUserEntry::new()
                        .with_position(0.0)
                        .with_file(
                            source
                                .file
                                .as_ref()
                                .map(|file| file.to_wire_value(recipient.capabilities.media_match))
                                .unwrap_or_else(|| json!({})),
                        )
                        .with_controller(false)
                        .with_is_ready(false)
                        .with_features(source.capabilities.to_wire_value()),
                );
            }
            if recipient.capabilities.is_gui_user() {
                for (index, room) in known_rooms.iter().enumerate() {
                    if !projected.iter().any(|(_, session)| session.room == *room) {
                        rooms
                            .entry(room.clone())
                            .or_default()
                            .insert(empty_room_identity(index + 1), legacy_dummy_list_entry());
                    }
                }
            }
            let mut message = ProtocolMessage::list(ListPayload::rooms(rooms));
            let snapshot_limit =
                limit.min(self.resource_limits.queued_bytes_total / 4 / projected.len().max(1));
            if !message_fits_line_limit(&message, snapshot_limit)? {
                if let ProtocolMessage::List(list) = &mut message
                    && let ListPayload::Rooms(rooms) = &mut list.list
                {
                    omit_discovery_signatures(rooms);
                }
                if !message_fits_line_limit(&message, snapshot_limit)? {
                    return Err(frame_capacity_error());
                }
            }
            let view_recipients = projected
                .iter()
                .filter(|(_, peer)| {
                    (!isolate_rooms || peer.room == recipient.room)
                        && self.frame_limit_for_capabilities(&peer.capabilities)
                            == self.frame_limit_for_capabilities(&recipient.capabilities)
                        && peer.capabilities.is_gui_user() == recipient.capabilities.is_gui_user()
                        && peer.capabilities.media_match == recipient.capabilities.media_match
                })
                .count();
            projected_fanout_bytes = projected_fanout_bytes.saturating_add(
                encode_message_line(&message)?
                    .len()
                    .saturating_add(2)
                    .saturating_mul(view_recipients),
            );
            // A transition may publish old/new rosters plus state. Reserve
            // four such views before any per-recipient clones are built.
            if projected_fanout_bytes > self.resource_limits.queued_bytes_total / 4 {
                return Err(frame_capacity_error());
            }
        }
        Ok(())
    }
}
