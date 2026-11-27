use deepseek_harness_rs::config::HarnessConfig;
use deepseek_harness_rs::db::HarnessDb;
use deepseek_harness_rs::state::AppState;
use deepseek_harness_rs::web;
use std::net::SocketAddr;
use std::path::PathBuf;
use sysinfo::{Pid, System};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = HarnessConfig::default();
    let mut mode = std::env::var("HARNESS_MODE").unwrap_or_else(|_| "standalone".to_string());

    let args: Vec<String> = std::env::args().collect();
    // Version/help must exit before any server startup.
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("harness {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "harness {} — ultra-efficient agentic workspace runtime\n\n\
             USAGE:\n  harness [OPTIONS]\n\n\
             OPTIONS:\n\
             \x20 --mode <standalone|server>   Run mode (default: standalone)\n\
             \x20 --host <ADDR>                Bind address (default: 127.0.0.1)\n\
             \x20 --port <PORT>                Bind port (default: 8080)\n\
             \x20 --sandbox <PATH>             Workspace sandbox root\n\
             \x20 --db <PATH>                  SQLite database path\n\
             \x20 --server-id <ID>             Server identity for remote dispatch\n\
             \x20 --check-update               Check GitHub for a newer release\n\
             \x20 --update                     Download, verify and install the latest release\n\
             \x20 --version, -V                Print version and exit\n\
             \x20 --help, -h                   Print this help and exit\n\n\
             ENVIRONMENT:\n\
             \x20 HARNESS_AUTH_TOKEN           Require a bearer token on /api/*\n\
             \x20 HARNESS_MASTER_KEY           AES-256 key (hex or base64) for stored credentials\n\
             \x20 MODEL_API_KEY, MODEL_API_BASE, MODEL_NAME\n\
             \x20 HARNESS_UPDATE_REPO          owner/repo for auto-update\n",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" if i + 1 < args.len() => {
                mode = args[i + 1].clone();
                i += 1;
            }
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
            "--server-id" if i + 1 < args.len() => {
                config.server_identity = args[i + 1].clone();
                i += 1;
            }
            "--db" if i + 1 < args.len() => {
                config.db_path = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--check-update" => {
                let repo = config.update_repo.clone();
                match deepseek_harness_rs::updater::check(&repo, env!("CARGO_PKG_VERSION")).await {
                    Ok(info) if info.available => {
                        println!(
                            "Update available: {} -> {} ({})",
                            info.current_version, info.latest_version, info.html_url
                        );
                    }
                    Ok(info) => println!("Up to date (v{}).", info.current_version),
                    Err(e) => eprintln!("Update check failed: {e}"),
                }
                return Ok(());
            }
            "--update" => {
                let repo = config.update_repo.clone();
                let dest = std::env::temp_dir().join("harness-update");
                match deepseek_harness_rs::updater::update_now(&repo, env!("CARGO_PKG_VERSION"), &dest).await {
                    Ok(info) if info.available => {
                        println!(
                            "Installed v{} (was v{}). Restarting...",
                            info.latest_version, info.current_version
                        );
                        return Ok(());
                    }
                    Ok(info) => {
                        println!("Already up to date (v{}).", info.current_version);
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("Update failed: {e}");
                        return Err(e.into());
                    }
                }
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
    if let Ok(host_str) = std::env::var("HOST") {
        config.host = host_str;
    }
    if let Ok(db_str) = std::env::var("HARNESS_DB_PATH") {
        if !db_str.trim().is_empty() {
            config.db_path = Some(PathBuf::from(db_str));
        }
    }

    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    let is_server_only = mode.eq_ignore_ascii_case("server");

    // Fail safe: a non-loopback bind exposes a fully privileged API. Require a
    // token in that case so the harness cannot be driven by any local webpage.
    if !web::security::is_loopback_bind(&config.host) && config.auth_token.trim().is_empty() {
        return Err(
            "Refusing to bind to a non-loopback address without HARNESS_AUTH_TOKEN set. \
             Set HARNESS_AUTH_TOKEN or bind 127.0.0.1."
                .into(),
        );
    }

    let mut db_for_shutdown: Option<HarnessDb> = None;
    let state = if is_server_only {
        AppState::new(config.clone())
    } else {
        let db_path = config
            .db_path
            .clone()
            .unwrap_or_else(|| config.sandbox_root.join("harness.db"));
        let db = HarnessDb::open(&db_path)?;
        db_for_shutdown = Some(db.clone());
        AppState::with_db(config.clone(), db)
    };

    // Start the automation scheduler (not in pure execution-server mode).
    if !is_server_only {
        deepseek_harness_rs::automation_runner::spawn_scheduler(std::sync::Arc::new(state.clone()));
    }

    let router = if is_server_only {
        web::build_server_router(state.clone())
    } else {
        web::build_router(state.clone())
    };

    // Background auto-update: check, download with progress, verify, install,
    // and relaunch - fully automatic on launch (unless disabled in settings).
    // The pipeline reads the live config itself, so the settings toggles take
    // effect without a restart. Execution-server mode only reports availability.
    if !is_server_only {
        deepseek_harness_rs::auto_update::spawn(
            std::sync::Arc::new(state.clone()),
            env!("CARGO_PKG_VERSION"),
        );
    } else {
        let repo = {
            let conf = state.config.read().await;
            conf.update_repo.clone()
        };
        tokio::spawn(async move {
            match deepseek_harness_rs::updater::check(&repo, env!("CARGO_PKG_VERSION")).await {
                Ok(info) if info.available => println!(
                    "Update available: v{} -> v{} ({})",
                    info.current_version, info.latest_version, info.html_url
                ),
                Ok(_) => {}
                Err(e) => eprintln!("[harness-update] check failed: {e}"),
            }
        });
    }

    let mut sys = System::new();
    let pid = Pid::from_u32(std::process::id());
    sys.refresh_process(pid);
    let start_rss = sys
        .process(pid)
        .map(|p| p.memory() as f64 / 1024.0 / 1024.0)
        .unwrap_or(0.0);

    println!("============================================================");
    println!(
        "              🦀 Harness Workspace Runtime v{}           ",
        env!("CARGO_PKG_VERSION")
    );
    println!("============================================================");
    println!("Mode                : {}", mode);
    println!("Listening on        : http://{}", addr);
    println!("Workspace Sandbox   : {}", config.sandbox_root.display());
    if !is_server_only {
        let db_p = config
            .db_path
            .clone()
            .unwrap_or_else(|| config.sandbox_root.join("harness.db"));
        println!("SQLite Database     : {}", db_p.display());
    }
    println!("Starting RSS RAM    : {:.2} MB", start_rss);
    println!("============================================================");

    // A relaunched process can start before its predecessor released the port, so
    // retry briefly instead of crashing (this is the normal update path).
    let listener =
        deepseek_harness_rs::updater::bind_with_retry(addr, std::time::Duration::from_secs(30))
            .await?;
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // Reclaim storage + checkpoint WAL on clean shutdown (best-effort).
    if let Some(db) = db_for_shutdown {
        match db.optimize() {
            Ok(()) => println!("Database checkpointed and optimized."),
            Err(e) => eprintln!("[harness-db] shutdown optimize failed: {e}"),
        }
    }

    println!("\nHarness shut down cleanly.");
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    println!("\nShutdown signal received, draining active tasks...");
}
