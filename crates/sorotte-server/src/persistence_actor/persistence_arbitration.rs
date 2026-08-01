use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use super::ServerPersistenceEffect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RoomEffectEnqueueDisposition {
    Accepted,
    IgnoredStale,
    NotRoomEffect,
}

#[derive(Debug, Default)]
struct RoomPersistenceDesiredState {
    highest_seen_version: u64,
    desired_effect: Option<ServerPersistenceEffect>,
    unresolved_failure_version: Option<u64>,
}

#[derive(Debug, Default)]
pub(super) struct RoomPersistenceArbitration {
    states: BTreeMap<String, RoomPersistenceDesiredState>,
}

pub(super) type DesiredRoomEffects = Arc<Mutex<RoomPersistenceArbitration>>;

pub(super) fn room_effect_key_and_version(effect: &ServerPersistenceEffect) -> Option<(&str, u64)> {
    match effect {
        ServerPersistenceEffect::SaveRoom {
            room_name, version, ..
        }
        | ServerPersistenceEffect::DeleteRoom { room_name, version } => {
            Some((room_name.as_str(), *version))
        }
        ServerPersistenceEffect::RecordStatsSnapshot { .. } => None,
    }
}

impl RoomPersistenceArbitration {
    pub(super) fn enqueue(
        &mut self,
        effect: ServerPersistenceEffect,
    ) -> RoomEffectEnqueueDisposition {
        let Some((room_name, version)) = room_effect_key_and_version(&effect) else {
            return RoomEffectEnqueueDisposition::NotRoomEffect;
        };
        let state = self.states.entry(room_name.to_owned()).or_default();
        if version <= state.highest_seen_version {
            return RoomEffectEnqueueDisposition::IgnoredStale;
        }
        state.highest_seen_version = version;
        state.desired_effect = Some(effect);
        state.unresolved_failure_version = None;
        RoomEffectEnqueueDisposition::Accepted
    }

    pub(super) fn desired_effects(&self) -> Vec<ServerPersistenceEffect> {
        self.states
            .values()
            .filter_map(|state| state.desired_effect.clone())
            .collect()
    }

    pub(super) fn is_effect_current(&self, effect: &ServerPersistenceEffect) -> bool {
        let Some((room_name, version)) = room_effect_key_and_version(effect) else {
            return false;
        };
        self.states.get(room_name).is_some_and(|state| {
            state.highest_seen_version == version
                && state
                    .desired_effect
                    .as_ref()
                    .and_then(room_effect_key_and_version)
                    .is_some_and(|(_, desired_version)| desired_version == version)
        })
    }

    pub(super) fn is_version_current(&self, room_name: &str, version: u64) -> bool {
        self.states
            .get(room_name)
            .is_some_and(|state| state.highest_seen_version == version)
    }

    pub(super) fn mark_applied(&mut self, room_name: &str, version: u64, is_delete: bool) {
        if !self.is_version_current(room_name, version) {
            return;
        }
        if is_delete {
            self.states.remove(room_name);
        } else if let Some(state) = self.states.get_mut(room_name) {
            state.desired_effect = None;
            state.unresolved_failure_version = None;
        }
    }

    pub(super) fn mark_failed(&mut self, room_name: &str, version: u64) {
        if !self.is_version_current(room_name, version) {
            return;
        }
        if let Some(state) = self.states.get_mut(room_name) {
            state.unresolved_failure_version = Some(version);
        }
    }

    pub(super) fn is_settled(&self) -> bool {
        self.states.values().all(|state| {
            state.desired_effect.is_none() && state.unresolved_failure_version.is_none()
        })
    }

    pub(super) fn should_report_recovery(&self, applied_any: bool) -> bool {
        applied_any && self.is_settled()
    }
}
