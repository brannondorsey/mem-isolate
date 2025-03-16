// Compile error if tracing is not enabled
#[cfg(not(feature = "tracing"))]
compile_error!(
    "This example requires the 'tracing' feature to be enabled. Run with: cargo run --example tracing --features tracing"
);

use mem_isolate::MemIsolateError;
use tracing::{Level, info, instrument};
use tracing_subscriber::EnvFilter;

#[instrument]
fn main() -> Result<(), MemIsolateError> {
    init_tracing_subscriber();

    mem_isolate::execute_in_isolated_process(|| {
        info!("look ma, I'm in the callable!");
        42
    })?;

    Ok(())
}

/// Defaults to TRACE but can be overridden by the RUST_LOG environment variable
fn init_tracing_subscriber() {
    let env_filter = EnvFilter::builder()
        .with_default_directive(Level::TRACE.into())
        .from_env_lossy();
    tracing_subscriber::fmt().with_env_filter(env_filter).init();
}
