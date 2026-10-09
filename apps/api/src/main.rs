use sqlx::postgres::PgPoolOptions;
use wishsync_api::{app, AppState};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = PgPoolOptions::new().max_connections(10).connect(&url).await.expect("db");
    sqlx::migrate!("../../db/migrations").run(&pool).await.expect("migrate");
    let listener = tokio::net::TcpListener::bind(std::env::var("BIND").unwrap_or("0.0.0.0:8080".into())).await.unwrap();
    wishsync_api::notify::spawn_worker(pool.clone());
    axum::serve(listener, app(AppState { pool })).await.unwrap();
}
