use sorotte_protocol::{SOROTTE_PLAYBACK_BARRIER_V1, SOROTTE_READINESS_V2, SetPayload};

pub(super) fn ordered_set_commands(set: SetPayload) -> Vec<(String, SetPayload)> {
    let mut order = set.command_order.clone();
    for command in [
        "room",
        "file",
        "user",
        "controllerAuth",
        "newControlledRoom",
        "ready",
        "playlistChange",
        "playlistIndex",
        "features",
        SOROTTE_PLAYBACK_BARRIER_V1,
        SOROTTE_READINESS_V2,
    ] {
        if !order.iter().any(|candidate| candidate == command) {
            order.push(command.to_owned());
        }
    }
    order
        .into_iter()
        .map(|command| (command, set.clone()))
        .collect()
}
