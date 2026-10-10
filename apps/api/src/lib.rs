pub mod account;
pub mod config;
pub mod admin;
pub mod dashboard;
pub mod error;
pub mod notify;
pub mod reports;
pub mod wishlists;
pub mod uploads;
pub mod validate;
pub mod auth;
pub mod auth_ext;
pub mod public;
pub mod guest;
pub mod claims;
pub mod idempotency;
pub mod ratelimit;
pub mod session;
pub mod points;
pub mod sealed;
pub mod contributions;
pub mod wallet;
pub mod orders;

use axum::{response::IntoResponse, Router};
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
            .merge(contributions::routes())
            .merge(wallet::routes())
            .merge(dashboard::routes())
            .merge(reports::routes())
            .merge(admin::routes())
            .merge(orders::routes())
            .merge(account::routes())
            .merge(wishlists::routes())
            .merge(uploads::routes())
            .merge(auth::routes()))
        .layer(axum::middleware::from_fn_with_state(state.clone(), read_only))
        .layer(cors())
        .layer(axum::middleware::map_response(problem_json))
        .layer(axum::middleware::map_response(security_headers))
        .with_state(state)
}

/// F-18：所有回應加安全標頭；已自行設定者（如公開清單的 Cache-Control）不覆蓋。
async fn security_headers(mut res: axum::response::Response) -> axum::response::Response {
    use axum::http::{header, HeaderValue};
    let h = res.headers_mut();
    for (k, v) in [(header::X_CONTENT_TYPE_OPTIONS, "nosniff"), (header::REFERRER_POLICY, "no-referrer"), (header::CACHE_CONTROL, "no-store")] {
        h.entry(k).or_insert(HeaderValue::from_static(v));
    }
    res
}

/// F-12：框架層 rejection（text/plain 或無 content-type 的 4xx）改為 RFC 9457 problem+json；
/// 原始文字（可能含內部型別名）只寫 tracing debug，不回給客戶端。自家 problem+json 不動。
async fn problem_json(res: axum::response::Response) -> axum::response::Response {
    use axum::http::{header, HeaderValue, StatusCode};
    let st = res.status();
    let ct = res.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !st.is_client_error() || !(ct.is_empty() || ct.starts_with("text/plain")) { return res; }
    let (parts, body) = res.into_parts();
    let raw = axum::body::to_bytes(body, 4096).await.unwrap_or_default();
    tracing::debug!(status = st.as_u16(), rejection = %String::from_utf8_lossy(&raw), "framework rejection");
    let (code, title) = match st {
        StatusCode::NOT_FOUND => ("NOT_FOUND", "找不到資源"),
        StatusCode::METHOD_NOT_ALLOWED => ("METHOD_NOT_ALLOWED", "不支援的請求方法"),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ("UNSUPPORTED_MEDIA_TYPE", "不支援的內容類型，請使用 application/json"),
        StatusCode::PAYLOAD_TOO_LARGE => ("PAYLOAD_TOO_LARGE", "請求內容過大"),
        StatusCode::UNPROCESSABLE_ENTITY => ("VALIDATION_FAILED", "請求內容格式不正確"),
        _ => ("BAD_REQUEST", "請求格式不正確"),
    };
    let body = serde_json::json!({
        "type": format!("https://wishsync.tw/problems/{}", code.to_lowercase().replace('_', "-")),
        "title": title, "status": st.as_u16(), "code": code, "detail": title,
    });
    let mut out = (st, axum::Json(body)).into_response();
    for (k, v) in parts.headers.iter() {
        if k != header::CONTENT_TYPE && k != header::CONTENT_LENGTH { out.headers_mut().append(k.clone(), v.clone()); }
    }
    out.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/problem+json"));
    out
}

/// 前端（Pages）與 API 不同網域：只允許 APP_URL，並帶憑證。
fn cors() -> tower_http::cors::CorsLayer {
    use axum::http::{header, HeaderName, Method};
    let origin = config::get().app_url;
    tower_http::cors::CorsLayer::new()
        .allow_origin(origin.parse::<axum::http::HeaderValue>().expect("APP_URL"))
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE, HeaderName::from_static("x-guest-token"), HeaderName::from_static("idempotency-key")])
        .expose_headers([HeaderName::from_static("idempotency-replayed")])
}

/// system_flags.read_only = true → 非安全方法回 503 READ_ONLY_MODE（退訂 POST 放行：保護性的使用者操作，且只寫退訂偏好；admin system-flags、登出與登入除外：否則 staff 登出後無法登入來關閉唯讀）
async fn read_only(
    axum::extract::State(st): axum::extract::State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::{http::Method, response::IntoResponse};
    let p = req.uri().path();
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS)
        || p.starts_with("/api/v1/admin/system-flags") || matches!(p, "/api/v1/auth/logout" | "/api/v1/auth/login" | "/api/v1/auth/otp/request" | "/api/v1/auth/otp/verify" | "/api/v1/unsubscribe");
    if !safe {
        match sqlx::query_scalar::<_, serde_json::Value>("SELECT value FROM system_flags WHERE key = 'read_only'").fetch_optional(&st.pool).await {
            Ok(v) if v.as_ref().and_then(|v| v.as_bool()) != Some(true) => {}
            Ok(_) => return error::AppError::Problem { status: 503, code: "READ_ONLY_MODE", detail: "系統維護中，稍後再試".into(), errors: None, retry_after: Some(60) }.into_response(),
            Err(e) => return error::AppError::Db(e).into_response(),
        }
    }
    next.run(req).await
}
