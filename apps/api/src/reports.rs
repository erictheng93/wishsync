//! 檢舉（公開）+ staff 處理檢舉 / 下架與恢復清單（docs/04 4.11）。
use crate::{
    admin::{audit, limit, page, PageQ, Staff},
    dashboard,
    error::AppError,
    ratelimit,
    session::{hash_token, cookie_value, CurrentUser},
    AppState,
};
use axum::{
    extract::{FromRequestParts, Path, Query, State},
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/public/wishlists/{slug}/reports", post(create))
        .route("/admin/reports", get(list))
        .route("/admin/reports/{id}", patch(handle))
        .route("/admin/wishlists/{id}/moderation", patch(moderate))
}

#[derive(Deserialize)]
struct NewReport { reason: String, detail: Option<String>, item_id: Option<Uuid>, turnstile_token: Option<String> }

/// Turnstile siteverify。未設 secret（僅 dev/test）略過。契約錯誤碼表：Turnstile 失敗 = 403 FORBIDDEN（不是 422，
/// 因為欄位格式本身合法、是人機驗證不通過）；siteverify 連不上 = 503（fail closed，不放行）。secret 不寫入 log。
pub async fn verify_turnstile(cfg: &crate::config::Config, token: Option<&str>, ip: &str) -> Result<(), AppError> {
    let Some(secret) = cfg.turnstile_secret.as_deref() else { return Ok(()) };
    let fail = || AppError::problem(403, "FORBIDDEN", "人機驗證失敗，請重新整理後再試");
    let token = token.map(str::trim).filter(|t| !t.is_empty()).ok_or_else(fail)?;
    let mut form = vec![("secret", secret), ("response", token)];
    if ip != "unknown" { form.push(("remoteip", ip)); }
    let res = reqwest::Client::new().post(&cfg.turnstile_verify_url).form(&form).timeout(std::time::Duration::from_secs(5)).send().await;
    let v: Value = match res {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or(Value::Null),
        Ok(r) => { tracing::error!(status = %r.status(), "turnstile siteverify"); return Err(AppError::problem(503, "SERVICE_UNAVAILABLE", "人機驗證服務暫時無法使用")); }
        Err(e) => { tracing::error!(error = %e.without_url(), "turnstile siteverify"); return Err(AppError::problem(503, "SERVICE_UNAVAILABLE", "人機驗證服務暫時無法使用")); }
    };
    if v["success"] == true { Ok(()) } else { Err(fail()) }
}

async fn create(State(st): State<AppState>, Path(slug): Path<String>, mut parts: Parts, Json(b): Json<NewReport>) -> Result<Response, AppError> {
    if !["scam", "inappropriate", "copyright", "personal_info", "other"].contains(&b.reason.as_str()) {
        return Err(AppError::invalid("/reason", "INVALID", "reason 不合法"));
    }
    if b.detail.as_deref().is_some_and(|d| d.chars().count() > 1000) { return Err(AppError::invalid("/detail", "TOO_LONG", "最多 1000 字")); }
    let w: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, moderation_status::text FROM wishlists
         WHERE slug = $1 AND deleted_at IS NULL AND visibility <> 'private' AND status IN ('active', 'closed')")
        .bind(&slug).fetch_optional(&st.pool).await?;
    let (wid, m) = w.ok_or(AppError::NotFound)?;
    if m == "hidden" { return Err(AppError::WishlistRemoved); }

    // 檢舉者（選填）：session 或 guest token（X-Guest-Token / ws_guest，DB 存 SHA-256(token 字串)）
    let user = CurrentUser::from_request_parts(&mut parts, &st).await.ok().map(|u| u.id);
    let mut guest: Option<Uuid> = None;
    if user.is_none() {
        let tok = parts.headers.get("x-guest-token").and_then(|v| v.to_str().ok()).or_else(|| cookie_value(&parts, "ws_guest"));
        if let Some(t) = tok {
            guest = sqlx::query_scalar("SELECT id FROM guests WHERE guest_token_hash = $1 AND deleted_at IS NULL")
                .bind(hash_token(t)).fetch_optional(&st.pool).await?;
        }
    }
    if user.is_some() || guest.is_some() {
        let dup: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM content_reports WHERE wishlist_id = $1 AND created_at > now() - interval '24 hours'
               AND reporter_user_id IS NOT DISTINCT FROM $2 AND reporter_guest_id IS NOT DISTINCT FROM $3 LIMIT 1")
            .bind(wid).bind(user).bind(guest).fetch_optional(&st.pool).await?;
        if let Some(id) = dup { return Ok((StatusCode::CREATED, Json(json!({ "id": id, "status": "open" }))).into_response()); }
    }
    let ip = crate::config::client_ip(&parts, &crate::config::get());
    ratelimit::check(&st.pool, &format!("report_ip:{ip}"), 10, 3600).await?;
    verify_turnstile(&crate::config::get(), b.turnstile_token.as_deref(), &ip).await?;
    if user.is_none() && guest.is_none() {
        // 匿名檢舉者以 IP 近似「同一檢舉者」：同清單 24 小時 1 筆
        ratelimit::check(&st.pool, &format!("report_wl:{wid}:{ip}"), 1, 86400).await?;
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO content_reports (wishlist_id, item_id, reason, detail, reporter_user_id, reporter_guest_id)
         VALUES ($1, $2, $3::report_reason, $4, $5, $6) RETURNING id")
        .bind(wid).bind(b.item_id).bind(&b.reason).bind(&b.detail).bind(user).bind(guest).fetch_one(&st.pool).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "status": "open" }))).into_response())
}

