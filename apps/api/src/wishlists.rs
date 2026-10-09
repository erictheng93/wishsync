//! 清單（建立者）與品項 CRUD，全部限擁有者；非擁有者一律 NOT_FOUND。
use crate::{error::AppError, session::CurrentUser, uploads, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use rand::Rng;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::PgPool;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/wishlists", post(create).get(list))
        .route("/wishlists/{id}", get(get_one).patch(update).delete(archive))
        .route("/wishlists/{id}/dashboard", get(dashboard))
        .route("/wishlists/{id}/items", post(item_create))
        .route("/wishlists/{id}/items/reorder", post(item_reorder))
        .route("/items/{item_id}", patch(item_update).delete(item_delete))
}

type R<T> = Result<T, AppError>;
fn app_url() -> String { std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:3000".into()).trim_end_matches('/').to_string() }
/// 完成度唯一定義：sum(qty_claimed)/sum(qty_needed)*100 四捨五入；總需求 0 回 0。
pub fn completion_pct(claimed: i64, needed: i64) -> i64 { if needed <= 0 { 0 } else { (claimed * 100 + needed / 2) / needed } }
fn forbidden(d: &str) -> AppError { AppError::problem(403, "FORBIDDEN", d) }
fn closed() -> AppError { AppError::problem(409, "WISHLIST_CLOSED", "清單已關閉或封存，無法修改。") }
fn stale() -> AppError { AppError::problem(409, "STALE_VERSION", "這份資料已在其他地方被修改，請重新載入後再試。") }
fn surprise_locked_err() -> AppError { forbidden("驚喜模式期間無法刪除品項或調降數量。") }
/// 樂觀並行控制：body 的 expected_updated_at（選填）；未帶 = None，維持舊行為。
fn expected(m: &Map<String, Value>) -> R<Option<DateTime<Utc>>> {
    match m.get("expected_updated_at") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => s.parse().map(Some).map_err(|_| AppError::invalid("/expected_updated_at", "FORMAT", "expected_updated_at 須為 RFC 3339 時間")),
        _ => Err(AppError::invalid("/expected_updated_at", "TYPE", "expected_updated_at 須為字串")),
    }
}
fn today_tw() -> NaiveDate { (Utc::now() + Duration::hours(8)).date_naive() }

// ---------- 清單 ----------
#[derive(sqlx::FromRow)]
struct W {
    id: Uuid, ty: String, status: String, visibility: String, slug: String, title: String, description: Option<String>,
    cover_image_key: Option<String>, cover_image_status: String, event_date: Option<NaiveDate>, show_claimer_names: bool,
    surprise_mode: bool, surprise_locked: bool, claim_ttl_hours: Option<i32>, moderation_status: String,
    moderation_reason: Option<String>, moderated_at: Option<DateTime<Utc>>, created_at: DateTime<Utc>, updated_at: DateTime<Utc>,
}
const WCOLS: &str = "id, type::text AS ty, status::text AS status, visibility::text AS visibility, slug::text AS slug, title, description,
  cover_image_key, cover_image_status::text AS cover_image_status, event_date, show_claimer_names, surprise_mode,
  (surprise_mode AND event_date IS NOT NULL AND now() < (event_date::timestamp AT TIME ZONE 'Asia/Taipei')) AS surprise_locked,
  claim_ttl_hours, moderation_status::text AS moderation_status, moderation_reason, moderated_at, created_at, updated_at";

impl W {
    fn json(&self) -> Value {
        let img = uploads::S3::from_env();
        json!({
            "id": self.id, "type": self.ty, "status": self.status, "visibility": self.visibility, "slug": self.slug,
            "title": self.title, "description": self.description,
            "cover_image_url": self.cover_image_key.as_ref().filter(|_| self.cover_image_status == "ready").map(|k| img.public_url(k)),
            "cover_image_status": self.cover_image_status, "event_date": self.event_date,
            "show_claimer_names": self.show_claimer_names, "surprise_mode": self.surprise_mode, "surprise_locked": self.surprise_locked,
            "claim_ttl_hours": self.claim_ttl_hours, "moderation_status": self.moderation_status,
            "moderation_reason": self.moderation_reason, "moderated_at": self.moderated_at,
            "has_shipping_address": false, "org_id": null, "location": null, "address": null, "site_status": null,
            "share_url": format!("{}/s/{}", app_url(), self.slug), "created_at": self.created_at, "updated_at": self.updated_at,
        })
    }
}

