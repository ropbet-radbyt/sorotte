use super::*;

#[test]
fn canonicalize_legacy_hello_fields_aligns_shared_capabilities() {
    let mut legacy_message = json!({
        "Hello": {
            "features": {
                "chat": true,
                "isolateRooms": false,
                "managedRooms": true,
                "maxChatMessageLength": 150,
                "maxFilenameLength": 250,
                "maxRoomNameLength": 35,
                "maxUsernameLength": 16,
                "persistentRooms": false,
                "readiness": true,
                "setOthersReadiness": true
            }
        }
    });
    let mut runtime_message = json!({
        "Hello": {
            "features": {
                "chat": true,
                "featureList": true,
                "isolateRooms": false,
                "managedRooms": true,
                "persistentRooms": false,
                "readiness": true,
                "setOthersReadiness": true,
                "uiMode": "UNKNOWN"
            }
        }
    });
    canonicalize_legacy_hello_fields(&mut legacy_message);
    canonicalize_legacy_hello_fields(&mut runtime_message);
    assert_eq!(legacy_message, runtime_message);
}

#[test]
fn canonicalize_legacy_hello_fields_aligns_empty_motd_shapes() {
    let mut legacy_message = json!({
        "Hello": {
            "motd": null
        }
    });
    let mut runtime_message = json!({
        "Hello": {
            "motd": ""
        }
    });
    canonicalize_legacy_hello_fields(&mut legacy_message);
    canonicalize_legacy_hello_fields(&mut runtime_message);
    assert_eq!(legacy_message, runtime_message);
}

#[test]
fn canonicalize_legacy_hello_fields_preserves_non_default_motd() {
    let mut first_message = json!({
        "Hello": {
            "motd": "Welcome to a custom room."
        }
    });
    let mut second_message = json!({
        "Hello": {
            "motd": "Different custom room text."
        }
    });
    canonicalize_legacy_hello_fields(&mut first_message);
    canonicalize_legacy_hello_fields(&mut second_message);
    assert_ne!(first_message, second_message);
}
