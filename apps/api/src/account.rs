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
    Router::new().route("/me", delete(delete_me)).route("/me/export", get(export)).route("/unsubscribe", get(unsubscribe_redirect).post(unsubscribe))
}

// ---------- 一鍵退訂（4.13）：token = base64url("u|g:<id>:<exp>") . hex(HMAC-SHA256) ----------
fn mac(msg: &str) -> Hmac<Sha256> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(crate::config::get().unsub_secret.as_bytes()).unwrap();
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

/// GET 不變更狀態（信件掃描器預取無害）：只 302 到前端確認頁，由頁面 POST 才退訂。token 驗證留給 POST。
async fn unsubscribe_redirect(Query(q): Query<UnsubQ>) -> Response {
    // token 只含 base64url / '.' / hex；含其他字元者視為無效，不帶入 URL（避免注入）
    let t = q.token.filter(|t| t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))).unwrap_or_default();
    let loc = format!("{}/unsubscribe?token={t}", crate::config::get().app_url);
    (StatusCode::FOUND, [(header::LOCATION, loc)]).into_response()
}

#[derive(Deserialize)]
struct UnsubBody { token: Option<String> }

/// POST {token}：HMAC 驗證（verify_slice 常數時間）；成功 204、冪等；無效 / 過期 422 INVALID_TOKEN
async fn unsubscribe(State(st): State<AppState>, Json(b): Json<UnsubBody>) -> Result<StatusCode, AppError> {
    let Some((k, id)) = b.token.as_deref().and_then(verify) else {
        return Err(AppError::problem(422, "INVALID_TOKEN", "退訂連結無效或已過期"));
    };
    let mut tx = st.pool.begin().await?;
    if k == 'u' {
        sqlx::query("UPDATE users SET notification_prefs = jsonb_set(COALESCE(notification_prefs, '{}'::jsonb), '{email_claims}', 'false'::jsonb) WHERE id = $1 AND deleted_at IS NULL").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE notifications SET status = 'cancelled' WHERE user_id = $1 AND status = 'pending' AND kind IN ('claim.created', 'claim.digest')").bind(id).execute(&mut *tx).await?;
    } else {
        sqlx::query("UPDATE guests SET email = NULL WHERE id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE notifications SET status = 'cancelled' WHERE guest_id = $1 AND status = 'pending'").bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
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
    // P2-A：錢包、點數流水與本人的認捐（不含他人資料）
    let wallet: Option<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id', id, 'balance', balance, 'status', status::text, 'created_at', created_at) FROM point_wallets WHERE user_id = $1")
        .bind(u.id).fetch_optional(&st.pool).await?;
    let ledger: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', l.id, 'delta', l.delta, 'balance_after', l.balance_after, 'entry_type', l.entry_type::text, 'ref_type', l.ref_type,
                'ref_id', l.ref_id, 'note', l.note, 'created_at', l.created_at)
           FROM point_ledger l JOIN point_wallets w ON w.id = l.wallet_id WHERE w.user_id = $1 ORDER BY l.seq").bind(u.id).fetch_all(&st.pool).await?;
    let contributions: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'item_id', item_id, 'wishlist_id', wishlist_id, 'points', points, 'refunded_points', refunded_points, 'status', status::text,
                'message', message, 'is_anonymous', is_anonymous, 'captured_at', captured_at, 'released_at', released_at, 'created_at', created_at)
           FROM contributions WHERE user_id = $1 ORDER BY id").bind(u.id).fetch_all(&st.pool).await?;
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id) VALUES ('user', $1, 'account.export', 'users', $1)").bind(u.id).execute(&st.pool).await?;
    let now = chrono::Utc::now();
    let mut res = Json(json!({ "exported_at": now, "user": user, "identities": identities, "wishlists": wishlists, "claims": claims,
        "wallet": wallet, "ledger": ledger, "contributions": contributions })).into_response();
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
    // P2-A：還有進行中的認捐（pledged）或點數餘額就不能刪帳號（否則點數會憑空消失）。先鎖錢包，與認捐 / 營運發點互斥。
    // captured（已達標、採購中）的認捐不擋：點數已花出，帳號匿名化後 donor_name 一併去識別。
    sqlx::query("SELECT id FROM point_wallets WHERE user_id = $1 FOR UPDATE").bind(id).execute(&mut *tx).await?;
    let held: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM contributions WHERE user_id = $1 AND status = 'pledged'), (SELECT coalesce(sum(balance), 0)::bigint FROM point_wallets WHERE user_id = $1)")
        .bind(id).fetch_one(&mut *tx).await?;
    if held.0 > 0 || held.1 > 0 {
        return Err(AppError::problem(409, "ACCOUNT_HAS_POINTS", format!("帳號還有 {} 筆進行中的認捐、{} 點餘額。請先到錢包撤回認捐，並聯絡客服處理剩餘點數後再刪除帳號。", held.0, held.1)));
    }
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL").bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM otp_challenges WHERE email = (SELECT email FROM users WHERE id = $1)").bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM auth_identities WHERE user_id = $1").bind(id).execute(&mut *tx).await?;
    let archived: Vec<Uuid> = sqlx::query_scalar("UPDATE wishlists SET status = 'archived', visibility = 'private', closed_at = COALESCE(closed_at, now()) WHERE owner_id = $1 AND status <> 'archived' RETURNING id")
        .bind(id).fetch_all(&mut *tx).await?;
    crate::wishlists::release_pledged(&mut tx, &archived).await?; // 別人捐在這些清單上、尚未達標的點數退回各自錢包
    // F-08：取消他人清單上的 reserved 認領並回補（purchased / delivered 已履行，保留）。鎖序同 claims::apply：先 item（依 id）後 claim。
    sqlx::query("SELECT id FROM wishlist_items WHERE id IN (SELECT item_id FROM claims WHERE user_id = $1 AND status = 'reserved') ORDER BY id FOR UPDATE")
        .bind(id).execute(&mut *tx).await?;
    let wids: Vec<Uuid> = sqlx::query_scalar(
        "WITH x AS (UPDATE claims SET status = 'cancelled', cancelled_at = now(), expires_at = NULL
                     WHERE user_id = $1 AND status = 'reserved' RETURNING id, item_id, qty),
         r AS (UPDATE wishlist_items i SET qty_claimed = i.qty_claimed - s.q
                FROM (SELECT item_id, sum(qty)::int AS q FROM x GROUP BY item_id) s
                WHERE i.id = s.item_id RETURNING i.wishlist_id),
         a AS (INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff)
                SELECT 'user'::actor_type, $1, 'claim.account_delete_cancel', 'claims', id, jsonb_build_object('qty', qty) FROM x)
         SELECT DISTINCT wishlist_id FROM r").bind(id).fetch_all(&mut *tx).await?;
    for w in wids { crate::dashboard::notify(&mut *tx, w).await?; }
    sqlx::query("UPDATE claims SET claimer_name = '已刪除的使用者' WHERE user_id = $1").bind(id).execute(&mut *tx).await?;
    sqlx::query("UPDATE contributions SET donor_name = '已刪除的使用者' WHERE user_id = $1").bind(id).execute(&mut *tx).await?;
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
