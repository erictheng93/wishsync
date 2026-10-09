use sqlx::postgres::PgPoolOptions;
use wishsync_api::{app, AppState};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let cfg = wishsync_api::config::Config::from_env();
    if let Some(w) = cfg.bind_warning() { tracing::warn!("{w}"); eprintln!("WARN: {w}"); }
    let bind = cfg.bind.clone();
    wishsync_api::config::init(cfg);
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = PgPoolOptions::new().max_connections(10).connect(&url).await.expect("db");
    sqlx::migrate!("../../db/migrations").run(&pool).await.expect("migrate");
    let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
    wishsync_api::notify::spawn_worker(pool.clone());
    wishsync_api::ratelimit::spawn_cleanup(pool.clone());
    axum::serve(listener, app(AppState { pool }).into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap();
}
