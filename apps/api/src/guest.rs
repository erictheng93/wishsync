//! 訪客身分（X-Guest-Token header 優先，其次 ws_guest cookie；DB 只存 SHA-256）與 /guest/me。
use crate::{claims::{ClaimRow, CLAIM_COLS}, error::AppError, session::{cookie_value, hash_token, CurrentUser}, AppState};
use axum::{
    extract::{FromRequestParts, Query, State},
    http::{header, request::Parts, HeaderValue},
    response::{IntoResponse, Response},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::FromRow;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new().route("/guest/me", get(me).patch(update_me).delete(delete_me)).route("/guest/recover", post(recover))
}

#[derive(Clone, Copy)]
pub enum Actor { Guest(Uuid), User(Uuid) }

/// 32 bytes 隨機（base64url）；回傳 (明文, hash)
pub fn new_token() -> (String, Vec<u8>) {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    let t = URL_SAFE_NO_PAD.encode(b);
    let h = hash_token(&t);
    (t, h)
}

pub fn cookie(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("ws_guest={token}; Path=/; Max-Age=31536000; HttpOnly; Secure; SameSite=Lax")).unwrap()
}

async fn guest_id(parts: &Parts, st: &AppState) -> Result<Option<Uuid>, AppError> {
    let tok = parts.headers.get("x-guest-token").and_then(|v| v.to_str().ok()).map(str::to_owned)
        .or_else(|| cookie_value(parts, "ws_guest").map(str::to_owned));
    let Some(tok) = tok else { return Ok(None) };
    let row: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM guests WHERE guest_token_hash = $1 AND deleted_at IS NULL")
        .bind(hash_token(&tok)).fetch_optional(&st.pool).await?;
    // 帶了但無效 → 401（避免靜默建立新訪客而丟失既有認領）
    row.map(|(id,)| Some(id)).ok_or(AppError::Unauthorized)
}

/// 有效 ws_session → User；否則有 guest token → Guest；皆無 → None
pub struct MaybeActor(pub Option<Actor>);

impl FromRequestParts<AppState> for MaybeActor {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, AppError> {
        match CurrentUser::from_request_parts(parts, st).await {
            Ok(u) => return Ok(MaybeActor(Some(Actor::User(u.id)))),
            Err(AppError::Db(e)) => return Err(e.into()),
            Err(_) => {}
        }
        Ok(MaybeActor(guest_id(parts, st).await?.map(Actor::Guest)))
    }
}

pub struct GuestAuth(pub Uuid);

impl FromRequestParts<AppState> for GuestAuth {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, AppError> {
        guest_id(parts, st).await?.map(GuestAuth).ok_or(AppError::Unauthorized)
    }
}

fn no_store(body: Value) -> Response {
    ([(header::CACHE_CONTROL, "private, no-store")], Json(body)).into_response()
}

#[derive(Deserialize)]
struct Page { limit: Option<i64>, cursor: Option<Uuid> }

#[derive(FromRow)]
struct Row {
    #[sqlx(flatten)] claim: ClaimRow,
    item_title: String, image_key: Option<String>, image_status: String,
    slug: String, wl_title: String, event_date: Option<chrono::NaiveDate>, wl_status: String,
}

/// 登入使用者認領時身分是 User（claims.user_id），也要看得到自己的認領；訪客則走 guest_id。
async fn me(State(st): State<AppState>, MaybeActor(actor): MaybeActor, Query(p): Query<Page>) -> Result<Response, AppError> {
    let (is_user, id) = match actor { Some(Actor::User(u)) => (true, u), Some(Actor::Guest(g)) => (false, g), None => return Err(AppError::Unauthorized) };
    let (name, contact): (String, Option<String>) = if is_user {
        sqlx::query_as("SELECT display_name, NULL FROM users WHERE id = $1").bind(id).fetch_one(&st.pool).await?
    } else {
        sqlx::query_as("SELECT display_name, contact FROM guests WHERE id = $1").bind(id).fetch_one(&st.pool).await?
    };
    let limit = p.limit.unwrap_or(20).clamp(1, 50);
    let mut rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {CLAIM_COLS}, i.title AS item_title, i.image_key, i.image_status::text AS image_status,
                w.slug::text AS slug, w.title AS wl_title, w.event_date, w.status::text AS wl_status
         FROM claims c JOIN wishlist_items i ON i.id = c.item_id JOIN wishlists w ON w.id = i.wishlist_id
         WHERE (CASE WHEN $4 THEN c.user_id ELSE c.guest_id END) = $1 AND ($2::uuid IS NULL OR c.id < $2) ORDER BY c.id DESC LIMIT $3"))
        .bind(id).bind(p.cursor).bind(limit + 1).bind(is_user).fetch_all(&st.pool).await?;
    let next = if rows.len() as i64 > limit { rows.truncate(limit as usize); rows.last().map(|r| r.claim.id) } else { None };
    let claims: Vec<Value> = rows.into_iter().map(|r| json!({
        "claim": r.claim,
        "item": { "id": r.claim.item_id, "title": r.item_title, "image_url": crate::public::image_url(r.image_key.as_deref(), &r.image_status) },
        "wishlist": { "slug": r.slug, "title": r.wl_title, "event_date": r.event_date, "status": r.wl_status },
    })).collect();
    Ok(no_store(json!({ "guest": { "display_name": name, "contact": contact, "is_user": is_user }, "claims": claims, "next_cursor": next })))
}

