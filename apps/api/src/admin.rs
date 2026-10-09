//! 營運後台：STAFF 守衛 + 清單/使用者搜尋 + system-flags + 簡易統計。
//! 檢舉佇列與下架在 reports.rs（同樣使用 `Staff`）。
use crate::{error::AppError, session::CurrentUser, AppState};
use axum::{
    extract::{FromRequestParts, Path, Query, State},
    http::{header, request::Parts, HeaderValue},
    response::{IntoResponse, Response},
    routing::{get, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/wishlists", get(list_wishlists))
        .route("/admin/users", get(list_users))
        .route("/admin/system-flags", get(list_flags))
        .route("/admin/system-flags/{key}", put(set_flag))
        .route("/admin/stats", get(stats))
}

/// 已登入且 is_staff；否則 401 / 403 STAFF_ONLY。
pub struct Staff(pub Uuid);

impl FromRequestParts<AppState> for Staff {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, AppError> {
        let u = CurrentUser::from_request_parts(parts, st).await?;
        if u.is_staff { Ok(Staff(u.id)) } else { Err(AppError::problem(403, "STAFF_ONLY", "僅營運人員可使用")) }
    }
}

pub async fn audit<'e, E: sqlx::PgExecutor<'e>>(ex: E, staff: Uuid, action: &str, entity: &str, entity_id: Option<Uuid>, diff: Value) -> Result<(), AppError> {
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff) VALUES ('staff', $1, $2, $3, $4, $5)")
        .bind(staff).bind(action).bind(entity).bind(entity_id).bind(diff).execute(ex).await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct PageQ { pub cursor: Option<Uuid>, pub limit: Option<i64>, pub q: Option<String>, pub status: Option<String>, pub moderation_status: Option<String> }

pub fn limit(q: &PageQ) -> i64 { q.limit.unwrap_or(20).clamp(1, 100) }

/// rows 為 limit+1 筆 (id, json)；cursor 為最後一筆 id（uuid v7 時間序，opaque 給前端）。
pub fn page(mut rows: Vec<(Uuid, Value)>, limit: i64) -> Response {
    let next = if rows.len() as i64 > limit { rows.truncate(limit as usize); rows.last().map(|r| r.0) } else { None };
    let mut res = Json(json!({ "data": rows.into_iter().map(|r| r.1).collect::<Vec<_>>(), "next_cursor": next })).into_response();
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    res
}

async fn list_wishlists(State(st): State<AppState>, Staff(sid): Staff, Query(q): Query<PageQ>) -> Result<Response, AppError> {
    if let Some(m) = &q.moderation_status { if m != "ok" && m != "hidden" { return Err(AppError::invalid("/moderation_status", "INVALID", "ok | hidden")); } }
    let qs = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = qs { audit(&st.pool, sid, "admin.search", "wishlists", None, json!({ "q": s })).await?; }
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(
        "SELECT w.id, jsonb_build_object('id', w.id, 'slug', w.slug::text, 'title', w.title, 'status', w.status::text, 'type', w.type::text,
           'owner', jsonb_build_object('id', u.id, 'display_name', u.display_name, 'email', u.email),
           'moderation_status', w.moderation_status::text,
           'open_report_count', (SELECT count(*) FROM content_reports r WHERE r.wishlist_id = w.id AND r.status = 'open'),
           'created_at', w.created_at)
         FROM wishlists w JOIN users u ON u.id = w.owner_id
         WHERE w.deleted_at IS NULL
           AND ($1::text IS NULL OR w.slug::text = $1 OR left(u.email, length($1)) = lower($1))
           AND ($2::text IS NULL OR w.moderation_status::text = $2)
           AND ($3::uuid IS NULL OR w.id > $3)
         ORDER BY w.id LIMIT $4")
        .bind(qs).bind(&q.moderation_status).bind(q.cursor).bind(limit(&q) + 1).fetch_all(&st.pool).await?;
    Ok(page(rows, limit(&q)))
}

async fn list_users(State(st): State<AppState>, Staff(sid): Staff, Query(q): Query<PageQ>) -> Result<Response, AppError> {
    let qs = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = qs { audit(&st.pool, sid, "admin.search", "users", None, json!({ "q": s })).await?; }
    let qid = qs.and_then(|s| s.parse::<Uuid>().ok());
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(
        "SELECT u.id, jsonb_build_object('id', u.id, 'display_name', u.display_name, 'email', u.email, 'is_staff', u.is_staff,
           'blocked', false, 'deleted', u.deleted_at IS NOT NULL,
           'wishlist_count', (SELECT count(*) FROM wishlists w WHERE w.owner_id = u.id AND w.deleted_at IS NULL),
           'created_at', u.created_at)
         FROM users u
         WHERE ($1::text IS NULL OR u.id = $2 OR left(u.email, length($1)) = lower($1))
           AND ($3::uuid IS NULL OR u.id > $3)
         ORDER BY u.id LIMIT $4")
        .bind(qs).bind(qid).bind(q.cursor).bind(limit(&q) + 1).fetch_all(&st.pool).await?;
    Ok(page(rows, limit(&q)))
}

async fn list_flags(State(st): State<AppState>, _s: Staff) -> Result<Json<Value>, AppError> {
    let rows: Vec<(String, Value, chrono::DateTime<chrono::Utc>)> = sqlx::query_as("SELECT key, value, updated_at FROM system_flags ORDER BY key").fetch_all(&st.pool).await?;
    Ok(Json(json!({ "data": rows.into_iter().map(|(k, v, t)| json!({ "key": k, "value": v, "updated_at": t })).collect::<Vec<_>>() })))
}

/// 白名單 key：目前僅 read_only（boolean）。READ_ONLY_MODE 的 503 攔截不在本模組。
async fn set_flag(State(st): State<AppState>, Staff(sid): Staff, Path(key): Path<String>, Json(b): Json<Value>) -> Result<Json<Value>, AppError> {
    if key != "read_only" { return Err(AppError::NotFound); }
    let v = b.get("value").filter(|v| v.is_boolean()).cloned().ok_or_else(|| AppError::invalid("/value", "INVALID", "value 必須為 boolean"))?;
    let mut tx = st.pool.begin().await?;
    let (updated_at,): (chrono::DateTime<chrono::Utc>,) = sqlx::query_as(
        "INSERT INTO system_flags (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value RETURNING updated_at")
        .bind(&key).bind(&v).fetch_one(&mut *tx).await?;
    audit(&mut *tx, sid, "system_flag.set", "system_flags", None, json!({ "key": key, "value": v })).await?;
    tx.commit().await?;
    tracing::warn!(key = %key, value = %v, "system flag changed");
    Ok(Json(json!({ "key": key, "value": v, "updated_at": updated_at })))
}

/// 最簡統計：直接聚合現有表，不引入第三方。
pub async fn stats_json(pool: &PgPool) -> Result<Value, sqlx::Error> {
    let (v,): (Value,) = sqlx::query_as(
        "SELECT jsonb_build_object(
           'users', (SELECT count(*) FROM users WHERE deleted_at IS NULL),
           'wishlists', (SELECT count(*) FROM wishlists WHERE deleted_at IS NULL),
           'active_wishlists', (SELECT count(*) FROM wishlists WHERE deleted_at IS NULL AND status = 'active'),
           'hidden_wishlists', (SELECT count(*) FROM wishlists WHERE moderation_status = 'hidden'),
           'claims_by_status', (SELECT COALESCE(jsonb_object_agg(s, n), '{}') FROM (SELECT status::text s, count(*) n FROM claims GROUP BY 1) t),
           'open_reports', (SELECT count(*) FROM content_reports WHERE status = 'open'),
           'notifications_by_status', (SELECT COALESCE(jsonb_object_agg(s, n), '{}') FROM (SELECT status::text s, count(*) n FROM notifications GROUP BY 1) t),
           'audit_last_7d', (SELECT COALESCE(jsonb_object_agg(a, n), '{}') FROM (SELECT action a, count(*) n FROM audit_logs WHERE created_at > now() - interval '7 days' GROUP BY 1) t),
           'flags', (SELECT COALESCE(jsonb_object_agg(key, value), '{}') FROM system_flags))")
        .fetch_one(pool).await?;
    Ok(v)
}

async fn stats(State(st): State<AppState>, _s: Staff) -> Result<Json<Value>, AppError> { Ok(Json(stats_json(&st.pool).await?)) }