async fn load_owned(pool: &PgPool, id: Uuid, user: Uuid) -> R<W> {
    sqlx::query_as(&format!("SELECT {WCOLS} FROM wishlists WHERE id=$1 AND owner_id=$2 AND deleted_at IS NULL"))
        .bind(id).bind(user).fetch_optional(pool).await?.ok_or(AppError::NotFound)
}

fn new_slug() -> String {
    const A: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut r = rand::thread_rng();
    (0..10).map(|_| A[r.gen_range(0..A.len())] as char).collect()
}

fn obj(v: &Value) -> R<&Map<String, Value>> { v.as_object().ok_or_else(|| AppError::problem(400, "BAD_REQUEST", "請求內容必須是 JSON 物件")) }

/// 讀取字串欄位：None=未帶；Some(None)=null/空字串；Some(Some)=值（trim 後，長度 min..=max 字元）
fn text(m: &Map<String, Value>, k: &str, min: usize, max: usize) -> R<Option<Option<String>>> {
    match m.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.is_empty() && min == 0 { return Ok(Some(None)); }
            let n = s.chars().count();
            if n < min.max(1) || n > max { return Err(AppError::invalid(&format!("/{k}"), "RANGE", &format!("{k} 長度須為 {} 到 {max} 字", min.max(1)))); }
            Ok(Some(Some(s.to_string())))
        }
        _ => Err(AppError::invalid(&format!("/{k}"), "TYPE", &format!("{k} 必須是字串"))),
    }
}
fn boolean(m: &Map<String, Value>, k: &str) -> R<Option<bool>> {
    match m.get(k) { None => Ok(None), Some(Value::Bool(b)) => Ok(Some(*b)), _ => Err(AppError::invalid(&format!("/{k}"), "TYPE", &format!("{k} 必須是布林值"))) }
}
fn int(m: &Map<String, Value>, k: &str, lo: i64, hi: i64) -> R<Option<Option<i64>>> {
    match m.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(v) => match v.as_i64() {
            Some(n) if (lo..=hi).contains(&n) => Ok(Some(Some(n))),
            _ => Err(AppError::invalid(&format!("/{k}"), "RANGE", &format!("{k} 必須介於 {lo} 與 {hi}"))),
        },
    }
}
fn one_of<'a>(m: &'a Map<String, Value>, k: &str, allowed: &[&str]) -> R<Option<&'a str>> {
    match m.get(k) {
        None => Ok(None),
        Some(Value::String(s)) if allowed.contains(&s.as_str()) => Ok(Some(s)),
        _ => Err(AppError::invalid(&format!("/{k}"), "ENUM", &format!("{k} 必須是 {}", allowed.join(" / ")))),
    }
}
fn date(m: &Map<String, Value>, k: &str) -> R<Option<Option<NaiveDate>>> {
    match m.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => s.parse().map(|d| Some(Some(d))).map_err(|_| AppError::invalid(&format!("/{k}"), "FORMAT", "日期格式須為 YYYY-MM-DD")),
        _ => Err(AppError::invalid(&format!("/{k}"), "TYPE", "日期格式須為 YYYY-MM-DD")),
    }
}
/// 圖片 key：須為 uploads 發出的格式，且前綴符合用途
fn image_key(m: &Map<String, Value>, k: &str, dir: &str) -> R<Option<Option<String>>> {
    Ok(match text(m, k, 0, 200)? {
        Some(Some(s)) if !(uploads::valid_key(&s) && s.starts_with(dir)) => return Err(AppError::invalid(&format!("/{k}"), "FORMAT", "圖片 key 格式不正確")),
        v => v,
    })
}