async fn update_me(State(st): State<AppState>, GuestAuth(gid): GuestAuth, body: axum::body::Bytes) -> Result<Response, AppError> {
    let v: Value = serde_json::from_slice(&body).map_err(|_| AppError::invalid("/", "INVALID", "請求內容格式錯誤"))?;
    let name = match v.get("display_name") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if (1..=30).contains(&s.trim().chars().count()) => Some(s.trim().to_owned()),
        _ => return Err(AppError::invalid("/display_name", "RANGE", "暱稱需為 1–30 字")),
    };
    let (set_contact, contact) = match v.get("contact") {
        None => (false, None),
        Some(Value::Null) => (true, None),
        Some(Value::String(s)) if s.chars().count() <= 100 => (true, Some(s.clone())),
        _ => return Err(AppError::invalid("/contact", "RANGE", "聯絡方式至多 100 字")),
    };
    let mut tx = st.pool.begin().await?;
    let (dn, ct): (String, Option<String>) = sqlx::query_as(
        "UPDATE guests SET display_name = COALESCE($2, display_name), contact = CASE WHEN $3 THEN $4 ELSE contact END
         WHERE id = $1 RETURNING display_name, contact")
        .bind(gid).bind(&name).bind(set_contact).bind(&contact).fetch_one(&mut *tx).await?;
    if name.is_some() {
        sqlx::query("UPDATE claims SET claimer_name = $2 WHERE guest_id = $1").bind(gid).bind(&dn).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(no_store(json!({ "guest": { "display_name": dn, "contact": ct } })))
}

/// D40：清除暱稱 / 聯絡 / email，claims 保留；之後該 token 回 401（guest_id() 檢查 deleted_at）
async fn delete_me(State(st): State<AppState>, GuestAuth(gid): GuestAuth) -> Result<Response, AppError> {
    let mut tx = st.pool.begin().await?;
    sqlx::query("UPDATE guests SET display_name = '已刪除的訪客', contact = NULL, email = NULL, deleted_at = now() WHERE id = $1 AND deleted_at IS NULL").bind(gid).execute(&mut *tx).await?;
    sqlx::query("UPDATE claims SET claimer_name = '已刪除的訪客' WHERE guest_id = $1").bind(gid).execute(&mut *tx).await?;
    sqlx::query("UPDATE notifications SET status = 'cancelled' WHERE guest_id = $1 AND status = 'pending'").bind(gid).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM guest_recovery_tokens WHERE guest_id = $1").bind(gid).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id) VALUES ('guest', $1, 'guest.delete', 'guests', $1)").bind(gid).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, "ws_guest=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax"), (header::CACHE_CONTROL, "private, no-store")]).into_response())
}

/// 4.12：單一語句驗證 + 標記已用 + 換發 guest token；0 rows 一律 404
async fn recover(State(st): State<AppState>, body: axum::body::Bytes) -> Result<Response, AppError> {
    let v: Value = serde_json::from_slice(&body).map_err(|_| AppError::invalid("/", "INVALID", "請求內容格式錯誤"))?;
    let tok = v.get("token").and_then(Value::as_str).filter(|t| !t.is_empty() && t.len() <= 200)
        .ok_or_else(|| AppError::invalid("/token", "REQUIRED", "token 必填"))?;
    let (new, h) = new_token();
    let row: Option<(String,)> = sqlx::query_as(
        "WITH t AS (UPDATE guest_recovery_tokens SET used_at = now() WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now() RETURNING guest_id)
         UPDATE guests g SET guest_token_hash = $2, last_seen_at = now() FROM t WHERE g.id = t.guest_id AND g.deleted_at IS NULL RETURNING g.display_name")
        .bind(hash_token(tok)).bind(h).fetch_optional(&st.pool).await?;
    let (name,) = row.ok_or(AppError::NotFound)?;
    let mut res = Json(json!({ "guest": { "display_name": name }, "guest_token": new })).into_response();
    res.headers_mut().insert(header::SET_COOKIE, cookie(&new));
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    Ok(res)
}
