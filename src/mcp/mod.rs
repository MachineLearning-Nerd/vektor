mod handlers;
mod schemas;
mod server;

pub async fn start_stdio_server(config: crate::config::Config) -> crate::error::Result<()> {
    server::start_stdio_server(config).await
}
