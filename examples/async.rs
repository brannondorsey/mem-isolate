// Compile error if tracing is not enabled
#[cfg(not(feature = "async"))]
compile_error!(
    "This example requires the 'async' feature to be enabled. Run with: cargo run --example async --features async"
);

use mem_isolate::MemIsolateError;
use tracing::{Level, info, instrument};
use tracing_subscriber::EnvFilter;

#[tokio::main]
#[instrument]
async fn main() -> Result<(), MemIsolateError> {
    init_tracing_subscriber();

    println!("Starting async example");
    mem_isolate::execute_in_isolated_process_async(async || {
        // TODO: Come back here and figure out what's hanging
        println!("Inside async callable");
        42
    })
    .await?;
    println!("Finished async example");
    Ok(())
}

/// Defaults to TRACE but can be overridden by the RUST_LOG environment variable
fn init_tracing_subscriber() {
    let env_filter = EnvFilter::builder()
        .with_default_directive(Level::TRACE.into())
        .from_env_lossy();
    tracing_subscriber::fmt().with_env_filter(env_filter).init();
}
