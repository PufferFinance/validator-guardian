// #[macro_use]
extern crate anyhow;
extern crate env_logger;

// Antithesis coverage instrumentation. The `antithesis-instrumentation` crate
// links the Antithesis coverage runtime; it must be referenced at least once so
// Cargo does not drop it as unused during linking. Gated behind the
// `antithesis_instr` feature and paired with sancov rustflags (antithesis/Dockerfile).
#[cfg(feature = "antithesis_instr")]
use antithesis_instrumentation as _;

pub mod constants;
pub mod crypto;
pub mod enclave;
// TODO: Check lighthouse if we can replace
pub mod client;
pub mod eth2;
pub mod io;

#[macro_export]
macro_rules! strip_0x_prefix {
    ($hex:expr) => {
        $hex.strip_prefix("0x").unwrap_or(&$hex).into()
    };
}
