use super::*;

#[test]
fn legacy_timing_canonicalizer_aligns_latency_from_independent_clock_origins() {
    let mut canonicalizer = LegacyTimingCanonicalizer::default();
    let mut legacy_message = json!({
        "State": {
            "ping": {
                "latencyCalculation": 1770633211.852
            }
        }
    });
    let mut runtime_message = json!({
        "State": {
            "ping": {
                "latencyCalculation": 0.0
            }
        }
    });
    canonicalizer.canonicalize_message(&mut legacy_message, LegacyTimingSide::Legacy);
    canonicalizer.canonicalize_message(&mut runtime_message, LegacyTimingSide::Runtime);
    assert_eq!(legacy_message, runtime_message);

    let mut legacy_next = json!({
        "State": {
            "ping": {
                "latencyCalculation": 1770633212.852
            }
        }
    });
    let mut runtime_next = json!({
        "State": {
            "ping": {
                "latencyCalculation": 1.0
            }
        }
    });
    canonicalizer.canonicalize_message(&mut legacy_next, LegacyTimingSide::Legacy);
    canonicalizer.canonicalize_message(&mut runtime_next, LegacyTimingSide::Runtime);
    assert_eq!(legacy_next, runtime_next);
}

#[test]
fn legacy_timing_canonicalizer_aligns_server_rtt_from_independent_nonzero_origins() {
    let mut canonicalizer = LegacyTimingCanonicalizer::default();
    let mut legacy_message = json!({
        "State": {
            "ping": {
                "latencyCalculation": 1770633211.852,
                "serverRtt": 1770632677.433682
            }
        }
    });
    let mut runtime_message = json!({
        "State": {
            "ping": {
                "latencyCalculation": 0.0,
                "serverRtt": 0.0
            }
        }
    });
    canonicalizer.canonicalize_message(&mut legacy_message, LegacyTimingSide::Legacy);
    canonicalizer.canonicalize_message(&mut runtime_message, LegacyTimingSide::Runtime);
    assert_eq!(
        legacy_message
            .pointer("/State/ping/serverRtt")
            .and_then(Value::as_f64),
        Some(0.0)
    );
    assert_eq!(
        runtime_message
            .pointer("/State/ping/serverRtt")
            .and_then(Value::as_f64),
        Some(0.0)
    );
}
