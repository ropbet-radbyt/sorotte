use super::*;

use serde_json::{Value, json};
use sorotte_player_api::{PlayerAttachmentEpoch, PlayerCommandId};

const RECOGNIZED_SENSITIVE_KEYS: &[&str] = &[
    "password",
    "PASSWORD",
    "roomPassword",
    "room-password",
    "secret",
    "clientSecret",
    "accessToken",
    "access-token",
    "refresh_token",
    "apiKey",
    "api-key",
    "authorization",
    "proxyAuthorization",
    "cookie",
    "cookies",
    "token",
    "httpHeaderFields",
    "headers",
];

fn generated_canary(case: usize, depth: usize, escaped: bool) -> String {
    format!(
        "SOROTTE_PRIVACY_CANARY_{case:03}_{depth:02}_{}Aa9",
        if escaped { "ESC" } else { "RAW" }
    )
}

fn unicode_escape(value: &str) -> String {
    value
        .chars()
        .map(|character| format!("\\u{:04x}", u32::from(character)))
        .collect()
}

fn percent_encode(value: &str, uppercase: bool) -> String {
    value
        .bytes()
        .map(|byte| {
            if uppercase {
                format!("%{byte:02X}")
            } else {
                format!("%{byte:02x}")
            }
        })
        .collect()
}

fn hex_encode(value: &str) -> String {
    value.bytes().map(|byte| format!("{byte:02x}")).collect()
}

fn forbidden_encodings(canary: &str) -> Vec<String> {
    vec![
        canary.to_owned(),
        unicode_escape(canary),
        unicode_escape(canary).to_ascii_uppercase(),
        percent_encode(canary, true),
        percent_encode(canary, false),
        hex_encode(canary),
        hex_encode(canary).to_ascii_uppercase(),
    ]
}

fn assert_no_canary(label: &str, output: &str, canary: &str) {
    for forbidden in forbidden_encodings(canary) {
        assert!(
            !output.contains(&forbidden),
            "{label} retained credential canary encoding {forbidden:?}: {output}"
        );
    }
}

fn credential_leaf_from_wire(key: &str, canary: &str, escaped: bool) -> Value {
    let key = if escaped {
        format!("\"{}\"", unicode_escape(key))
    } else {
        serde_json::to_string(key).expect("test key must serialize")
    };
    let value = if escaped {
        format!("\"{}\"", unicode_escape(canary))
    } else {
        serde_json::to_string(canary).expect("test canary must serialize")
    };
    serde_json::from_str(&format!("{{{key}:{value}}}"))
        .expect("generated credential leaf must be valid JSON")
}

fn nested(mut value: Value, depth: usize) -> Value {
    for level in 0..depth {
        value = match level % 3 {
            0 => json!({ format!("layer_{level}"): value }),
            1 => json!([level, value, {"safe": true}]),
            _ => json!({"items": [{"ordinal": level}, value]}),
        };
    }
    value
}

fn assert_all_transcript_outputs_are_sanitized(raw_json: Value, canary: &str, sequence: u64) {
    let record = MpvTranscriptRecord::sanitized(
        PlayerAttachmentEpoch::new(1),
        sequence,
        sequence.saturating_mul(10),
        Some(PlayerCommandId::new(sequence)),
        Some(i64::try_from(sequence).expect("small generated sequence")),
        raw_json,
    );
    assert_eq!(
        sanitize_json(record.raw_json.clone()),
        record.raw_json,
        "transcript sanitization must be idempotent"
    );

    let transcript =
        MpvTranscript::new(vec![record.clone()]).expect("generated record must be valid");
    let exported = transcript
        .to_json_lines()
        .expect("transcript must serialize");
    let restored =
        MpvTranscript::from_json_lines(&exported).expect("sanitized transcript must parse");
    let reexported = restored
        .to_json_lines()
        .expect("restored transcript must serialize");
    assert_eq!(
        reexported, exported,
        "sanitized transcript JSON-lines must be a stable fixed point"
    );

    let outputs = [
        ("retained raw JSON", record.raw_json.to_string()),
        ("record Debug", format!("{record:?}")),
        ("transcript Debug", format!("{transcript:?}")),
        ("JSON-lines export", exported),
        ("round-trip JSON-lines export", reexported),
        ("redacted diagnostic dump", transcript.redacted_debug_dump()),
        ("sanitized JSON digest", record.raw_json_sha256()),
    ];
    for (label, output) in outputs {
        assert_no_canary(label, &output, canary);
    }
}

#[test]
fn generated_nested_and_escaped_credentials_never_reach_transcript_outputs() {
    let mut sequence = 1_u64;
    for (case, key) in RECOGNIZED_SENSITIVE_KEYS.iter().enumerate() {
        for depth in 0..=6 {
            for escaped in [false, true] {
                let canary = generated_canary(case, depth, escaped);
                let raw_json = json!({
                    "event": "generated-privacy-event",
                    "payload": nested(
                        credential_leaf_from_wire(key, &canary, escaped),
                        depth,
                    ),
                });
                assert_all_transcript_outputs_are_sanitized(raw_json, &canary, sequence);
                sequence += 1;
            }
        }
    }
}

