use super::*;

#[test]
fn canonicalize_legacy_set_user_features_aligns_missing_and_default_shapes() {
    let mut legacy_message = json!({
        "Set": {
            "user": {
                "bob": {
                    "event": {
                        "joined": true,
                        "features": {
                            "chat": false,
                            "featureList": false,
                            "managedRooms": false,
                            "persistentRooms": false,
                            "readiness": false,
                            "sharedPlaylists": false,
                            "uiMode": "Unknown"
                        }
                    }
                }
            }
        }
    });
    let mut runtime_message = json!({
        "Set": {
            "user": {
                "bob": {
                    "event": {
                        "joined": true
                    }
                }
            }
        }
    });
    canonicalize_legacy_set_user_features(&mut legacy_message);
    canonicalize_legacy_set_user_features(&mut runtime_message);
    assert_eq!(legacy_message, runtime_message);
}

#[test]
fn canonicalize_legacy_set_user_features_aligns_compat_missing_features_marker() {
    let mut legacy_message = json!({
        "Set": {
            "user": {
                "charlie": {
                    "event": {
                        "joined": true,
                        "features": {}
                    },
                    "features": {}
                }
            }
        }
    });
    let mut runtime_message = json!({
        "Set": {
            "user": {
                "charlie": {
                    "event": {
                        "joined": true
                    }
                }
            }
        }
    });
    legacy_message
        .pointer_mut("/Set/user/charlie/event/features")
        .and_then(Value::as_object_mut)
        .expect("event features object should exist")
        .insert(
            LEGACY_COMPAT_MISSING_FEATURES_MARKER.to_owned(),
            Value::Bool(true),
        );
    legacy_message
        .pointer_mut("/Set/user/charlie/features")
        .and_then(Value::as_object_mut)
        .expect("user features object should exist")
        .insert(
            LEGACY_COMPAT_MISSING_FEATURES_MARKER.to_owned(),
            Value::Bool(true),
        );
    canonicalize_legacy_set_user_features(&mut legacy_message);
    canonicalize_legacy_set_user_features(&mut runtime_message);
    assert_eq!(legacy_message, runtime_message);
}

#[test]
fn canonicalize_legacy_list_fields_aligns_feature_and_not_ready_shapes() {
    let mut legacy_message = json!({
        "List": {
            "room1": {
                "bob": {
                    "controller": false,
                    "features": {
                        "chat": false,
                        "featureList": false,
                        "managedRooms": false,
                        "persistentRooms": false,
                        "readiness": false,
                        "sharedPlaylists": false,
                        "uiMode": "Unknown"
                    },
                    "isReady": null,
                    "file": {},
                    "position": 0
                }
            }
        }
    });
    let mut runtime_message = json!({
        "List": {
            "room1": {
                "bob": {
                    "controller": false,
                    "isReady": false,
                    "file": {},
                    "position": 0.0
                }
            }
        }
    });
    let options = MessageNormalizationOptions {
        normalize_list_features: false,
        normalize_list_position: false,
        normalize_list_file: false,
        normalize_list_is_ready_when_false_or_null: false,
        ..MessageNormalizationOptions::default()
    };
    legacy_message = normalize_cross_impl_message_with_options(legacy_message, options);
    runtime_message = normalize_cross_impl_message_with_options(runtime_message, options);
    canonicalize_legacy_list_fields(&mut legacy_message);
    canonicalize_legacy_list_fields(&mut runtime_message);
    assert_eq!(legacy_message, runtime_message);
}

#[test]
fn canonicalize_legacy_list_fields_aligns_compat_missing_features_marker() {
    let mut legacy_message = json!({
        "List": {
            "room1": {
                "charlie": {
                    "controller": false,
                    "features": {},
                    "file": {},
                    "position": 0.0
                }
            }
        }
    });
    let mut runtime_message = json!({
        "List": {
            "room1": {
                "charlie": {
                    "controller": false,
                    "file": {},
                    "position": 0.0
                }
            }
        }
    });
    legacy_message
        .pointer_mut("/List/room1/charlie/features")
        .and_then(Value::as_object_mut)
        .expect("list user features object should exist")
        .insert(
            LEGACY_COMPAT_MISSING_FEATURES_MARKER.to_owned(),
            Value::Bool(true),
        );
    canonicalize_legacy_list_fields(&mut legacy_message);
    canonicalize_legacy_list_fields(&mut runtime_message);
    assert_eq!(legacy_message, runtime_message);
}
