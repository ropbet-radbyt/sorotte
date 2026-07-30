//! Public-boundary robustness properties for protocol lines.
//!
//! This integration suite deliberately treats the protocol crate as a black
//! box. The source-order oracle uses serde's streaming map visitor rather than
//! the codec's handwritten scanner.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use proptest::{
    prelude::*,
    test_runner::{Config as ProptestConfig, RngSeed},
};
use serde::de::{Deserializer as _, IgnoredAny, MapAccess, Visitor};
use sorotte_protocol::{
    DEFAULT_MAX_PROTOCOL_LINE_BYTES, ProtocolMessage, decode_line, decode_message_line,
    decode_message_line_items, decode_message_lines, encode_line, encode_message_line,
};

const DEFAULT_CASES: u32 = 512;
const MAX_CASES: u32 = 100_000;
const PROPERTY_SEED: u64 = 0x5A17_0FFE_C0DE_2026;
const MAX_GENERATED_BYTES: usize = 4_096;

#[derive(Clone, Copy, Debug)]
enum CorpusExpectation {
    Typed,
    Json,
    Invalid,
}

const CORPUS_FILES: [(&str, CorpusExpectation); 14] = [
    (
        "typed-escaped-composite-order.json",
        CorpusExpectation::Typed,
    ),
    ("typed-duplicate-set.json", CorpusExpectation::Typed),
    ("typed-nested-lookalikes.json", CorpusExpectation::Typed),
    ("typed-unicode-chat.json", CorpusExpectation::Typed),
    ("typed-leading-whitespace.json", CorpusExpectation::Typed),
    ("json-unknown-after-valid.json", CorpusExpectation::Json),
    ("json-empty-object.json", CorpusExpectation::Json),
    ("json-array.json", CorpusExpectation::Json),
    ("json-null.json", CorpusExpectation::Json),
    ("json-duplicate-unknown.json", CorpusExpectation::Json),
    ("invalid-truncated-object.json", CorpusExpectation::Invalid),
    ("invalid-string-escape.json", CorpusExpectation::Invalid),
    ("invalid-trailing-token.json", CorpusExpectation::Invalid),
    ("invalid-unclosed-array.json", CorpusExpectation::Invalid),
];

struct OrderedKeysVisitor;

impl<'de> Visitor<'de> for OrderedKeysVisitor {
    type Value = Vec<String>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            map.next_value::<IgnoredAny>()?;
            keys.push(key);
        }
        Ok(keys)
    }
}

fn configured_proptest() -> ProptestConfig {
    let cases = match std::env::var("PROPTEST_CASES") {
        Ok(raw) => raw
            .parse::<u32>()
            .ok()
            .filter(|cases| *cases > 0)
            .unwrap_or_else(|| panic!("PROPTEST_CASES must be an integer from 1 to {MAX_CASES}"))
            .min(MAX_CASES),
        Err(_) => DEFAULT_CASES,
    };
    ProptestConfig {
        cases,
        max_shrink_iters: 20_000,
        rng_seed: RngSeed::Fixed(PROPERTY_SEED),
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

fn corpus_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
        .join("protocol_parser")
}

fn read_corpus() -> Vec<(PathBuf, Vec<u8>)> {
    CORPUS_FILES
        .iter()
        .map(|(name, _)| {
            let path = corpus_directory().join(name);
            let metadata =
                std::fs::symlink_metadata(&path).expect("corpus seed metadata must be readable");
            assert!(
                metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
                "corpus seeds must be direct regular files: {}",
                path.display()
            );
            let bytes = std::fs::read(&path).expect("corpus seed must be readable");
            assert!(
                bytes.len() <= DEFAULT_MAX_PROTOCOL_LINE_BYTES,
                "corpus seed exceeds the production line limit: {}",
                path.display()
            );
            (path, bytes)
        })
        .collect()
}

fn cached_corpus_bytes() -> &'static Vec<Vec<u8>> {
    static CORPUS: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    CORPUS.get_or_init(|| read_corpus().into_iter().map(|(_, bytes)| bytes).collect())
}

