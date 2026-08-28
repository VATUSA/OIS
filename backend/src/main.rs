//! OIS backend — Axum API for the VATUSA platform.

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    // Pretty, backtrace-rich reports for both panics and the error returned from `run()`.
    color_eyre::install()?;
    ois_backend::run().await?;
    Ok(())
}
