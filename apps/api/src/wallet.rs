//! 捐贈者錢包：GET /wallet（餘額 + ledger 分頁）、GET /wallet/contributions（我的認捐）。
use crate::{contributions::C_JSON, error::AppError, points::DISPLAY_STATUS_SQL, session::CurrentUser, AppState};
use axum::{
    extract::{Query, State},
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new().route("/wallet", get(wallet)).route("/wallet/contributions", get(contributions))
}

#[derive(Deserialize)]
struct Page { cursor: Option<String>, limit: Option<i64>, status: Option<String> }

fn limit(l: Option<i64>) -> i64 { l.unwrap_or(20).clamp(1, 100) }

fn no_store(body: Value) -> Response {
    let mut r = Json(body).into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    r
}

async fn wallet(State(st): State<AppState>, user: CurrentUser, Query(q): Query<Page>) -> Result<Response, AppError> {
    let cursor: Option<i64> = match q.cursor.as_deref() {
        Some(c) => Some(c.parse().map_err(|_| AppError::invalid("/cursor", "INVALID", "cursor 格式錯誤"))?),
        None => None,
    };
    let lim = limit(q.limit);
    let w: Option<(Uuid, i64, String)> = sqlx::query_as("SELECT id, balance, status::text FROM point_wallets WHERE user_id = $1").bind(user.id).fetch_optional(&st.pool).await?;
    let held: i64 = sqlx::query_scalar("SELECT COALESCE(sum(points - refunded_points), 0)::bigint FROM contributions WHERE user_id = $1 AND status IN ('pledged', 'captured')")
        .bind(user.id).fetch_one(&st.pool).await?;
    let (mut data, mut next) = (vec![], None);
    if let Some((wid, ..)) = &w {
        let mut rows: Vec<(i64, Value)> = sqlx::query_as(
            "SELECT seq, jsonb_build_object('id', id, 'delta', delta, 'balance_after', balance_after, 'entry_type', entry_type,
                      'ref_type', ref_type, 'ref_id', ref_id, 'note', note, 'created_at', created_at)
               FROM point_ledger WHERE wallet_id = $1 AND ($2::bigint IS NULL OR seq < $2) ORDER BY seq DESC LIMIT $3")
            .bind(wid).bind(cursor).bind(lim + 1).fetch_all(&st.pool).await?;
        if rows.len() as i64 > lim { rows.truncate(lim as usize); next = rows.last().map(|r| r.0.to_string()); }
        data = rows.into_iter().map(|r| r.1).collect();
    }
    let (id, balance, status) = w.map_or((None, 0, "active".to_string()), |(i, b, s)| (Some(i), b, s));
    Ok(no_store(json!({ "wallet": { "id": id, "balance": balance, "held_points": held, "status": status }, "data": data, "next_cursor": next })))
}

async fn contributions(State(st): State<AppState>, user: CurrentUser, Query(q): Query<Page>) -> Result<Response, AppError> {
    if q.status.as_deref().is_some_and(|s| !["pledged", "captured", "released", "reallocated"].contains(&s)) {
        return Err(AppError::invalid("/status", "ENUM", "status 只能是 pledged / captured / released / reallocated"));
    }
    let cursor: Option<Uuid> = match q.cursor.as_deref() {
        Some(c) => Some(c.parse().map_err(|_| AppError::invalid("/cursor", "INVALID", "cursor 格式錯誤"))?),
        None => None,
    };
    let lim = limit(q.limit);
    let window = "(i.funding_status = 'expired' AND now() < i.expired_at + interval '7 days')";
    let mut rows: Vec<(Uuid, Value)> = sqlx::query_as(&format!(
        "SELECT c.id, jsonb_build_object(
           'contribution', {C_JSON},
           'item', jsonb_build_object('id', i.id, 'title', i.title, 'image_key', i.image_key, 'image_status', i.image_status,
              'funding_status', i.funding_status, 'display_status', {DISPLAY_STATUS_SQL}, 'funding_deadline', i.funding_deadline,
              'reallocation_deadline', CASE WHEN i.funding_status = 'expired' THEN i.expired_at + interval '7 days' END,
              'target_points', i.target_points, 'pledged_points', i.pledged_points),
           'wishlist', jsonb_build_object('slug', w.slug, 'title', w.title, 'event_date', w.event_date),
           'can_withdraw', c.status = 'pledged' AND (i.funding_status = 'open' OR {window}),
           'can_reallocate', c.status = 'pledged' AND {window})
         FROM contributions c JOIN wishlist_items i ON i.id = c.item_id JOIN wishlists w ON w.id = c.wishlist_id
         LEFT JOIN purchase_orders po ON po.item_id = i.id
         WHERE c.user_id = $1 AND ($2::text IS NULL OR c.status::text = $2) AND ($3::uuid IS NULL OR c.id < $3)
         ORDER BY c.id DESC LIMIT $4"))
        .bind(user.id).bind(&q.status).bind(cursor).bind(lim + 1).fetch_all(&st.pool).await?;
    let mut next = None;
    if rows.len() as i64 > lim { rows.truncate(lim as usize); next = rows.last().map(|r| r.0.to_string()); }
    let data: Vec<Value> = rows.into_iter().map(|(_, mut v)| {
        let it = &mut v["item"];
        let url = crate::public::image_url(it["image_key"].as_str(), it["image_status"].as_str().unwrap_or("none"));
        if let Some(o) = it.as_object_mut() { o.remove("image_key"); o.remove("image_status"); }
        it["image_url"] = json!(url);
        v
    }).collect();
    Ok(no_store(json!({ "data": data, "next_cursor": next })))
}
