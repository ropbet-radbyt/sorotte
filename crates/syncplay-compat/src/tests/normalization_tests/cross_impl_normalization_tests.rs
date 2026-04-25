use super::*;

#[test]
fn normalize_cross_impl_message_treats_null_fields_as_absent() {
    let with_null = json!({
        "Set": {
            "playlistChange": {
                "files": [],
                "user": null
            }
        }
    });
    let without_null = json!({
        "Set": {
            "playlistChange": {
                "files": []
            }
        }
    });

    assert_eq!(
        normalize_cross_impl_message(with_null),
        normalize_cross_impl_message(without_null)
    );
}

#[test]
fn normalize_cross_impl_message_normalizes_state_ping_timing_fields() {
    let first = json!({
        "State": {
            "playstate": {
                "position": 12.500001,
                "paused": false,
                "doSeek": false,
                "setBy": "alice"
            },
            "ping": {
                "latencyCalculation": 100.1,
                "serverRtt": 0.0
            },
            "ignoringOnTheFly": {
                "server": 1
            }
        }
    });
    let second = json!({
        "State": {
            "playstate": {
                "position": 12.500499,
                "paused": false,
                "doSeek": false,
                "setBy": "alice"
            },
            "ping": {
                "latencyCalculation": 999.9,
                "serverRtt": 0.25
            },
            "ignoringOnTheFly": {
                "server": 1
            }
        }
    });

    assert_eq!(
        normalize_cross_impl_message(first),
        normalize_cross_impl_message(second)
    );
}

#[test]
fn normalize_cross_impl_message_normalizes_set_user_features_by_default() {
    let first = json!({
        "Set": {
            "user": {
                "bob": {
                    "event": {
                        "joined": true,
                        "features": {
                            "chat": false
                        }
                    }
                }
            }
        }
    });
    let second = json!({
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

    assert_eq!(
        normalize_cross_impl_message(first),
        normalize_cross_impl_message(second)
    );
}

#[test]
fn normalize_cross_impl_message_can_preserve_set_user_features() {
    let first = json!({
        "Set": {
            "user": {
                "bob": {
                    "event": {
                        "joined": true,
                        "features": {
                            "chat": false
                        }
                    }
                }
            }
        }
    });
    let second = json!({
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

    let options = MessageNormalizationOptions {
        normalize_set_user_event_features: false,
        normalize_set_user_features: false,
        ..MessageNormalizationOptions::default()
    };
    assert_ne!(
        normalize_cross_impl_message_with_options(first, options),
        normalize_cross_impl_message_with_options(second, options)
    );
}

#[test]
fn normalize_cross_impl_message_canonicalizes_list_position_number_types() {
    let with_integer_position = json!({
        "List": {
            "room1": {
                "alice": {
                    "controller": false,
                    "position": 0,
                    "file": {},
                    "isReady": null
                }
            }
        }
    });
    let with_float_position = json!({
        "List": {
            "room1": {
                "alice": {
                    "controller": false,
                    "position": 0.0,
                    "file": {},
                    "isReady": null
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
    assert_eq!(
        normalize_cross_impl_message_with_options(with_integer_position, options),
        normalize_cross_impl_message_with_options(with_float_position, options)
    );
}

#[test]
fn normalize_cross_impl_message_can_preserve_state_ping_server_rtt() {
    let first = json!({
        "State": {
            "playstate": {
                "position": 12.500001,
                "paused": false,
                "doSeek": false,
                "setBy": "alice"
            },
            "ping": {
                "latencyCalculation": 100.1,
                "serverRtt": 0.0
            },
            "ignoringOnTheFly": {
                "server": 1
            }
        }
    });
    let second = json!({
        "State": {
            "playstate": {
                "position": 12.500499,
                "paused": false,
                "doSeek": false,
                "setBy": "alice"
            },
            "ping": {
                "latencyCalculation": 999.9,
                "serverRtt": 0.25
            },
            "ignoringOnTheFly": {
                "server": 1
            }
        }
    });

    let options = MessageNormalizationOptions {
        normalize_ping_latency_calculation: true,
        normalize_ping_client_latency_calculation: true,
        normalize_ping_client_rtt: true,
        normalize_ping_server_rtt: false,
        ..MessageNormalizationOptions::default()
    };
    assert_ne!(
        normalize_cross_impl_message_with_options(first, options),
        normalize_cross_impl_message_with_options(second, options)
    );
}

#[test]
fn normalize_cross_impl_message_can_preserve_state_ping_client_timing_fields() {
    let first = json!({
        "State": {
            "playstate": {
                "position": 2.0,
                "paused": false,
                "doSeek": true,
                "setBy": "alice"
            },
            "ping": {
                "latencyCalculation": 100.1,
                "clientLatencyCalculation": 124.1,
                "clientRtt": 1.0,
                "serverRtt": 0.0
            }
        }
    });
    let second = json!({
        "State": {
            "playstate": {
                "position": 2.0,
                "paused": false,
                "doSeek": true,
                "setBy": "alice"
            },
            "ping": {
                "latencyCalculation": 100.1,
                "clientLatencyCalculation": 126.1,
                "clientRtt": 2.0,
                "serverRtt": 0.0
            }
        }
    });

    let options = MessageNormalizationOptions {
        normalize_ping_latency_calculation: false,
        normalize_ping_client_latency_calculation: false,
        normalize_ping_client_rtt: false,
        normalize_ping_server_rtt: false,
        ..MessageNormalizationOptions::default()
    };
    assert_ne!(
        normalize_cross_impl_message_with_options(first, options),
        normalize_cross_impl_message_with_options(second, options)
    );
}

#[test]
fn normalize_cross_impl_message_can_preserve_state_ping_latency_calculation() {
    let first = json!({
        "State": {
            "playstate": {
                "position": 2.0,
                "paused": false,
                "doSeek": true,
                "setBy": "alice"
            },
            "ping": {
                "latencyCalculation": 100.1,
                "serverRtt": 0.0
            }
        }
    });
    let second = json!({
        "State": {
            "playstate": {
                "position": 2.0,
                "paused": false,
                "doSeek": true,
                "setBy": "alice"
            },
            "ping": {
                "latencyCalculation": 101.6,
                "serverRtt": 0.0
            }
        }
    });

    let options = MessageNormalizationOptions {
        normalize_ping_latency_calculation: false,
        normalize_ping_client_latency_calculation: false,
        normalize_ping_client_rtt: false,
        normalize_ping_server_rtt: false,
        ..MessageNormalizationOptions::default()
    };
    assert_ne!(
        normalize_cross_impl_message_with_options(first, options),
        normalize_cross_impl_message_with_options(second, options)
    );
}
