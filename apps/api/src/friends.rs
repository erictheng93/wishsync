//! 好友：邀請連結（/invite/{token}）、申請＋同意、列表、解除。見 0005_social。
use crate::{access::are_friends, error::AppError, guest::new_token, ratelimit, session::{hash_token, CurrentUser}, AppState};
use axum::{extract::{Path, State}, http::StatusCode, routing::{delete, get, post}, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

type R<T> = Result<T, AppError>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/friends", get(list))
        .route("/friends/requests", post(request))
        .route("/friends/requests/{id}/accept", post(accept))
        .route("/friends/requests/{id}", delete(drop_request))
        .route("/friends/invites", post(invite_create).delete(invite_revoke))
        .route("/friends/invites/{token}", get(invite_preview))
        .route("/friends/invites/{token}/accept", post(invite_accept))
        .route("/friends/{user_id}", delete(unfriend))
}

fn pair(a: Uuid, b: Uuid) -> (Uuid, Uuid) { if a < b { (a, b) } else { (b, a) } }

/// 成為好友（冪等）；同時清掉雙方之間的待處理申請
async fn befriend(ex: &mut sqlx::PgConnection, a: Uuid, b: Uuid) -> R<()> {
    let (x, y) = pair(a, b);
    sqlx::query("INSERT INTO friendships (user_a, user_b) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(x).bind(y).execute(&mut *ex).await?;
    sqlx::query("DELETE FROM friend_requests WHERE (from_user = $1 AND to_user = $2) OR (from_user = $2 AND to_user = $1)").bind(a).bind(b).execute(&mut *ex).await?;
    Ok(())
}

async fn friend_json(pool: &PgPool, me: Uuid, other: Uuid) -> R<Value> {
    let (x, y) = pair(me, other);
    let (name, handle, since): (String, Option<String>, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "SELECT u.display_name, u.handle, f.created_at FROM friendships f JOIN users u ON u.id = $3
         WHERE f.user_a = $1 AND f.user_b = $2").bind(x).bind(y).bind(other).fetch_one(pool).await?;
    Ok(json!({ "id": other, "display_name": name, "handle": handle, "since": since }))
}

async fn list(u: CurrentUser, State(st): State<AppState>) -> R<Json<Value>> {
    let friends: Vec<(Uuid, String, Option<String>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT o.id, o.display_name, o.handle, f.created_at FROM friendships f
         JOIN users o ON o.id = CASE WHEN f.user_a = $1 THEN f.user_b ELSE f.user_a END AND o.deleted_at IS NULL
         WHERE f.user_a = $1 OR f.user_b = $1 ORDER BY o.display_name, o.id").bind(u.id).fetch_all(&st.pool).await?;
    let reqs = |incoming: bool| {
        let sql = format!("SELECT r.id, o.id, o.display_name, o.handle, r.created_at FROM friend_requests r
            JOIN users o ON o.id = r.{} AND o.deleted_at IS NULL WHERE r.{} = $1 ORDER BY r.created_at DESC",
            if incoming { "from_user" } else { "to_user" }, if incoming { "to_user" } else { "from_user" });
        let pool = st.pool.clone();
        async move {
            let rows: Vec<(Uuid, Uuid, String, Option<String>, chrono::DateTime<chrono::Utc>)> =
                sqlx::query_as(&sql).bind(u.id).fetch_all(&pool).await?;
            Ok::<_, AppError>(rows.into_iter().map(|(id, uid, n, h, at)| json!({ "id": id, "user": { "id": uid, "display_name": n, "handle": h }, "created_at": at })).collect::<Vec<_>>())
        }
    };
    Ok(Json(json!({
        "friends": friends.into_iter().map(|(id, n, h, s)| json!({ "id": id, "display_name": n, "handle": h, "since": s })).collect::<Vec<_>>(),
        "incoming": reqs(true).await?, "outgoing": reqs(false).await? })))
}

#[derive(Deserialize)]
struct ReqBody { to: String }

/// 一律 202，不洩漏帳號是否存在
async fn request(u: CurrentUser, State(st): State<AppState>, Json(b): Json<ReqBody>) -> R<(StatusCode, Json<Value>)> {
    ratelimit::check(&st.pool, &format!("friend_req:{}", u.id), 30, 3600).await?;
    let to = b.to.trim().to_lowercase();
    if to.is_empty() || to.chars().count() > 200 { return Err(AppError::invalid("/to", "FORMAT", "請輸入 Email 或帳號代號")); }
    let target: Option<Uuid> = if to.contains('@') && !to.starts_with('@') {
        sqlx::query_scalar("SELECT id FROM users WHERE email = $1 AND deleted_at IS NULL").bind(&to).fetch_optional(&st.pool).await?
    } else {
        sqlx::query_scalar("SELECT id FROM users WHERE handle = $1 AND deleted_at IS NULL").bind(to.trim_start_matches('@')).fetch_optional(&st.pool).await?
    };
    if let Some(t) = target.filter(|t| *t != u.id) {
        if !are_friends(&st.pool, u.id, t).await? {
            let mut tx = st.pool.begin().await?;
            let reverse: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM friend_requests WHERE from_user = $1 AND to_user = $2)").bind(t).bind(u.id).fetch_one(&mut *tx).await?;
            if reverse { befriend(&mut *tx, u.id, t).await?; }
            else { sqlx::query("INSERT INTO friend_requests (from_user, to_user) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(u.id).bind(t).execute(&mut *tx).await?; }
            tx.commit().await?;
        }
    }
    Ok((StatusCode::ACCEPTED, Json(json!({}))))
}

async fn accept(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>) -> R<Json<Value>> {
    let mut tx = st.pool.begin().await?;
    let from: Uuid = sqlx::query_scalar("SELECT from_user FROM friend_requests WHERE id = $1 AND to_user = $2 FOR UPDATE")
        .bind(id).bind(u.id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    befriend(&mut *tx, u.id, from).await?;
    tx.commit().await?;
    Ok(Json(json!({ "friend": friend_json(&st.pool, u.id, from).await? })))
}

async fn drop_request(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>) -> R<StatusCode> {
    let n = sqlx::query("DELETE FROM friend_requests WHERE id = $1 AND (to_user = $2 OR from_user = $2)").bind(id).bind(u.id).execute(&st.pool).await?.rows_affected();
    if n == 0 { return Err(AppError::NotFound); }
    Ok(StatusCode::NO_CONTENT)
}

async fn unfriend(u: CurrentUser, State(st): State<AppState>, Path(other): Path<Uuid>) -> R<StatusCode> {
    let (x, y) = pair(u.id, other);
    let mut tx = st.pool.begin().await?;
    sqlx::query("DELETE FROM friendships WHERE user_a = $1 AND user_b = $2").bind(x).bind(y).execute(&mut *tx).await?;
    // 雙方清單的指定名單中，彼此都要移除
    sqlx::query("DELETE FROM wishlist_allowed_users a USING wishlists w
                 WHERE a.wishlist_id = w.id AND ((w.owner_id = $1 AND a.user_id = $2) OR (w.owner_id = $2 AND a.user_id = $1))")
        .bind(u.id).bind(other).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- 邀請連結 ----------

async fn invite_create(u: CurrentUser, State(st): State<AppState>) -> R<(StatusCode, Json<Value>)> {
    let (token, h) = new_token();
    let mut tx = st.pool.begin().await?;
    sqlx::query("UPDATE friend_invites SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL").bind(u.id).execute(&mut *tx).await?;
    let exp: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("INSERT INTO friend_invites (user_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '7 days') RETURNING expires_at")
        .bind(u.id).bind(h).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "token": token, "url": format!("{}/invite/{token}", crate::auth::app_url()), "expires_at": exp }))))
}

async fn invite_revoke(u: CurrentUser, State(st): State<AppState>) -> R<StatusCode> {
    sqlx::query("UPDATE friend_invites SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL").bind(u.id).execute(&st.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 有效邀請 → (邀請者 id, 暱稱, handle)
async fn find_invite(pool: &PgPool, token: &str) -> R<(Uuid, String, Option<String>)> {
    sqlx::query_as("SELECT u.id, u.display_name, u.handle FROM friend_invites i JOIN users u ON u.id = i.user_id AND u.deleted_at IS NULL
                    WHERE i.token_hash = $1 AND i.revoked_at IS NULL AND i.expires_at > now()")
        .bind(hash_token(token)).fetch_optional(pool).await?.ok_or(AppError::NotFound)
}

async fn invite_preview(State(st): State<AppState>, Path(token): Path<String>) -> R<Json<Value>> {
    let (_, n, h) = find_invite(&st.pool, &token).await?;
    Ok(Json(json!({ "inviter": { "display_name": n, "handle": h } })))
}

async fn invite_accept(u: CurrentUser, State(st): State<AppState>, Path(token): Path<String>) -> R<Json<Value>> {
    let (inviter, ..) = find_invite(&st.pool, &token).await?;
    if inviter == u.id { return Err(AppError::problem(422, "SELF_INVITE", "不能接受自己的邀請")); }
    let mut tx = st.pool.begin().await?;
    befriend(&mut *tx, u.id, inviter).await?;
    tx.commit().await?;
    Ok(Json(json!({ "friend": friend_json(&st.pool, u.id, inviter).await? })))
}