fn streaming_source_key_order(line: &str) -> serde_json::Result<Vec<String>> {
    let mut deserializer = serde_json::Deserializer::from_str(line);
    let keys = deserializer.deserialize_map(OrderedKeysVisitor)?;
    deserializer.end()?;

    let mut seen = BTreeSet::new();
    Ok(keys
        .into_iter()
        .filter(|key| seen.insert(key.clone()))
        .collect())
}

fn assert_successful_message_roundtrip(message: &ProtocolMessage) {
    let encoded = encode_message_line(message).expect("decoded messages must re-encode");
    let decoded = decode_message_line(&encoded).expect("re-encoded messages must decode again");
    assert_eq!(
        message, &decoded,
        "successful typed decoding must be semantically stable"
    );
}

fn assert_public_string_invariants(line: &str) {
    let raw = decode_line(line);
    let items = decode_message_line_items(line);
    let messages = decode_message_lines(line);
    let first = decode_message_line(line);

    assert_eq!(
        raw.is_ok(),
        items.is_ok(),
        "line-item framing must accept exactly the syntactically valid JSON inputs"
    );
    let Ok(value) = raw else {
        assert!(messages.is_err());
        assert!(first.is_err());
        return;
    };

    let items = items.expect("valid JSON must produce line-item diagnostics");
    assert!(
        !items.is_empty(),
        "valid JSON must produce at least one diagnostic item"
    );

    let encoded_value = encode_line(&value).expect("decoded JSON values must re-encode");
    assert_eq!(
        decode_line(&encoded_value).expect("re-encoded JSON must decode"),
        value,
        "successful raw decoding must be semantically stable"
    );

    if let Some(object) = value.as_object() {
        let expected_order =
            streaming_source_key_order(line).expect("streaming order oracle must parse valid JSON");
        let actual_order = items
            .iter()
            .filter_map(|item| item.command.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            actual_order, expected_order,
            "line items must preserve unique top-level source order"
        );

        for item in &items {
            if let Some(command) = &item.command {
                assert_eq!(
                    object.get(command),
                    Some(&item.payload),
                    "duplicate commands must retain their surviving JSON value"
                );
            }
        }
    } else {
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].command, None);
        assert_eq!(items[0].payload, value);
    }

    let all_typed = items.iter().all(|item| item.message.is_ok());
    assert_eq!(
        messages.is_ok(),
        all_typed,
        "aggregate decoding must fail if and only if an item fails typed decoding"
    );
    assert_eq!(
        first.is_ok(),
        all_typed,
        "single-message decoding remains aggregate-strict"
    );

    if let Ok(messages) = messages {
        assert_eq!(messages.len(), items.len());
        assert_eq!(
            messages
                .iter()
                .map(ProtocolMessage::kind)
                .collect::<Vec<_>>(),
            items
                .iter()
                .map(|item| {
                    item.message
                        .as_ref()
                        .expect("all item outcomes were checked")
                        .kind()
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(
            first
                .as_ref()
                .expect("all item outcomes were checked")
                .kind(),
            messages
                .first()
                .expect("valid aggregate cannot be empty")
                .kind()
        );
    }

    for item in items {
        if let Ok(message) = item.message {
            assert_successful_message_roundtrip(&message);
        }
    }
}

fn assert_public_byte_boundary(bytes: &[u8]) {
    if let Ok(line) = std::str::from_utf8(bytes) {
        assert_public_string_invariants(line);
    }
}

fn mutate_corpus_seed(seed_index: u8, edits: &[(u8, u16, u8)]) -> Vec<u8> {
    let corpus = cached_corpus_bytes();
    let mut bytes = corpus[usize::from(seed_index) % corpus.len()].clone();

    for (operation, offset_hint, byte) in edits {
        match operation % 4 {
            0 if bytes.len() < MAX_GENERATED_BYTES => {
                let offset = usize::from(*offset_hint) % (bytes.len() + 1);
                bytes.insert(offset, *byte);
            }
            1 if !bytes.is_empty() => {
                let offset = usize::from(*offset_hint) % bytes.len();
                bytes[offset] = *byte;
            }
            2 if !bytes.is_empty() => {
                let offset = usize::from(*offset_hint) % bytes.len();
                bytes.remove(offset);
            }
            3 if !bytes.is_empty() => {
                let new_len = usize::from(*offset_hint) % (bytes.len() + 1);
                bytes.truncate(new_len);
            }
            _ => {}
        }
    }
    bytes
}

#[test]
fn checked_in_protocol_parser_corpus_is_complete_and_replays() {
    let actual_names = {
        let mut names = std::fs::read_dir(corpus_directory())
            .expect("protocol parser corpus directory must exist")
            .map(|entry| {
                entry
                    .expect("corpus directory entry must be readable")
                    .file_name()
                    .into_string()
                    .expect("corpus filenames must be UTF-8")
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    };
    let mut expected_names = CORPUS_FILES
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect::<Vec<_>>();
    expected_names.sort();
    assert_eq!(
        actual_names, expected_names,
        "corpus additions and removals must update the explicit replay manifest"
    );

    for ((path, bytes), (_, expectation)) in read_corpus().into_iter().zip(CORPUS_FILES) {
        let line = std::str::from_utf8(&bytes).expect("checked-in corpus must be UTF-8");
        assert_public_string_invariants(line);
        match expectation {
            CorpusExpectation::Typed => assert!(
                decode_message_line(line).is_ok(),
                "typed corpus seed must decode: {}",
                path.display()
            ),
            CorpusExpectation::Json => assert!(
                decode_line(line).is_ok(),
                "JSON corpus seed must parse: {}",
                path.display()
            ),
            CorpusExpectation::Invalid => assert!(
                decode_line(line).is_err(),
                "invalid corpus seed must remain rejected: {}",
                path.display()
            ),
        }
    }

    println!(
        "protocol-parser-corpus-replay: files={}",
        CORPUS_FILES.len()
    );
}

#[test]
fn json_framing_whitespace_is_semantically_neutral_for_the_corpus() {
    for (path, bytes) in read_corpus() {
        let line = std::str::from_utf8(&bytes).expect("checked-in corpus must be UTF-8");
        let Ok(expected) = decode_line(line) else {
            continue;
        };
        for framed in [
            format!("{line}\n"),
            format!("{line}\r\n"),
            format!(" \t{line}\r\n"),
        ] {
            assert_eq!(
                decode_line(&framed).expect("JSON whitespace framing must remain valid"),
                expected,
                "{}",
                path.display()
            );
            assert_public_string_invariants(&framed);
        }
    }
}

proptest! {
    #![proptest_config(configured_proptest())]

    #[test]
    fn arbitrary_bytes_are_total_at_the_public_utf8_boundary(
        bytes in proptest::collection::vec(any::<u8>(), 0..=MAX_GENERATED_BYTES),
    ) {
        assert_public_byte_boundary(&bytes);
    }

    #[test]
    fn arbitrary_unicode_strings_are_total_across_all_decode_entrypoints(
        characters in proptest::collection::vec(any::<char>(), 0..=512),
    ) {
        let line = characters.into_iter().collect::<String>();
        assert_public_string_invariants(&line);
    }

    #[test]
    fn deterministic_corpus_mutations_preserve_public_decode_invariants(
        seed_index in any::<u8>(),
        edits in proptest::collection::vec((any::<u8>(), any::<u16>(), any::<u8>()), 0..=24),
    ) {
        let bytes = mutate_corpus_seed(seed_index, &edits);
        prop_assert!(bytes.len() <= MAX_GENERATED_BYTES);
        assert_public_byte_boundary(&bytes);
    }
}

#[test]
fn corpus_paths_remain_within_the_test_directory() {
    let corpus = corpus_directory();
    assert!(corpus.starts_with(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")));
    for (path, _) in read_corpus() {
        assert!(path.starts_with(&corpus));
    }
}
