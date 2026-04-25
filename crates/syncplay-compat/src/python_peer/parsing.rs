use super::*;

impl LegacyServerPythonPeerHarness {
    pub(super) fn parse_peer_status_line(&self, status_line: &str) -> Result<Value, InteropError> {
        let parsed: Value = serde_json::from_str(status_line).map_err(|error| {
            InteropError::InvalidPythonBatchResponse(format!(
                "python live peer status line was not valid JSON ({status_line:?}): {error}"
            ))
        })?;
        match parsed.get("status").and_then(Value::as_str) {
            Some("connected")
            | Some("ready-command-sent")
            | Some("local-ready")
            | Some("user-ready")
            | Some("chat-command-sent")
            | Some("chat-message")
            | Some("user-file")
            | Some("playlist-command-sent")
            | Some("playlist-index-command-sent")
            | Some("playlist")
            | Some("playlist-index")
            | Some("local-controller")
            | Some("user-controller")
            | Some("snapshot") => Ok(parsed),
            Some("error") => Err(InteropError::InvalidPythonBatchResponse(
                parsed
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| {
                        format!(
                            "python live peer reported an unspecified error line: {status_line:?}"
                        )
                    }),
            )),
            Some(other) => Err(InteropError::InvalidPythonBatchResponse(format!(
                "python live peer reported unexpected status {other:?}: {status_line:?}"
            ))),
            None => Err(InteropError::InvalidPythonBatchResponse(format!(
                "python live peer status line did not include a status field: {status_line:?}"
            ))),
        }
    }

    pub(super) fn parse_peer_snapshot(
        status: &Value,
    ) -> Result<LegacyPythonPeerSnapshot, InteropError> {
        let username = status
            .get("username")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                InteropError::InvalidPythonBatchResponse(format!(
                    "python live peer status did not include a username string: {status}"
                ))
            })?;
        let room = status
            .get("room")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                InteropError::InvalidPythonBatchResponse(format!(
                    "python live peer status did not include a room string: {status}"
                ))
            })?;
        let local_ready = status.get("localReady").and_then(Value::as_bool);
        let local_file_name = match status.get("fileName") {
            Some(Value::Null) | None => None,
            Some(value) => Some(value.as_str().map(str::to_owned).ok_or_else(|| {
                InteropError::InvalidPythonBatchResponse(format!(
                    "python live peer status included a malformed local file name: {value}"
                ))
            })?),
        };
        let local_controller = status.get("localController").and_then(Value::as_bool);
        let observed_users = status
            .get("users")
            .and_then(Value::as_object)
            .map(|users| {
                users
                    .iter()
                    .map(|(username, ready)| (username.clone(), ready.as_bool()))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let observed_user_file_names = status
            .get("userFiles")
            .and_then(Value::as_object)
            .map(|users| {
                users
                    .iter()
                    .map(|(username, file_name)| {
                        let file_name = match file_name {
                            Value::Null => Ok(None),
                            Value::String(file_name) => Ok(Some(file_name.clone())),
                            other => Err(InteropError::InvalidPythonBatchResponse(format!(
                                "python live peer status included a malformed user file name for {username}: {other}"
                            ))),
                        }?;
                        Ok((username.clone(), file_name))
                    })
                    .collect::<Result<BTreeMap<_, _>, InteropError>>()
            })
            .transpose()?
            .unwrap_or_default();
        let observed_user_controllers = status
            .get("controllers")
            .and_then(Value::as_object)
            .map(|users| {
                users
                    .iter()
                    .map(|(username, controller)| (username.clone(), controller.as_bool()))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let playlist = status
            .get("playlist")
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .map(|file| {
                        file.as_str().map(str::to_owned).ok_or_else(|| {
                            InteropError::InvalidPythonBatchResponse(format!(
                                "python live peer status included a malformed playlist entry: {file}"
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>, InteropError>>()
            })
            .transpose()?
            .unwrap_or_default();
        let playlist_index = match status.get("playlistIndex") {
            Some(Value::Null) | None => None,
            Some(value) => {
                let index = value.as_u64().ok_or_else(|| {
                    InteropError::InvalidPythonBatchResponse(format!(
                        "python live peer status included a malformed playlist index: {value}"
                    ))
                })?;
                Some(usize::try_from(index).map_err(|_| {
                    InteropError::InvalidPythonBatchResponse(format!(
                        "python live peer playlist index exceeded usize range: {index}"
                    ))
                })?)
            }
        };
        let chat_messages = status
            .get("chatMessages")
            .and_then(Value::as_array)
            .map(|messages| {
                messages
                    .iter()
                    .map(|message| {
                        let sender = message
                            .get("sender")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .ok_or_else(|| {
                                InteropError::InvalidPythonBatchResponse(format!(
                                    "python live peer status included a malformed chat sender: {message}"
                                ))
                            })?;
                        let message_text = message
                            .get("message")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .ok_or_else(|| {
                                InteropError::InvalidPythonBatchResponse(format!(
                                    "python live peer status included a malformed chat message: {message}"
                                ))
                            })?;
                        Ok(LegacyPythonPeerChatMessage {
                            sender,
                            message: message_text,
                        })
                    })
                    .collect::<Result<Vec<_>, InteropError>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok(LegacyPythonPeerSnapshot {
            username,
            room,
            local_ready,
            local_file_name,
            local_controller,
            observed_users,
            observed_user_file_names,
            observed_user_controllers,
            playlist,
            playlist_index,
            chat_messages,
        })
    }
}
