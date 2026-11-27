use deepseek_harness_rs::config::HarnessConfig;
use deepseek_harness_rs::db::HarnessDb;
use deepseek_harness_rs::state::AppState;
use deepseek_harness_rs::web;
use std::net::SocketAddr;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = HarnessConfig::default();
    config.port = 8080;

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("harness-client {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "harness-client {} — standalone client (Web UI + history database)\n\n\
             USAGE:\n  harness-client [OPTIONS]\n\n\
             OPTIONS:\n\
             \x20 --host <ADDR>       Bind address (default: 127.0.0.1)\n\
             \x20 --port <PORT>       Bind port (default: 8080)\n\
             \x20 --db <PATH>         SQLite history database path\n\
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
            "--db" if i + 1 < args.len() => {
                config.db_path = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    let db_path = config
        .db_path
        .clone()
        .unwrap_or_else(|| config.sandbox_root.join("harness.db"));
    let db = HarnessDb::open(&db_path)?;
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    let state = AppState::with_db(config.clone(), db);
    let router = web::build_router(state);

    println!("============================================================");
    println!("   💬 Harness Client (History DB + Multi-Server UI)   ");
    println!("============================================================");
    println!("Web UI            : http://{}", addr);
    println!("SQLite Database   : {}", db_path.display());
    println!("============================================================");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
