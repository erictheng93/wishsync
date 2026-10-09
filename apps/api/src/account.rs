//! 帳號刪除 / 去識別化（docs/04 4.9，D40）。P2 錢包前置檢查不在 MVP。
use crate::{error::AppError, session::CurrentUser, AppState};
use axum::{http::{header, HeaderValue, StatusCode}, extract::{Query, State}, response::{IntoResponse, Response}, routing::{delete, get}, Json, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new().route("/me", delete(delete_me)).route("/me/export", get(export)).route("/unsubscribe", get(unsubscribe))
}

// ---------- 一鍵退訂（4.13）：token = base64url("u|g:<id>:<exp>") . hex(HMAC-SHA256) ----------
fn mac(msg: &str) -> Hmac<Sha256> {
    let key = std::env::var("UNSUB_SECRET").or_else(|_| std::env::var("OAUTH_SECRET")).unwrap_or_else(|_| "dev-oauth-secret".into());
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key.as_bytes()).unwrap();
    m.update(msg.as_bytes());
    m
}

/// kind: 'u'（建立者）| 'g'（訪客）；有效一年
pub fn unsub_token(kind: char, id: Uuid) -> String {
    let msg = format!("{kind}:{id}:{}", chrono::Utc::now().timestamp() + 365 * 86400);
    format!("{}.{}", B64.encode(&msg), hex::encode(mac(&msg).finalize().into_bytes()))
}

fn verify(token: &str) -> Option<(char, Uuid)> {
    let (m, sig) = token.split_once('.')?;
    let msg = String::from_utf8(B64.decode(m).ok()?).ok()?;
    mac(&msg).verify_slice(&hex::decode(sig).ok()?).ok()?;
    let mut it = msg.split(':');
    let (k, id, exp) = (it.next()?.chars().next()?, it.next()?.parse().ok()?, it.next()?.parse::<i64>().ok()?);
    (exp > chrono::Utc::now().timestamp() && matches!(k, 'u' | 'g')).then_some((k, id))
}

#[derive(Deserialize)]
struct UnsubQ { token: Option<String> }

/// 簽章無效 / 過期一律 302 ?ok=0；冪等
async fn unsubscribe(State(st): State<AppState>, Query(q): Query<UnsubQ>) -> Result<Response, AppError> {
    let ok = match q.token.as_deref().and_then(verify) {
        Some((k, id)) => {
            let mut tx = st.pool.begin().await?;
            if k == 'u' {
                sqlx::query("UPDATE users SET notification_prefs = jsonb_set(COALESCE(notification_prefs, '{}'::jsonb), '{email_claims}', 'false'::jsonb) WHERE id = $1 AND deleted_at IS NULL").bind(id).execute(&mut *tx).await?;
                sqlx::query("UPDATE notifications SET status = 'cancelled' WHERE user_id = $1 AND status = 'pending' AND kind IN ('claim.created', 'claim.digest')").bind(id).execute(&mut *tx).await?;
            } else {
                sqlx::query("UPDATE guests SET email = NULL WHERE id = $1").bind(id).execute(&mut *tx).await?;
                sqlx::query("UPDATE notifications SET status = 'cancelled' WHERE guest_id = $1 AND status = 'pending'").bind(id).execute(&mut *tx).await?;
            }
            tx.commit().await?;
            1
        }
        None => 0,
    };
    let base = std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:3000".into());
    let loc = format!("{}/unsubscribed?ok={ok}", base.trim_end_matches('/'));
    Ok((StatusCode::FOUND, [(header::LOCATION, loc)]).into_response())
}

