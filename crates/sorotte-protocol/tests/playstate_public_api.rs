use std::collections::BTreeMap;

use sorotte_protocol::PlaystatePayload;

#[test]
fn downstream_legacy_playstate_struct_literals_remain_source_compatible() {
    let legacy = PlaystatePayload {
        position: Some(7.0),
        paused: Some(true),
        do_seek: Some(false),
        set_by: Some("alice".to_owned()),
        extra: BTreeMap::new(),
    };

    assert_eq!(legacy.position, Some(7.0));
    assert_eq!(legacy.transport_revision().unwrap(), None);
}
