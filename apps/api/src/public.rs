//! GET /public/wishlists/{slug}（契約第 5 章；訪客視角，不讀 cookie，可被 CDN 快取）
use crate::{error::AppError, AppState};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{json, Value};
use sqlx::FromRow;
use std::collections::HashMap;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new().route("/public/wishlists/{slug}", get(wishlist))
}

pub fn image_url(key: Option<&str>, status: &str) -> Option<String> {
    key.filter(|_| status == "ready").map(|k| crate::uploads::S3::get().public_url(k))
}

#[derive(FromRow)]
struct W {
    id: Uuid, slug: String, r#type: String, status: String, visibility: String, moderation: String,
    title: String, description: Option<String>, cover_image_key: Option<String>, cover_image_status: String,
    event_date: Option<NaiveDate>, show_claimer_names: bool, surprise_mode: bool,
    owner_id: Uuid, owner_name: String, locked: bool, updated_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct It {
    id: Uuid, title: String, brand: Option<String>, spec: Option<String>, image_key: Option<String>, image_status: String,
    product_url: Option<String>, unit_price_amount: Option<i64>, priority: String, funding_mode: String,
    qty_needed: i32, qty_claimed: i32,
    target_points: Option<i64>, pledged_points: i64, funding_status: Option<String>, display_status: Option<String>, funding_deadline: Option<DateTime<Utc>>,
}

async fn wishlist(State(st): State<AppState>, Path(slug): Path<String>, req: HeaderMap) -> Result<Response, AppError> {
    if !crate::validate::slug_ok(&slug) { return Err(AppError::NotFound); }
    let w: W = sqlx::query_as(
        "SELECT w.id, w.slug::text AS slug, w.type::text AS type, w.status::text AS status, w.visibility::text AS visibility,
                w.moderation_status::text AS moderation, w.title, w.description, w.cover_image_key,
                w.cover_image_status::text AS cover_image_status, w.event_date, w.show_claimer_names, w.surprise_mode,
                w.owner_id, u.display_name AS owner_name,
                (w.surprise_mode AND (w.event_date IS NULL OR now() < (w.event_date::timestamp AT TIME ZONE 'Asia/Taipei'))) AS locked,
                GREATEST(w.updated_at, COALESCE((SELECT max(updated_at) FROM wishlist_items WHERE wishlist_id = w.id), w.updated_at)) AS updated_at
         FROM wishlists w JOIN users u ON u.id = w.owner_id
         WHERE w.slug = $1 AND w.deleted_at IS NULL")
        .bind(&slug).fetch_optional(&st.pool).await?.ok_or(AppError::NotFound)?;
    if w.status == "draft" || w.status == "archived" || w.visibility == "private" { return Err(AppError::NotFound); }
    if w.moderation == "hidden" { return Err(AppError::WishlistRemoved); }

    let cache = [(header::CACHE_CONTROL, "public, s-maxage=10, stale-while-revalidate=60".to_string())];
    let etag = format!("\"w-{}-{}{}\"", w.slug, w.updated_at.timestamp_millis(), if w.locked { "-l" } else { "" });
    if req.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return Ok((StatusCode::NOT_MODIFIED, cache, [(header::ETAG, etag)]).into_response());
    }

    let items: Vec<It> = sqlx::query_as(
        &format!("SELECT i.id, i.title, i.brand, i.spec, i.image_key, i.image_status::text AS image_status, i.product_url, i.unit_price_amount,
                i.priority::text AS priority, i.funding_mode::text AS funding_mode, i.qty_needed, i.qty_claimed,
                i.target_points, i.pledged_points, i.funding_status::text AS funding_status, ({}) AS display_status, i.funding_deadline
         FROM wishlist_items i LEFT JOIN purchase_orders po ON po.item_id = i.id
         WHERE i.wishlist_id = $1 AND i.deleted_at IS NULL ORDER BY i.sort_order, i.id", crate::points::DISPLAY_STATUS_SQL))
        .bind(w.id).fetch_all(&st.pool).await?;

    // 驚喜鎖定期間無論 show_claimer_names 為何一律不輸出（伺服器端強制）
    let visible = w.show_claimer_names && !w.locked;
    let mut claimers: HashMap<Uuid, Vec<Value>> = HashMap::new();
    if visible {
        let rows: Vec<(Uuid, String, i64)> = sqlx::query_as(
            "SELECT c.item_id, c.claimer_name, sum(c.qty)::bigint FROM claims c JOIN wishlist_items i ON i.id = c.item_id
             WHERE i.wishlist_id = $1 AND i.deleted_at IS NULL AND c.status IN ('reserved','purchased','delivered')
             GROUP BY c.item_id, c.claimer_name ORDER BY min(c.created_at)")
            .bind(w.id).fetch_all(&st.pool).await?;
        for (item, name, qty) in rows { claimers.entry(item).or_default().push(json!({ "display_name": name, "qty": qty })); }
    }

    // 眾籌捐贈者：同一位（同 user 且非匿名）合併成一筆；匿名者各自一筆、一律「匿名朋友」。只算 pledged / captured。
    // 與 claimers 同一道閘：show_claimer_names 且未驚喜鎖定才輸出。
    let mut contributors: HashMap<Uuid, Vec<Value>> = HashMap::new();
    if visible {
        let rows: Vec<(Uuid, Uuid, bool, String, i64)> = sqlx::query_as(
            "SELECT c.item_id, c.user_id, c.is_anonymous, c.donor_name, c.points FROM contributions c JOIN wishlist_items i ON i.id = c.item_id
             WHERE i.wishlist_id = $1 AND i.deleted_at IS NULL AND c.status IN ('pledged', 'captured') ORDER BY c.created_at, c.id")
            .bind(w.id).fetch_all(&st.pool).await?;
        let mut merged: HashMap<(Uuid, Uuid), usize> = HashMap::new(); // (item, user) -> 該品項 contributors 的索引（僅非匿名）
        for (item, user, anon, name, points) in rows {
            let list = contributors.entry(item).or_default();
            if anon { list.push(json!({ "display_name": "匿名朋友", "points": points })); continue; }
            match merged.get(&(item, user)) {
                Some(&k) => list[k]["points"] = json!(list[k]["points"].as_i64().unwrap_or(0) + points),
                None => { merged.insert((item, user), list.len()); list.push(json!({ "display_name": name, "points": points })); }
            }
        }
    }

    // 完成度：數量型 = qty_claimed/qty_needed；眾籌型達標（funded / fulfilled）算 1/1（見 wishlists::units）
    let (mut total, mut got, mut fulfilled) = (0i64, 0i64, 0usize);
    for i in &items {
        let (c, n) = crate::wishlists::units(&i.funding_mode, i.qty_needed, i.qty_claimed, i.funding_status.as_deref());
        total += n; got += c;
        if c >= n { fulfilled += 1; }
    }
    let pct = crate::wishlists::completion_pct;
    let item_json: Vec<Value> = items.iter().map(|i| {
        let cf = i.funding_mode == "crowdfund";
        let funded = matches!(i.funding_status.as_deref(), Some("funded" | "fulfilled"));
        let mut v = json!({
            "id": i.id, "title": i.title, "brand": i.brand, "spec": i.spec,
            "image_url": image_url(i.image_key.as_deref(), &i.image_status),
            "product_url": i.product_url, "unit_price_amount": i.unit_price_amount, "priority": i.priority,
            "funding_mode": i.funding_mode, "qty_needed": i.qty_needed, "qty_claimed": i.qty_claimed,
            "qty_remaining": i.qty_needed - i.qty_claimed, "is_fully_claimed": if cf { funded } else { i.qty_claimed >= i.qty_needed },
            "target_points": i.target_points, "pledged_points": cf.then_some(i.pledged_points),
            "remaining_points": i.target_points.filter(|_| cf).map(|t| (t - i.pledged_points).max(0)),
            "funding_status": i.funding_status, "display_status": i.display_status, "funding_deadline": i.funding_deadline,
            "progress_percent": if cf { pct(i.pledged_points, i.target_points.unwrap_or(0)) } else { pct(i.qty_claimed as i64, i.qty_needed as i64) },
        });
        if visible {
            v["claimers"] = json!(claimers.remove(&i.id).unwrap_or_default());
            if cf { v["contributors"] = json!(contributors.remove(&i.id).unwrap_or_default()); }
        }
        v
    }).collect();

    let body = json!({
        "slug": w.slug, "type": w.r#type, "status": w.status, "title": w.title, "description": w.description,
        "cover_image_url": image_url(w.cover_image_key.as_deref(), &w.cover_image_status),
        "event_date": w.event_date, "id": w.id,
        "owner": { "id": w.owner_id, "display_name": w.owner_name },
        "surprise_mode": w.surprise_mode, "claimers_visible": visible,
        "completion": { "item_count": items.len(), "fulfilled_count": fulfilled, "completion_pct": pct(got, total) },
        "items": item_json, "updated_at": w.updated_at,
    });
    Ok((cache, [(header::ETAG, etag)], Json(body)).into_response())
}