async fn create(u: CurrentUser, State(st): State<AppState>, Json(b): Json<Value>) -> R<Response> {
    let m = obj(&b)?;
    let ty = one_of(m, "type", &["personal", "registry", "relief"])?.ok_or_else(|| AppError::invalid("/type", "REQUIRED", "type 必填"))?;
    if ty == "relief" { return Err(AppError::invalid("/type", "ENUM", "relief 據點請使用 POST /orgs/{id}/sites")); }
    let title = text(m, "title", 1, 100)?.flatten().ok_or_else(|| AppError::invalid("/title", "REQUIRED", "title 必填"))?;
    let desc = text(m, "description", 0, 2000)?.flatten();
    let cover = image_key(m, "cover_image_key", "covers/")?.flatten();
    let event = date(m, "event_date")?.flatten();
    let vis = one_of(m, "visibility", &["link", "private"])?.unwrap_or("link");
    let names = boolean(m, "show_claimer_names")?.unwrap_or(false);
    let surprise = boolean(m, "surprise_mode")?.unwrap_or(false);
    let ttl = int(m, "claim_ttl_hours", 1, 24 * 365)?.flatten();
    if surprise && !event.is_some_and(|d| d > today_tw()) { return Err(AppError::invalid("/event_date", "REQUIRED", "驚喜模式需填寫未來的活動日期")); }
    for _ in 0..5 {
        let r = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO wishlists (owner_id, type, visibility, slug, title, description, cover_image_key, cover_image_status, event_date, show_claimer_names, surprise_mode, claim_ttl_hours)
             VALUES ($1,$2::wishlist_type,$3::visibility,$4,$5,$6,$7, CASE WHEN $7::text IS NULL THEN 'none' ELSE 'ready' END::image_status,$8,$9,$10,$11) RETURNING id")
            .bind(u.id).bind(ty).bind(vis).bind(new_slug()).bind(&title).bind(&desc).bind(&cover).bind(event).bind(names).bind(surprise).bind(ttl.map(|t| t as i32))
            .fetch_one(&st.pool).await;
        match r {
            Ok(id) => {
                let w = load_owned(&st.pool, id, u.id).await?;
                return Ok((StatusCode::CREATED, [(header::LOCATION, format!("/api/v1/wishlists/{id}"))], Json(w.json())).into_response());
            }
            Err(sqlx::Error::Database(e)) if e.constraint() == Some("wishlists_slug_key") => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(AppError::problem(500, "INTERNAL_ERROR", "無法產生分享代碼，請重試"))
}

#[derive(Deserialize)]
struct ListQ { cursor: Option<String>, limit: Option<i64>, status: Option<String>, #[serde(rename = "type")] ty: Option<String> }

async fn list(u: CurrentUser, State(st): State<AppState>, Query(q): Query<ListQ>) -> R<Json<Value>> {
    let limit = q.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) { return Err(AppError::invalid("/limit", "RANGE", "limit 必須介於 1 與 100")); }
    if q.status.as_deref().is_some_and(|s| !["draft", "active", "closed", "archived"].contains(&s)) { return Err(AppError::invalid("/status", "ENUM", "status 不正確")); }
    if q.ty.as_deref().is_some_and(|s| !["personal", "registry", "relief"].contains(&s)) { return Err(AppError::invalid("/type", "ENUM", "type 不正確")); }
    let after: Option<Uuid> = match &q.cursor {
        None => None,
        Some(c) => Some(B64.decode(c).ok().and_then(|b| String::from_utf8(b).ok()).and_then(|s| s.parse().ok()).ok_or_else(|| AppError::invalid("/cursor", "FORMAT", "cursor 無效"))?),
    };
    let rows: Vec<W> = sqlx::query_as(&format!(
        "SELECT {WCOLS} FROM wishlists WHERE owner_id=$1 AND deleted_at IS NULL AND ($2::uuid IS NULL OR id < $2)
         AND ($3::text IS NULL OR status::text=$3) AND ($4::text IS NULL OR type::text=$4) ORDER BY id DESC LIMIT $5"))
        .bind(u.id).bind(after).bind(&q.status).bind(&q.ty).bind(limit + 1).fetch_all(&st.pool).await?;
    let more = rows.len() as i64 > limit;
    let rows = &rows[..rows.len().min(limit as usize)];
    let ids: Vec<Uuid> = rows.iter().map(|w| w.id).collect();
    let stats: Vec<(Uuid, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT wishlist_id, count(*), count(*) FILTER (WHERE qty_claimed >= qty_needed), sum(qty_claimed)::bigint, sum(qty_needed)::bigint FROM wishlist_items
         WHERE wishlist_id = ANY($1) AND deleted_at IS NULL GROUP BY wishlist_id").bind(&ids).fetch_all(&st.pool).await?;
    let data: Vec<Value> = rows.iter().map(|w| {
        let (c, f, q, nd) = stats.iter().find(|s| s.0 == w.id).map(|s| (s.1, s.2, s.3, s.4)).unwrap_or((0, 0, 0, 0));
        let j = w.json();
        json!({ "id": w.id, "type": w.ty, "status": w.status, "slug": w.slug, "title": w.title, "event_date": w.event_date,
                "cover_image_url": j["cover_image_url"], "surprise_locked": w.surprise_locked,
                "completion": { "item_count": c, "fulfilled_count": f, "completion_pct": completion_pct(q, nd) },
                "moderation_status": w.moderation_status, "moderation_reason": w.moderation_reason,
                "updated_at": w.updated_at })
    }).collect();
    let next = if more { rows.last().map(|w| B64.encode(w.id.to_string())) } else { None };
    Ok(Json(json!({ "data": data, "next_cursor": next })))
}

// ---------- 品項 ----------
#[derive(sqlx::FromRow)]
struct I {
    id: Uuid, wishlist_id: Uuid, title: String, description: Option<String>, brand: Option<String>, spec: Option<String>,
    image_key: Option<String>, image_status: String, product_url: Option<String>, unit_price_amount: Option<i64>,
    funding_mode: String, priority: String, qty_needed: i32, qty_claimed: i32, sort_order: i32,
    created_at: DateTime<Utc>, updated_at: DateTime<Utc>,
}
const ICOLS: &str = "id, wishlist_id, title, description, brand, spec, image_key, image_status::text AS image_status, product_url, unit_price_amount,
  funding_mode::text AS funding_mode, priority::text AS priority, qty_needed, qty_claimed, sort_order, created_at, updated_at";

impl I {
    fn json(&self, locked: bool) -> Value {
        json!({
            "id": self.id, "wishlist_id": self.wishlist_id, "title": self.title, "description": self.description, "brand": self.brand, "spec": self.spec,
            "image_url": self.image_key.as_ref().filter(|_| self.image_status == "ready").map(|k| uploads::S3::from_env().public_url(k)),
            "image_status": self.image_status, "product_url": self.product_url, "unit_price_amount": self.unit_price_amount,
            "funding_mode": self.funding_mode, "priority": self.priority, "category": null, "urgency": null,
            "qty_needed": self.qty_needed, "qty_claimed": if locked { None } else { Some(self.qty_claimed) }, "qty_received": 0,
            "target_points": null, "pledged_points": null, "funding_status": null, "funding_deadline": null, "fulfillment_type": null,
            "catalog_product_id": null, "price_snapshot_amount": null, "expired_at": null, "order_status": null,
            "sort_order": self.sort_order, "created_at": self.created_at,
            // 認領會推進品項 updated_at；鎖定期間遮蔽，避免由其變化推知有人認領
            "updated_at": if locked { self.created_at } else { self.updated_at },
        })
    }
}

async fn items_of(pool: &PgPool, wid: Uuid) -> R<Vec<I>> {
    Ok(sqlx::query_as(&format!("SELECT {ICOLS} FROM wishlist_items WHERE wishlist_id=$1 AND deleted_at IS NULL ORDER BY sort_order, id")).bind(wid).fetch_all(pool).await?)
}

async fn get_one(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>) -> R<Json<Value>> {
    let w = load_owned(&st.pool, id, u.id).await?;
    let items: Vec<Value> = items_of(&st.pool, id).await?.iter().map(|i| i.json(w.surprise_locked)).collect();
    Ok(Json(json!({ "wishlist": w.json(), "items": items })))
}

async fn update(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>, Json(b): Json<Value>) -> R<Json<Value>> {
    let m = obj(&b)?;
    let mut w = load_owned(&st.pool, id, u.id).await?;
    if w.status == "archived" {
        // 重複封存為冪等（FR-15）：只帶 status=archived（與 expected_updated_at）時直接回目前狀態
        if m.get("status").is_some_and(|v| v == "archived") && m.keys().all(|k| k == "status" || k == "expected_updated_at") { return Ok(Json(w.json())); }
        return Err(closed());
    }
    let exp = expected(m)?;
    if let Some(v) = text(m, "title", 1, 100)? { w.title = v.unwrap_or_default(); }
    if let Some(v) = text(m, "description", 0, 2000)? { w.description = v; }
    if let Some(v) = image_key(m, "cover_image_key", "covers/")? { w.cover_image_status = if v.is_some() { "ready" } else { "none" }.into(); w.cover_image_key = v; }
    if let Some(v) = date(m, "event_date")? { w.event_date = v; }
    if let Some(v) = one_of(m, "visibility", &["link", "private"])? { w.visibility = v.into(); }
    if let Some(v) = boolean(m, "show_claimer_names")? { w.show_claimer_names = v; }
    if let Some(v) = int(m, "claim_ttl_hours", 1, 24 * 365)? { w.claim_ttl_hours = v.map(|t| t as i32); }
    if let Some(v) = boolean(m, "surprise_mode")? {
        if !v && w.surprise_locked { return Err(forbidden("驚喜鎖定期間不可關閉驚喜模式")); }
        w.surprise_mode = v;
    }
    if w.surprise_mode && !w.event_date.is_some_and(|d| d > today_tw()) && (m.contains_key("surprise_mode") || m.contains_key("event_date")) {
        return Err(AppError::invalid("/event_date", "REQUIRED", "驚喜模式需填寫未來的活動日期"));
    }
    if let Some(new) = one_of(m, "status", &["draft", "active", "closed", "archived"])? {
        let ok = matches!((w.status.as_str(), new), (a, b) if a == b) || matches!((w.status.as_str(), new),
            ("draft", "active") | ("active", "closed") | ("closed", "active") | ("draft", "archived") | ("active", "archived") | ("closed", "archived"));
        if !ok { return Err(AppError::invalid("/status", "TRANSITION", &format!("不能從 {} 變更為 {new}", w.status))); }
        if new == "active" && w.status == "draft" {
            let n: i64 = sqlx::query_scalar("SELECT count(*) FROM wishlist_items WHERE wishlist_id=$1 AND deleted_at IS NULL").bind(id).fetch_one(&st.pool).await?;
            let mut miss = vec![];
            if w.title.trim().is_empty() { miss.push(json!({ "pointer": "/title", "code": "REQUIRED", "detail": "標題不可為空" })); }
            if n == 0 { miss.push(json!({ "pointer": "/items", "code": "REQUIRED", "detail": "至少需要 1 個品項" })); }
            if w.surprise_mode && !w.event_date.is_some_and(|d| d > today_tw()) { miss.push(json!({ "pointer": "/event_date", "code": "REQUIRED", "detail": "驚喜模式的活動日須在今日之後" })); }
            if !miss.is_empty() {
                return Err(AppError::Problem { status: 409, code: "WISHLIST_NOT_PUBLISHABLE", detail: "清單尚不符合發佈條件。".into(), errors: Some(Value::Array(miss)), retry_after: None });
            }
        }
        w.status = new.into();
    }
    let res = sqlx::query("UPDATE wishlists SET title=$2, description=$3, cover_image_key=$4, cover_image_status=$5::image_status, event_date=$6, visibility=$7::visibility,
                 show_claimer_names=$8, surprise_mode=$9, claim_ttl_hours=$10, status=$11::wishlist_status,
                 closed_at = CASE WHEN $11 IN ('closed','archived') THEN coalesce(closed_at, now()) ELSE NULL END
                 WHERE id=$1 AND ($12::timestamptz IS NULL OR updated_at=$12)")
        .bind(id).bind(&w.title).bind(&w.description).bind(&w.cover_image_key).bind(&w.cover_image_status).bind(w.event_date).bind(&w.visibility)
        .bind(w.show_claimer_names).bind(w.surprise_mode).bind(w.claim_ttl_hours).bind(&w.status).bind(exp).execute(&st.pool).await?;
    if res.rows_affected() == 0 { return Err(stale()); }
    Ok(Json(load_owned(&st.pool, id, u.id).await?.json()))
}

async fn archive(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>) -> R<StatusCode> {
    load_owned(&st.pool, id, u.id).await?;
    sqlx::query("UPDATE wishlists SET status='archived', closed_at = coalesce(closed_at, now()) WHERE id=$1 AND status<>'archived'").bind(id).execute(&st.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn dashboard(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>) -> R<Json<Value>> {
    let w = load_owned(&st.pool, id, u.id).await?;
    let items = items_of(&st.pool, id).await?;
    let locked = w.surprise_locked;
    let (n, f) = (items.len() as i64, items.iter().filter(|i| i.qty_claimed >= i.qty_needed).count() as i64);
    let unlock: Option<DateTime<Utc>> = match w.event_date { Some(d) if w.surprise_mode => sqlx::query_scalar("SELECT ($1::date)::timestamp AT TIME ZONE 'Asia/Taipei'").bind(d).fetch_one(&st.pool).await?, _ => None };
    let claims = if locked { Value::Null } else {
        let rows: Vec<(Uuid, Uuid, String, i32, String, Option<String>, DateTime<Utc>)> = sqlx::query_as(
            "SELECT c.id, c.item_id, c.claimer_name, c.qty, c.status::text, c.note, c.created_at FROM claims c JOIN wishlist_items i ON i.id=c.item_id
             WHERE i.wishlist_id=$1 ORDER BY c.id").bind(id).fetch_all(&st.pool).await?;
        Value::Array(rows.into_iter().map(|r| json!({ "id": r.0, "item_id": r.1,
            "claimer_name": r.2, "qty": r.3, "status": r.4, "note": r.5, "created_at": r.6 })).collect())
    };
    Ok(Json(json!({
        "wishlist_id": id, "surprise_locked": locked, "unlock_at": unlock,
        "moderation_status": w.moderation_status, "moderation_reason": w.moderation_reason,
        "totals": { "item_count": n, "fulfilled_count": f, "completion_pct": completion_pct(items.iter().map(|i| i.qty_claimed as i64).sum(), items.iter().map(|i| i.qty_needed as i64).sum()),
            "qty_needed": items.iter().map(|i| i.qty_needed as i64).sum::<i64>(),
            "qty_claimed": items.iter().map(|i| i.qty_claimed as i64).sum::<i64>(),
            "target_points": null, "pledged_points": null, "funded_item_count": 0 },
        "items": items.iter().map(|i| json!({ "item_id": i.id, "title": i.title, "funding_mode": i.funding_mode, "qty_needed": i.qty_needed,
            "qty_claimed": if locked { None } else { Some(i.qty_claimed) }, "target_points": null, "pledged_points": null, "funding_status": null })).collect::<Vec<_>>(),
        "orders_summary": { "funded_count": 0, "ordered_count": 0, "shipped_count": 0, "delivered_count": 0 },
        "claims": claims, "contributions": null,
    })))
}

// ---------- 品項 handlers ----------
fn http_url(s: &str) -> bool { reqwest::Url::parse(s).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) }

fn apply_item(m: &Map<String, Value>, i: &mut I) -> R<()> {
    if let Some(v) = text(m, "title", 1, 120)? { i.title = v.unwrap_or_default(); }
    if let Some(v) = text(m, "description", 0, 2000)? { i.description = v; }
    if let Some(v) = text(m, "brand", 0, 100)? { i.brand = v; }
    if let Some(v) = text(m, "spec", 0, 200)? { i.spec = v; }
    if let Some(v) = text(m, "product_url", 0, 2000)? {
        if v.as_deref().is_some_and(|u| !http_url(u)) { return Err(AppError::invalid("/product_url", "FORMAT", "product_url 必須是 http(s) 網址")); }
        i.product_url = v;
    }
    if let Some(v) = int(m, "unit_price_amount", 0, 100_000_000)? { i.unit_price_amount = v; }
    if let Some(v) = one_of(m, "priority", &["high", "medium", "low"])? { i.priority = v.into(); }
    if let Some(v) = int(m, "qty_needed", 1, 9999)? { i.qty_needed = v.ok_or_else(|| AppError::invalid("/qty_needed", "REQUIRED", "qty_needed 不可為 null"))? as i32; }
    if let Some(v) = image_key(m, "image_key", "items/")? { i.image_status = if v.is_some() { "ready" } else { "none" }.into(); i.image_key = v; }
    if m.get("funding_mode").is_some_and(|v| v != "quantity") { return Err(AppError::invalid("/funding_mode", "ENUM", "目前僅支援 quantity")); }
    Ok(())
}

async fn item_create(u: CurrentUser, State(st): State<AppState>, Path(wid): Path<Uuid>, Json(b): Json<Value>) -> R<Response> {
    let m = obj(&b)?;
    let w = load_owned(&st.pool, wid, u.id).await?;
    if matches!(w.status.as_str(), "closed" | "archived") { return Err(closed()); }
    if !m.get("title").is_some_and(|t| t.is_string()) { return Err(AppError::invalid("/title", "REQUIRED", "title 必填")); }
    let mut i = I { id: Uuid::nil(), wishlist_id: wid, title: String::new(), description: None, brand: None, spec: None, image_key: None, image_status: "none".into(),
        product_url: None, unit_price_amount: None, funding_mode: "quantity".into(), priority: "medium".into(), qty_needed: 1, qty_claimed: 0, sort_order: 0,
        created_at: Utc::now(), updated_at: Utc::now() };
    apply_item(m, &mut i)?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO wishlist_items (wishlist_id, title, description, brand, spec, image_key, image_status, product_url, unit_price_amount, priority, qty_needed, sort_order)
         VALUES ($1,$2,$3,$4,$5,$6,$7::image_status,$8,$9,$10::item_priority,$11,
                 (SELECT coalesce(max(sort_order),0)+10 FROM wishlist_items WHERE wishlist_id=$1)) RETURNING id")
        .bind(wid).bind(&i.title).bind(&i.description).bind(&i.brand).bind(&i.spec).bind(&i.image_key).bind(&i.image_status)
        .bind(&i.product_url).bind(i.unit_price_amount).bind(&i.priority).bind(i.qty_needed).fetch_one(&st.pool).await?;
    let row: I = sqlx::query_as(&format!("SELECT {ICOLS} FROM wishlist_items WHERE id=$1")).bind(id).fetch_one(&st.pool).await?;
    Ok((StatusCode::CREATED, Json(row.json(w.surprise_locked))).into_response())
}

/// 取得擁有者的品項及其清單狀態
async fn owned_item(pool: &PgPool, item: Uuid, user: Uuid) -> R<(I, String, bool)> {
    let i: Option<I> = sqlx::query_as(&format!(
        "SELECT {ICOLS} FROM wishlist_items WHERE id=$1 AND deleted_at IS NULL AND wishlist_id IN (SELECT id FROM wishlists WHERE owner_id=$2 AND deleted_at IS NULL)"))
        .bind(item).bind(user).fetch_optional(pool).await?;
    let i = i.ok_or(AppError::NotFound)?;
    let w = load_owned(pool, i.wishlist_id, user).await?;
    Ok((i, w.status, w.surprise_locked))
}

async fn item_update(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>, Json(b): Json<Value>) -> R<Json<Value>> {
    let m = obj(&b)?;
    let (mut i, status, locked) = owned_item(&st.pool, id, u.id).await?;
    if matches!(status.as_str(), "closed" | "archived") { return Err(closed()); }
    let old_qty = i.qty_needed;
    // 鎖定期間品項的 updated_at 被遮蔽（見 I::json），故不做品項版本檢查
    let exp = if locked { None } else { expected(m)? };
    apply_item(m, &mut i)?;
    // 驚喜鎖定：不論有無認領，一律禁止調降（否則 QTY_BELOW_CLAIMED 會洩漏「有人認領」）
    if locked && i.qty_needed < old_qty { return Err(surprise_locked_err()); }
    // qty_needed >= qty_claimed 在同一 UPDATE 內判斷，避免與認領競爭
    let res = sqlx::query("UPDATE wishlist_items SET title=$2, description=$3, brand=$4, spec=$5, image_key=$6, image_status=$7::image_status, product_url=$8,
                 unit_price_amount=$9, priority=$10::item_priority, qty_needed=$11 WHERE id=$1 AND qty_claimed <= $11 AND ($12::timestamptz IS NULL OR updated_at=$12)")
        .bind(id).bind(&i.title).bind(&i.description).bind(&i.brand).bind(&i.spec).bind(&i.image_key).bind(&i.image_status).bind(&i.product_url)
        .bind(i.unit_price_amount).bind(&i.priority).bind(i.qty_needed).bind(exp).execute(&st.pool).await?;
    if res.rows_affected() == 0 {
        let cur: Option<(DateTime<Utc>, i32)> = sqlx::query_as("SELECT updated_at, qty_claimed FROM wishlist_items WHERE id=$1 AND deleted_at IS NULL").bind(id).fetch_optional(&st.pool).await?;
        return Err(match cur {
            None => AppError::NotFound,
            Some((u, _)) if exp.is_some_and(|e| e != u) => stale(),
            _ => AppError::problem(409, "QTY_BELOW_CLAIMED", "qty_needed 不可小於已認領數量。"),
        });
    }
    let row: I = sqlx::query_as(&format!("SELECT {ICOLS} FROM wishlist_items WHERE id=$1")).bind(id).fetch_one(&st.pool).await?;
    Ok(Json(row.json(locked)))
}

#[derive(Deserialize)]
struct ForceQ { force: Option<bool> }

async fn item_delete(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>, Query(q): Query<ForceQ>) -> R<StatusCode> {
    let (_, _, locked) = owned_item(&st.pool, id, u.id).await?;
    // 驚喜鎖定：一律禁止刪除，回應與有無認領無關
    if locked { return Err(surprise_locked_err()); }
    let mut tx = st.pool.begin().await?;
    let active: i64 = sqlx::query_scalar("SELECT count(*) FROM claims WHERE item_id=$1 AND status IN ('reserved','purchased')").bind(id).fetch_one(&mut *tx).await?;
    if active > 0 {
        if !q.force.unwrap_or(false) { return Err(AppError::problem(409, "ITEM_HAS_CLAIMS", "品項已有進行中的認領，需帶 force=true 才能刪除。")); }
        sqlx::query("UPDATE claims SET status='cancelled', cancelled_at=now() WHERE item_id=$1 AND status IN ('reserved','purchased')").bind(id).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE wishlist_items SET deleted_at=now() WHERE id=$1").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Reorder { item_ids: Vec<Uuid> }

async fn item_reorder(u: CurrentUser, State(st): State<AppState>, Path(wid): Path<Uuid>, Json(r): Json<Reorder>) -> R<StatusCode> {
    let w = load_owned(&st.pool, wid, u.id).await?;
    if matches!(w.status.as_str(), "closed" | "archived") { return Err(closed()); }
    let mut have: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM wishlist_items WHERE wishlist_id=$1 AND deleted_at IS NULL").bind(wid).fetch_all(&st.pool).await?;
    let mut want = r.item_ids.clone();
    have.sort(); want.sort();
    if have != want { return Err(AppError::invalid("/item_ids", "MISMATCH", "item_ids 必須與清單現有品項完全一致")); }
    sqlx::query("UPDATE wishlist_items i SET sort_order = t.ord::int * 10 FROM unnest($1::uuid[]) WITH ORDINALITY AS t(id, ord) WHERE i.id = t.id")
        .bind(&r.item_ids).execute(&st.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}
