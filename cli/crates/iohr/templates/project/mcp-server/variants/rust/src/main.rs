//! Runs the server over stdio, the way an MCP host starts it. Nothing is printed to stdout but
//! protocol messages, and there is no port and no credential.
use rmcp::{ServiceExt, transport::stdio};

mod server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let running = server::Server::new().serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
