//! RFC 9457 problem+json（錯誤碼總表見 docs/04 第 8 章）
use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    NotFound,
    WishlistRemoved,
    Unauthorized,
    Db(sqlx::Error),
    /// 通用 problem（切片 3）：任意 status/code，可附 errors[] 與 Retry-After
    /// 切片 1+2：problem + 額外頂層欄位（remaining / claim_id）
    Extra { status: u16, code: &'static str, detail: String, extra: serde_json::Value },
    Problem { status: u16, code: &'static str, detail: String, errors: Option<serde_json::Value>, retry_after: Option<u64> },
}

impl AppError {
    pub fn problem(status: u16, code: &'static str, detail: impl Into<String>) -> Self {
        AppError::Problem { status, code, detail: detail.into(), errors: None, retry_after: None }
    }
    /// 422 VALIDATION_FAILED，單一欄位錯誤
    pub fn invalid(pointer: &str, code: &str, detail: &str) -> Self {
        AppError::Problem { status: 422, code: "VALIDATION_FAILED", detail: detail.into(),
            errors: Some(json!([{ "pointer": pointer, "code": code, "detail": detail }])), retry_after: None }
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::Db(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if let AppError::Extra { status, code, detail, extra } = self {
            let mut body = json!({
                "type": format!("https://wishsync.tw/problems/{}", code.to_lowercase().replace('_', "-")),
                "title": code, "status": status, "code": code, "detail": detail,
            });
            if let (Some(b), Some(x)) = (body.as_object_mut(), extra.as_object()) { b.extend(x.clone()); }
            let st = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            return (st, [(header::CONTENT_TYPE, "application/problem+json")], Json(body)).into_response();
        }
        if let AppError::Problem { status, code, detail, errors, retry_after } = self {
            let st = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let mut body = json!({
                "type": format!("https://wishsync.tw/problems/{}", code.to_lowercase().replace('_', "-")),
                "title": code, "status": status, "code": code, "detail": detail,
            });
            if let Some(e) = errors { body["errors"] = e; }
            let mut res = (st, [(header::CONTENT_TYPE, "application/problem+json")], Json(body)).into_response();
            if let Some(r) = retry_after { res.headers_mut().insert(header::RETRY_AFTER, r.into()); }
            return res;
        }
        let (status, code, title) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND", "找不到資源"),
            AppError::WishlistRemoved => (StatusCode::GONE, "WISHLIST_REMOVED", "此清單已被下架"),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "請先登入"),
            AppError::Problem { .. } | AppError::Extra { .. } => unreachable!(),
            AppError::Db(e) => {
                tracing::error!(error = %e, "database error");
                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", "系統錯誤")
            }
        };
        let body = json!({
            "type": format!("https://wishsync.tw/problems/{}", code.to_lowercase().replace('_', "-")),
            "title": title,
            "status": status.as_u16(),
            "code": code,
        });
        (status, [(header::CONTENT_TYPE, "application/problem+json")], Json(body)).into_response()
    }
}
