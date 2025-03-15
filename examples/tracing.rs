// Compile error if tracing is not enabled
#[cfg(feature = "tracing")]
use tracing::{info, instrument};
#[cfg(not(feature = "tracing"))]
compile_error!(
    "This example requires the 'tracing' feature to be enabled. Run with: cargo run --example tracing --features tracing"
);

use mem_isolate::MemIsolateError;
#[cfg_attr(feature = "tracing", instrument)]
fn main() -> Result<(), MemIsolateError> {
    #[cfg(feature = "tracing")]
    {
        // TODO: Make sure RUST_LOG is overriding the default
        tracing_subscriber::fmt().with_env_filter("trace").init();
        mem_isolate::execute_in_isolated_process(|| {
            info!("look ma, I'm in the callable!");
            42
        })?;
    }

    Ok(())
}
