#[tokio::main]
async fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.iter().any(|argument| argument == "--healthcheck") {
        match healthcheck().await {
            Ok(()) => return,
            Err(error) => eprintln!("rust_mcp_healthcheck_failed error={error}"),
        }
        std::process::exit(1);
    }
    if arguments
        .iter()
        .any(|argument| argument == "--check-config")
    {
        match cdr_mcp_server::server::GatewayConfig::from_env() {
            Ok(config) => {
                eprintln!(
                    "rust_mcp_gateway_config_ok configured_devices={} bind={}",
                    config.configured_device_count(),
                    config.bind_address()
                );
                return;
            }
            Err(error) => {
                eprintln!("rust_mcp_gateway_config_failed error={error}");
                std::process::exit(1);
            }
        }
    }
    if let Err(error) = cdr_mcp_server::server::run_from_env().await {
        eprintln!("rust_mcp_gateway_failed error={error}");
        std::process::exit(1);
    }
}

async fn healthcheck() -> Result<(), String> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let mut stream = tokio::net::TcpStream::connect("127.0.0.1:8030")
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .map_err(|error| error.to_string())?;
    let mut response = [0_u8; 64];
    let bytes = stream
        .read(&mut response)
        .await
        .map_err(|error| error.to_string())?;
    let status = std::str::from_utf8(&response[..bytes]).map_err(|error| error.to_string())?;
    if status.starts_with("HTTP/1.1 200") || status.starts_with("HTTP/1.0 200") {
        Ok(())
    } else {
        Err("health endpoint did not return HTTP 200".into())
    }
}
