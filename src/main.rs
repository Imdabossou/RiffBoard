//! RiffBoard server: a public, play-only song library with highscores.
//!
//! `riffboard`            run the server (configured from the environment)
//! `riffboard admin ...`  operator tools, see `admin::USAGE`
//! `riffboard healthcheck` exit 0 if the local server answers (container probe)

mod admin;
mod chart;
mod config;
mod guard;
mod library;
mod pack;
mod settings;
mod store;
mod tunings;
mod util;
mod util_url;
mod web;

use std::net::SocketAddr;
use std::sync::Arc;

use serde_json::Value;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("admin") {
        std::process::exit(admin::run(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("healthcheck") {
        std::process::exit(admin::healthcheck());
    }
    if matches!(
        args.first().map(String::as_str),
        Some("-h" | "--help" | "help")
    ) {
        println!("riffboard [admin <command>]\n\n{}", admin::USAGE);
        return;
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("LOG_LEVEL")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(serve());
}

async fn serve() {
    let cfg = config::Config::from_env();
    if let Err(e) = std::fs::create_dir_all(&cfg.config_dir) {
        tracing::error!("can't create CONFIG_DIR {}: {e}", cfg.config_dir.display());
        std::process::exit(1);
    }
    let store = store::Store::open(&cfg.config_dir).unwrap_or_else(|e| {
        tracing::error!("can't open the leaderboard database: {e}");
        std::process::exit(1);
    });
    let token = util::random_token(32);
    store::write_private(&cfg.config_dir.join("admin.token"), &format!("{token}\n"));
    let plugins: Value = std::fs::read_to_string(cfg.plugins_dir.join("registry.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| {
            tracing::warn!("plugins/registry.json missing or invalid: no plugins loaded");
            Value::Array(vec![])
        });
    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port)
        .parse()
        .unwrap_or_else(|_| ([0, 0, 0, 0], cfg.port).into());
    let rescan_every = std::time::Duration::from_secs(cfg.rescan_secs);
    let state = Arc::new(web::AppState::new(cfg, store, token, plugins));

    let st = state.clone();
    tokio::task::spawn_blocking(move || st.rescan()).await.ok();
    tracing::info!("library: {} songs", state.index.all().len());
    let st = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(rescan_every).await;
            let s = st.clone();
            let _ = tokio::task::spawn_blocking(move || s.rescan()).await;
        }
    });

    let app = web::router(state);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("can't listen on {addr}: {e}");
            std::process::exit(1);
        });
    // Without TCP_NODELAY, small responses written in two segments wait on the
    // client's delayed ACK (a flat ~200 ms per request).
    use axum::serve::ListenerExt;
    let listener = listener.tap_io(|tcp| {
        let _ = tcp.set_nodelay(true);
    });
    tracing::info!(
        "RiffBoard {} listening on http://{addr}",
        env!("CARGO_PKG_VERSION")
    );
    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("signal");
            tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
        }
        #[cfg(not(unix))]
        let _ = ctrl_c.await;
    };
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .expect("server error");
}
