use deepseek_harness_rs::config::HarnessConfig;
use deepseek_harness_rs::state::AppState;
use deepseek_harness_rs::web;
use std::net::SocketAddr;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = HarnessConfig::default();
    config.port = 8081;
    config.server_identity = std::env::var("HARNESS_SERVER_ID").unwrap_or_else(|_| "server-1".to_string());

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("harness-server {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "harness-server {} — headless tool execution server\n\n\
             USAGE:\n  harness-server [OPTIONS]\n\n\
             OPTIONS:\n\
             \x20 --host <ADDR>       Bind address (default: 127.0.0.1)\n\
             \x20 --port <PORT>       Bind port (default: 8081)\n\
             \x20 --sandbox <PATH>    Workspace sandbox root\n\
             \x20 --id, --server-id <ID>  Server identity\n\
             \x20 --version, -V       Print version and exit\n\
             \x20 --help, -h          Print this help and exit\n",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--host" if i + 1 < args.len() => {
                config.host = args[i + 1].clone();
                i += 1;
            }
            "--port" if i + 1 < args.len() => {
                if let Ok(p) = args[i + 1].parse::<u16>() {
                    config.port = p;
                }
                i += 1;
            }
            "--sandbox" if i + 1 < args.len() => {
                config.sandbox_root = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            "--id" | "--server-id" if i + 1 < args.len() => {
                config.server_identity = args[i + 1].clone();
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    if let Ok(port_str) = std::env::var("PORT") {
        if let Ok(p) = port_str.parse::<u16>() {
            config.port = p;
        }
    }

    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    // Fail safe: never expose the tool-execution API on a non-loopback address
    // without a token. The server router can run arbitrary tools.
    if !web::security::is_loopback_bind(&config.host) && config.auth_token.trim().is_empty() {
        return Err(
            "Refusing to bind the execution server to a non-loopback address without \
             HARNESS_AUTH_TOKEN set. Set HARNESS_AUTH_TOKEN or bind 127.0.0.1."
                .into(),
        );
    }
    let state = AppState::new(config.clone());
    let router = web::build_server_router(state);

    println!("============================================================");
    println!("   🛠  Harness Tool Execution Server ({})   ", config.server_identity);
    println!("============================================================");
    println!("Listening on      : http://{}", addr);
    println!("Workspace Sandbox : {}", config.sandbox_root.display());
    println!("============================================================");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
