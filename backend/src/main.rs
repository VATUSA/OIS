//! OIS backend — Axum API for the VATUSA platform.

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ois_backend::run().await
}
