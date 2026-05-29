mod handlers;
mod schemas;
mod server;

pub async fn start_stdio_server() -> crate::error::Result<()> {
    server::start_stdio_server().await
}