#[test]
fn generated_opaque_payloads_remove_raw_and_encoded_canaries() {
    for depth in 0..=8 {
        let canary = generated_canary(900 + depth, depth, true);
        let embedded = nested(
            json!({
                "note": canary,
                "url": format!(
                    "https://viewer:{canary}@private.invalid/media/{canary}?access_token={}#{canary}",
                    percent_encode(&canary, true),
                ),
                "header": format!("Authorization: Bearer {canary}"),
            }),
            depth,
        )
        .to_string();
        let raw_json = json!({
            "event": MPV_EVENT_CLIENT_MESSAGE,
            "args": [
                format!("third-party-{canary}"),
                embedded,
                format!("Cookie: session={canary}"),
                format!(r"C:\Users\{canary}\private.mkv"),
            ],
        });
        assert_all_transcript_outputs_are_sanitized(
            raw_json,
            &canary,
            u64::try_from(depth + 1).expect("small generated depth"),
        );
    }
}

#[test]
fn escaping_and_round_trips_are_metamorphic_sanitization_fixed_points() {
    for (case, key) in RECOGNIZED_SENSITIVE_KEYS.iter().enumerate() {
        let canary = generated_canary(case, 3, false);
        let direct = nested(credential_leaf_from_wire(key, &canary, false), 3);
        let escaped = nested(credential_leaf_from_wire(key, &canary, true), 3);
        assert_eq!(
            direct, escaped,
            "JSON escaping must decode to the same credential-bearing value"
        );

        let sanitized = sanitize_json(json!({
            "event": "generated-metamorphic-event",
            "payload": direct,
        }));
        let escaped_sanitized = sanitize_json(json!({
            "event": "generated-metamorphic-event",
            "payload": escaped,
        }));
        assert_eq!(escaped_sanitized, sanitized);
        assert_eq!(sanitize_json(sanitized.clone()), sanitized);
        assert_no_canary(
            "metamorphic sanitized JSON",
            &sanitized.to_string(),
            &canary,
        );
    }
}

#[test]
fn generated_transcript_parse_diagnostics_never_echo_canaries() {
    for case in 0..32 {
        let canary = generated_canary(1_000 + case, case % 7, case % 2 == 0);
        let escaped = unicode_escape(&canary);
        let malformed = format!(
            "{{\"attachment_epoch\":1,\"ingress_sequence\":1,\
             \"monotonic_receipt_tick\":1,\"command_id\":null,\
             \"playlist_entry_id\":null,\"raw_json\":\
             {{\"password\":\"{escaped}\"}} trailing"
        );
        let error = MpvTranscript::from_json_lines(&malformed)
            .expect_err("malformed generated transcript must fail");
        assert_no_canary("parse error Display", &error.to_string(), &canary);
        assert_no_canary("parse error Debug", &format!("{error:?}"), &canary);

        let non_object = json!({
            "attachment_epoch": 1,
            "ingress_sequence": 1,
            "monotonic_receipt_tick": 1,
            "command_id": null,
            "playlist_entry_id": null,
            "raw_json": canary,
        })
        .to_string();
        let error = MpvTranscript::from_json_lines(&non_object)
            .expect_err("non-object raw transcript JSON must fail");
        assert_no_canary("validation error Display", &error.to_string(), &canary);
        assert_no_canary("validation error Debug", &format!("{error:?}"), &canary);
    }
}

#[test]
#[should_panic(expected = "structured credential aliases leaked from sanitized transcript")]
fn known_defect_tc_sec_001_structured_credential_aliases_leak_from_sanitized_transcript() {
    let aliases = [
        "credentials",
        "futureCredential",
        "set-cookie",
        "x-api-key",
        "httpHeaders",
    ];
    let mut leaked_aliases = Vec::new();
    for (case, alias) in aliases.into_iter().enumerate() {
        let canary = generated_canary(2_000 + case, 4, true);
        let record = MpvTranscriptRecord::sanitized(
            PlayerAttachmentEpoch::new(1),
            u64::try_from(case + 1).expect("small generated case"),
            u64::try_from(case + 1).expect("small generated case"),
            None,
            None,
            json!({
                "event": "generated-privacy-defect",
                "extension": nested(
                    credential_leaf_from_wire(alias, &canary, true),
                    4,
                ),
            }),
        );
        let transcript =
            MpvTranscript::new(vec![record]).expect("generated defect record must be valid");
        let exported = transcript
            .to_json_lines()
            .expect("generated defect transcript must serialize");
        if forbidden_encodings(&canary)
            .iter()
            .any(|forbidden| exported.contains(forbidden))
        {
            leaked_aliases.push(alias);
        }
    }

    assert!(
        leaked_aliases.is_empty(),
        "structured credential aliases leaked from sanitized transcript: {leaked_aliases:?}"
    );
}
