use super::*;

#[test]
fn create_client_session_applies_desync_overrides_from_config() {
    let config = ClientLoopConfig {
        rewind_on_desync_override: Some(false),
        fastforward_on_desync_override: Some(false),
        slow_on_desync_override: Some(false),
        rewind_threshold_seconds_override: Some(1.25),
        fastforward_threshold_seconds_override: Some(3.5),
        slowdown_threshold_seconds_override: Some(2.25),
        ..test_client_loop_config()
    };

    let session = create_client_session(&config);
    let desync = session.desync_config();
    assert!(!desync.rewind_on_desync);
    assert!(!desync.fastforward_on_desync);
    assert!(!desync.slow_on_desync);
    assert_eq!(desync.rewind_threshold_seconds, 1.25);
    assert_eq!(desync.fastforward_threshold_seconds, 3.5);
    assert_eq!(desync.slowdown_threshold_seconds, 2.25);
}
