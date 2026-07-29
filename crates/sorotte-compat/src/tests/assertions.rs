use super::*;

#[derive(Clone, Debug, PartialEq)]
struct ComparableOutbound {
    client_id: String,
    message: Value,
}

fn comparable_outbounds_match(
    python_outputs: &[ComparableOutbound],
    python_index: usize,
    rust_outputs: &[ComparableOutbound],
    rust_index: usize,
) -> bool {
    let Some(python_output) = python_outputs.get(python_index) else {
        return false;
    };
    let Some(rust_output) = rust_outputs.get(rust_index) else {
        return false;
    };
    python_output.client_id == rust_output.client_id && python_output.message == rust_output.message
}

mod legacy_client_assertions;
mod legacy_fanout_assertions;
mod legacy_process_assertions;
mod legacy_tls_assertions;
mod python_fanout_assertions;
mod tls_io_assertions;
mod trace_assertions;

pub(super) use legacy_client_assertions::*;
pub(super) use legacy_fanout_assertions::*;
pub(super) use legacy_process_assertions::*;
pub(super) use legacy_tls_assertions::*;
pub(super) use python_fanout_assertions::*;
pub(super) use tls_io_assertions::*;
pub(super) use trace_assertions::*;
