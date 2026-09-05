use super::*;

#[test]
fn opt_in_capture_records_decoded_event_pump_input() {
    let mut adapter = MpvAdapter::default();
    adapter.enable_lifecycle_transcript_capture();

    adapter.handle_ipc_event(&json!({
        "event": "client-message",
        "playlist_entry_id": 19,
        "args": ["synthetic", "private-value"],
    }));

    let transcript = adapter
        .take_lifecycle_transcript()
        .expect("enabled capture should return a transcript");
    assert_eq!(transcript.len(), 1);
    assert_eq!(transcript.records()[0].ingress_sequence, 1);
    assert_eq!(transcript.records()[0].command_id, None);
    assert_eq!(transcript.records()[0].playlist_entry_id, Some(19));
    assert!(
        !transcript
            .to_json_lines()
            .expect("transcript JSON")
            .contains("private-value")
    );
    assert!(adapter.take_lifecycle_transcript().is_none());
}