const REPORT_SQL: &str =
    "SELECT r.id, jsonb_build_object('id', r.id, 'wishlist', jsonb_build_object('id', w.id, 'slug', w.slug::text, 'title', w.title),
       'item_id', r.item_id, 'reason', r.reason::text, 'detail', r.detail, 'status', r.status::text,
       'reporter', CASE WHEN r.reporter_user_id IS NOT NULL THEN 'user' WHEN r.reporter_guest_id IS NOT NULL THEN 'guest' ELSE 'anonymous' END,
       'handled_at', r.handled_at, 'created_at', r.created_at)
     FROM content_reports r JOIN wishlists w ON w.id = r.wishlist_id";

async fn list(State(st): State<AppState>, _s: Staff, Query(q): Query<PageQ>) -> Result<Response, AppError> {
    let status = q.status.clone().unwrap_or_else(|| "open".into());
    if !["open", "actioned", "dismissed"].contains(&status.as_str()) { return Err(AppError::invalid("/status", "INVALID", "open | actioned | dismissed")); }
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(&format!(
        "{REPORT_SQL} WHERE r.status = $1::report_status AND ($2::uuid IS NULL OR r.id > $2) ORDER BY r.id LIMIT $3"))
        .bind(status).bind(q.cursor).bind(limit(&q) + 1).fetch_all(&st.pool).await?;
    Ok(page(rows, limit(&q)))
}

#[derive(Deserialize)]
struct Handle { status: String, note: Option<String> }

async fn handle(State(st): State<AppState>, Staff(sid): Staff, Path(id): Path<Uuid>, Json(b): Json<Handle>) -> Result<Json<Value>, AppError> {
    if b.status != "actioned" && b.status != "dismissed" { return Err(AppError::invalid("/status", "INVALID", "actioned | dismissed")); }
    let mut tx = st.pool.begin().await?;
    let cur: String = sqlx::query_scalar("SELECT status::text FROM content_reports WHERE id = $1 FOR UPDATE")
        .bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    if cur != "open" { return Err(AppError::problem(409, "INVALID_STATE_TRANSITION", "檢舉已處理")); }
    sqlx::query("UPDATE content_reports SET status = $2::report_status, handled_by = $3, handled_at = now() WHERE id = $1")
        .bind(id).bind(&b.status).bind(sid).execute(&mut *tx).await?;
    audit(&mut *tx, sid, "report.handle", "content_reports", Some(id), json!({ "status": b.status, "note": b.note })).await?;
    let (_, v): (Uuid, Value) = sqlx::query_as(&format!("{REPORT_SQL} WHERE r.id = $1")).bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
struct Moderate { moderation_status: String, reason: Option<String> }

async fn moderate(State(st): State<AppState>, Staff(sid): Staff, Path(id): Path<Uuid>, Json(b): Json<Moderate>) -> Result<Json<Value>, AppError> {
    let hidden = match b.moderation_status.as_str() {
        "hidden" => true,
        "ok" => false,
        _ => return Err(AppError::invalid("/moderation_status", "INVALID", "ok | hidden")),
    };
    let reason = b.reason.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if hidden && reason.is_none() { return Err(AppError::invalid("/reason", "REQUIRED", "下架必須填原因")); }
    let mut tx = st.pool.begin().await?;
    let row: Option<(String, String, String, Option<String>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "UPDATE wishlists SET moderation_status = $2::moderation_status, moderation_reason = $3, moderated_at = now(), moderated_by = $4
         WHERE id = $1 AND deleted_at IS NULL RETURNING id::text, slug::text, moderation_status::text, moderation_reason, moderated_at")
        .bind(id).bind(&b.moderation_status).bind(if hidden { reason } else { None }).bind(sid).fetch_optional(&mut *tx).await?;
    let (_, slug, ms, mr, at) = row.ok_or(AppError::NotFound)?;
    audit(&mut *tx, sid, "wishlist.moderate", "wishlists", Some(id), json!({ "status": b.moderation_status, "reason": reason })).await?;
    if hidden {
        sqlx::query("UPDATE content_reports SET status = 'actioned', handled_by = $2, handled_at = now() WHERE wishlist_id = $1 AND status = 'open'")
            .bind(id).bind(sid).execute(&mut *tx).await?;
    }
    dashboard::notify(&mut *tx, id).await?; // commit 時才送出，SSE 收到後關閉連線
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "slug": slug, "moderation_status": ms, "moderation_reason": mr, "moderated_at": at })))
}