// ---------- GET /me/export ----------
async fn export(State(st): State<AppState>, u: CurrentUser) -> Result<Response, AppError> {
    // 每帳號每日 3 次（以 audit_logs 計）
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE actor_type = 'user' AND actor_id = $1 AND action = 'account.export' AND created_at > now() - interval '1 day'")
        .bind(u.id).fetch_one(&st.pool).await?;
    if n >= 3 {
        return Err(AppError::Problem { status: 429, code: "RATE_LIMITED", detail: "匯出每日最多 3 次".into(), errors: None, retry_after: Some(3600) });
    }
    let user: Value = sqlx::query_scalar("SELECT jsonb_build_object('id', id, 'display_name', display_name, 'email', email, 'created_at', created_at) FROM users WHERE id = $1")
        .bind(u.id).fetch_one(&st.pool).await?;
    let identities: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('provider', provider::text) FROM auth_identities WHERE user_id = $1 ORDER BY created_at")
        .bind(u.id).fetch_all(&st.pool).await?;
    // 驚喜鎖定期間不含品項認領數；只匯出本人的認領（不含他人資料）
    let wishlists: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', w.id, 'slug', w.slug::text, 'title', w.title, 'type', w.type::text, 'status', w.status::text,
           'event_date', w.event_date, 'surprise_mode', w.surprise_mode, 'created_at', w.created_at,
           'items', (SELECT COALESCE(jsonb_agg(jsonb_build_object('id', i.id, 'title', i.title, 'qty_needed', i.qty_needed,
               'qty_claimed', CASE WHEN w.surprise_mode AND w.event_date IS NOT NULL AND now() < (w.event_date::timestamp AT TIME ZONE 'Asia/Taipei') THEN NULL ELSE i.qty_claimed END)
               ORDER BY i.sort_order, i.id), '[]'::jsonb) FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL))
         FROM wishlists w WHERE w.owner_id = $1 AND w.deleted_at IS NULL ORDER BY w.id").bind(u.id).fetch_all(&st.pool).await?;
    let claims: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'item_id', item_id, 'qty', qty, 'status', status::text, 'note', note, 'created_at', created_at)
           FROM claims WHERE user_id = $1 ORDER BY id").bind(u.id).fetch_all(&st.pool).await?;
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id) VALUES ('user', $1, 'account.export', 'users', $1)").bind(u.id).execute(&st.pool).await?;
    let now = chrono::Utc::now();
    let mut res = Json(json!({ "exported_at": now, "user": user, "identities": identities, "wishlists": wishlists, "claims": claims })).into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&format!("attachment; filename=\"wishsync-export-{}.json\"", now.format("%Y-%m-%d"))).unwrap());
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    Ok(res)
}

async fn delete_me(State(st): State<AppState>, u: CurrentUser, Json(b): Json<Value>) -> Result<Response, AppError> {
    if b.get("confirm").and_then(Value::as_str) != Some("DELETE") {
        return Err(AppError::invalid("/confirm", "INVALID", "confirm 必須為 \"DELETE\""));
    }
    let id = u.id;
    let mut tx = st.pool.begin().await?;
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL").bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM otp_challenges WHERE email = (SELECT email FROM users WHERE id = $1)").bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM auth_identities WHERE user_id = $1").bind(id).execute(&mut *tx).await?;
    sqlx::query("UPDATE wishlists SET status = 'archived', visibility = 'private', closed_at = COALESCE(closed_at, now()) WHERE owner_id = $1 AND status <> 'archived'")
        .bind(id).execute(&mut *tx).await?;
    sqlx::query("UPDATE claims SET claimer_name = '已刪除的使用者' WHERE user_id = $1").bind(id).execute(&mut *tx).await?;
    sqlx::query("UPDATE notifications SET status = 'cancelled' WHERE user_id = $1 AND status = 'pending'").bind(id).execute(&mut *tx).await?;
    let (at,): (chrono::DateTime<chrono::Utc>,) = sqlx::query_as(
        "UPDATE users SET display_name = '已刪除的使用者', email = NULL, avatar_key = NULL, notification_prefs = '{}'::jsonb,
                deleted_at = now(), anonymized_at = now() WHERE id = $1 AND deleted_at IS NULL RETURNING anonymized_at")
        .bind(id).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id) VALUES ('user', $1, 'account.delete', 'users', $1)")
        .bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    let mut res = Json(json!({ "deleted": true, "anonymized_at": at })).into_response();
    res.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_static("ws_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax"));
    Ok(res)
}
