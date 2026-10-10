//! 個人頁 GET /users/{handle}：依觀看者身分列出可見的清單與捐助。見 0005_social。
use crate::{access, error::AppError, AppState};
use axum::{extract::{Path, State}, http::request::Parts, routing::get, Json, Router};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new().route("/users/{handle}", get(profile))
}

async fn profile(State(st): State<AppState>, Path(handle): Path<String>, parts: Parts) -> Result<Json<Value>, AppError> {
    let viewer = access::viewer(&parts, &st).await?;
    let (uid, name): (Uuid, String) = sqlx::query_as("SELECT id, display_name FROM users WHERE handle = $1 AND deleted_at IS NULL")
        .bind(handle.to_lowercase()).fetch_optional(&st.pool).await?.ok_or(AppError::NotFound)?;
    let is_self = viewer == Some(uid);
    let friend = match viewer { Some(v) => access::are_friends(&st.pool, uid, v).await?, None => false };
    let relation = match viewer {
        None => "anonymous",
        Some(_) if is_self => "self",
        Some(_) if friend => "friend",
        Some(v) => {
            let (inc, out): (bool, bool) = sqlx::query_as(
                "SELECT EXISTS (SELECT 1 FROM friend_requests WHERE from_user = $1 AND to_user = $2),
                        EXISTS (SELECT 1 FROM friend_requests WHERE from_user = $2 AND to_user = $1)").bind(uid).bind(v).fetch_one(&st.pool).await?;
            if inc { "incoming" } else if out { "outgoing" } else { "none" }
        }
    };

    // 清單：public 一律；friends 需好友；selected 需在名單；本人另看 friends/selected（private/link/password 不列）
    let lists: Vec<(String, String, String, String, String, Option<String>, String, Option<NaiveDate>, i64, i64, i64)> = sqlx::query_as(
        "SELECT w.slug::text, w.title, w.type::text, w.status::text, w.visibility::text, w.cover_image_key, w.cover_image_status::text, w.event_date,
                (SELECT count(*) FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL),
                (SELECT COALESCE(sum(qty_claimed), 0) FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL),
                (SELECT COALESCE(sum(qty_needed), 0) FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL)
         FROM wishlists w
         WHERE w.owner_id = $1 AND w.deleted_at IS NULL AND w.moderation_status = 'ok' AND w.status IN ('active', 'closed')
           AND (w.visibility::text = 'public'
                OR (w.visibility::text IN ('friends', 'selected') AND $3)
                OR (w.visibility::text = 'friends' AND $4)
                OR (w.visibility::text = 'selected' AND EXISTS (SELECT 1 FROM wishlist_allowed_users a WHERE a.wishlist_id = w.id AND a.user_id = $2)))
         ORDER BY w.created_at DESC, w.id DESC")
        .bind(uid).bind(viewer).bind(is_self).bind(friend).fetch_all(&st.pool).await?;
    let wishlists: Vec<Value> = lists.into_iter().map(|(slug, title, ty, status, vis, ck, cs, ev, n, got, need)| json!({
        "slug": slug, "title": title, "type": ty, "status": status, "visibility": vis,
        "cover_image_url": crate::public::image_url(ck.as_deref(), &cs), "event_date": ev,
        "item_count": n, "completion_pct": crate::wishlists::completion_pct(got, need) })).collect();

    // 捐助：認領可見性（本人 / public / friends+好友）∩ 清單對觀看者可存取（public/link；friends/selected 依觀看者或擁有者），排除驚喜鎖定。
    // 與 access::claim_visible 同語意，這裡以 SQL 一次過濾（$3=本人、$4=與認領者為好友）。
    let rows: Vec<(String, i32, String, DateTime<Utc>, Option<String>, String, String, Option<String>)> = sqlx::query_as(
        "SELECT i.title, c.qty, c.status::text, c.created_at,
                CASE WHEN w.visibility::text = 'link' THEN NULL ELSE w.slug::text END, w.title, o.display_name, o.handle
         FROM claims c
         JOIN wishlist_items i ON i.id = c.item_id
         JOIN wishlists w ON w.id = i.wishlist_id
         JOIN users o ON o.id = w.owner_id
         WHERE c.user_id = $1 AND c.status IN ('reserved', 'purchased', 'delivered')
           AND (c.visibility::text = 'public' OR $3 OR (c.visibility::text = 'friends' AND $4))
           AND w.deleted_at IS NULL AND w.moderation_status = 'ok' AND w.status IN ('active', 'closed')
           AND NOT (w.surprise_mode AND (w.event_date IS NULL OR now() < (w.event_date::timestamp AT TIME ZONE 'Asia/Taipei')))
           AND (w.visibility::text IN ('public', 'link')
                OR w.owner_id = $2
                OR (w.visibility::text = 'friends' AND $2::uuid IS NOT NULL
                    AND EXISTS (SELECT 1 FROM friendships f WHERE f.user_a = LEAST($2::uuid, w.owner_id) AND f.user_b = GREATEST($2::uuid, w.owner_id)))
                OR (w.visibility::text = 'selected' AND EXISTS (SELECT 1 FROM wishlist_allowed_users a WHERE a.wishlist_id = w.id AND a.user_id = $2)))
         ORDER BY c.created_at DESC, c.id DESC LIMIT 50")
        .bind(uid).bind(viewer).bind(is_self).bind(friend).fetch_all(&st.pool).await?;
    let donations: Vec<Value> = rows.into_iter().map(|(it, qty, status, at, slug, wt, on, oh)| json!({
        "item_title": it, "qty": qty, "status": status, "created_at": at,
        "wishlist": { "slug": slug, "title": wt, "owner": { "display_name": on, "handle": oh } } })).collect();

    Ok(Json(json!({ "user": { "id": uid, "handle": handle.to_lowercase(), "display_name": name }, "relation": relation,
                    "wishlists": wishlists, "donations": donations })))
}
