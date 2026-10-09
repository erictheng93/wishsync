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
}

async fn wishlist(State(st): State<AppState>, Path(slug): Path<String>, req: HeaderMap) -> Result<Response, AppError> {
    if slug.len() != 10 { return Err(AppError::NotFound); }
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
        "SELECT id, title, brand, spec, image_key, image_status::text AS image_status, product_url, unit_price_amount,
                priority::text AS priority, funding_mode::text AS funding_mode, qty_needed, qty_claimed
         FROM wishlist_items WHERE wishlist_id = $1 AND deleted_at IS NULL ORDER BY sort_order, id")
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

    let (total, got): (i64, i64) = items.iter().fold((0, 0), |a, i| (a.0 + i.qty_needed as i64, a.1 + i.qty_claimed as i64));
    let pct = crate::wishlists::completion_pct;
    let fulfilled = items.iter().filter(|i| i.qty_claimed >= i.qty_needed).count();
    let item_json: Vec<Value> = items.iter().map(|i| {
        let mut v = json!({
            "id": i.id, "title": i.title, "brand": i.brand, "spec": i.spec,
            "image_url": image_url(i.image_key.as_deref(), &i.image_status),
            "product_url": i.product_url, "unit_price_amount": i.unit_price_amount, "priority": i.priority,
            "funding_mode": i.funding_mode, "qty_needed": i.qty_needed, "qty_claimed": i.qty_claimed,
            "qty_remaining": i.qty_needed - i.qty_claimed, "is_fully_claimed": i.qty_claimed >= i.qty_needed,
            "target_points": null, "pledged_points": null, "remaining_points": null,
            "funding_status": null, "display_status": null, "funding_deadline": null,
            "progress_percent": pct(i.qty_claimed as i64, i.qty_needed as i64),
        });
        if visible { v["claimers"] = json!(claimers.remove(&i.id).unwrap_or_default()); }
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
