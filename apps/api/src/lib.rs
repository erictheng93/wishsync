pub mod account;
pub mod admin;
pub mod dashboard;
pub mod error;
pub mod notify;
pub mod reports;
pub mod wishlists;
pub mod uploads;
pub mod auth;
pub mod auth_ext;
pub mod public;
pub mod guest;
pub mod claims;
pub mod idempotency;
pub mod session;

use axum::Router;
use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
}

/// 各模組各自提供 `routes()`，在此合併；新增模組只動這一處。
pub fn app(state: AppState) -> Router {
    Router::new()
        .nest("/api/v1", Router::new().merge(public::routes())
            .merge(guest::routes())
            .merge(claims::routes())
            .merge(dashboard::routes())
            .merge(reports::routes())
            .merge(admin::routes())
            .merge(account::routes())
            .merge(wishlists::routes())
            .merge(uploads::routes())
            .merge(auth::routes()))
        .layer(axum::middleware::from_fn_with_state(state.clone(), read_only))
        .layer(cors())
        .with_state(state)
}

/// 前端（Pages）與 API 不同網域：只允許 APP_URL，並帶憑證。
fn cors() -> tower_http::cors::CorsLayer {
    use axum::http::{header, HeaderName, Method};
    let origin = std::env::var("APP_URL").unwrap_or("http://localhost:3000".into());
    tower_http::cors::CorsLayer::new()
        .allow_origin(origin.parse::<axum::http::HeaderValue>().expect("APP_URL"))
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE, HeaderName::from_static("x-guest-token"), HeaderName::from_static("idempotency-key")])
        .expose_headers([HeaderName::from_static("idempotency-replayed")])
}

/// system_flags.read_only = true → 非安全方法回 503 READ_ONLY_MODE（admin system-flags、登出與登入除外：否則 staff 登出後無法登入來關閉唯讀）
async fn read_only(
    axum::extract::State(st): axum::extract::State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::{http::Method, response::IntoResponse};
    let p = req.uri().path();
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS)
        || p.starts_with("/api/v1/admin/system-flags") || matches!(p, "/api/v1/auth/logout" | "/api/v1/auth/login" | "/api/v1/auth/otp/request" | "/api/v1/auth/otp/verify");
    if !safe {
        match sqlx::query_scalar::<_, serde_json::Value>("SELECT value FROM system_flags WHERE key = 'read_only'").fetch_optional(&st.pool).await {
            Ok(v) if v.as_ref().and_then(|v| v.as_bool()) != Some(true) => {}
            Ok(_) => return error::AppError::Problem { status: 503, code: "READ_ONLY_MODE", detail: "系統維護中，稍後再試".into(), errors: None, retry_after: Some(60) }.into_response(),
            Err(e) => return error::AppError::Db(e).into_response(),
        }
    }
    next.run(req).await
}
