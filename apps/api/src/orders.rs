//! 營運後台（P2-A）：採購單佇列 / 狀態推進（admin/orders）與人工發點 / 凍結（admin/wallets）。全部限 STAFF。
//! 點數異動一律走 `points::post` / `release` / `refund_partial`；鎖序：清單（FOR SHARE）→ 錢包 → 品項 → 採購單 / 認捐。
use crate::{admin::{audit, limit, page, PageQ, Staff}, error::AppError, idempotency, points, AppState};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::FromRow;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/orders", get(list_orders))
        .route("/admin/orders/{id}", patch(patch_order))
        .route("/admin/wallets", get(list_wallets))
        .route("/admin/wallets/grants", post(grant))
        .route("/admin/wallets/{id}", patch(set_wallet_status))
}

type R<T> = Result<T, AppError>;
const STATUSES: [&str; 6] = ["pending", "placed", "shipped", "delivered", "failed", "cancelled"];

// ---------- GET /admin/orders ----------
#[derive(Deserialize)]
struct OrdQ { cursor: Option<Uuid>, limit: Option<i64>, status: Option<String>, fulfillment_type: Option<String> }

#[derive(FromRow)]
struct OrderRow {
    id: Uuid, fulfillment_type: String, status: String, item_id: Uuid, item_title: String, product_url: Option<String>,
    wishlist_id: Uuid, wishlist_title: String, target_points: Option<i64>, amount: i64, merchant_order_id: Option<String>,
    tracking_no: Option<String>, failure_reason: Option<String>, snapshot: Vec<u8>, operator_id: Option<Uuid>, operator_name: Option<String>,
    placed_at: Option<DateTime<Utc>>, shipped_at: Option<DateTime<Utc>>, delivered_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>, updated_at: DateTime<Utc>,
}

async fn list_orders(State(st): State<AppState>, Staff(sid): Staff, Query(q): Query<OrdQ>) -> R<Response> {
    if q.status.as_deref().is_some_and(|s| !STATUSES.contains(&s)) { return Err(AppError::invalid("/status", "ENUM", "status 不正確")); }
    if q.fulfillment_type.as_deref().is_some_and(|s| s != "concierge" && s != "catalog") { return Err(AppError::invalid("/fulfillment_type", "ENUM", "fulfillment_type 不正確")); }
    let lim = limit(&PageQ { cursor: q.cursor, limit: q.limit, q: None, status: None, moderation_status: None });
    let rows: Vec<OrderRow> = sqlx::query_as(
        "SELECT po.id, po.fulfillment_type::text AS fulfillment_type, po.status::text AS status, i.id AS item_id, i.title AS item_title, i.product_url,
                w.id AS wishlist_id, w.title AS wishlist_title, i.target_points, po.amount, po.merchant_order_id, po.tracking_no, po.failure_reason,
                po.shipping_address_snapshot AS snapshot, po.operator_user_id AS operator_id, ou.display_name AS operator_name,
                po.placed_at, po.shipped_at, po.delivered_at, po.created_at, po.updated_at
           FROM purchase_orders po JOIN wishlist_items i ON i.id = po.item_id JOIN wishlists w ON w.id = i.wishlist_id
           LEFT JOIN users ou ON ou.id = po.operator_user_id
          WHERE ($1::text IS NULL OR po.status::text = $1) AND ($2::text IS NULL OR po.fulfillment_type::text = $2) AND ($3::uuid IS NULL OR po.id > $3)
          ORDER BY po.id LIMIT $4")
        .bind(&q.status).bind(&q.fulfillment_type).bind(q.cursor).bind(lim + 1).fetch_all(&st.pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows.into_iter().take(lim as usize + 1) {
        // 讀取收件明文 = 敏感操作，逐筆留 audit（明文本身不進 audit）
        audit(&st.pool, sid, "order.view_address", "purchase_orders", Some(r.id), json!({ "item_id": r.item_id })).await?;
        let addr = crate::sealed::open(&r.snapshot).and_then(|b| serde_json::from_slice::<Value>(&b).ok()).unwrap_or(Value::Null);
        let image: Option<(Option<String>, String)> = sqlx::query_as("SELECT image_key, image_status::text FROM wishlist_items WHERE id = $1").bind(r.item_id).fetch_optional(&st.pool).await?;
        let image_url = image.and_then(|(k, s)| crate::public::image_url(k.as_deref(), &s));
        out.push((r.id, json!({
            "id": r.id, "fulfillment_type": r.fulfillment_type, "status": r.status,
            "item": { "id": r.item_id, "title": r.item_title, "product_url": r.product_url, "image_url": image_url },
            "wishlist": { "id": r.wishlist_id, "title": r.wishlist_title },
            "target_points": r.target_points, "amount": r.amount, "merchant": null, "merchant_order_id": r.merchant_order_id,
            "tracking_no": r.tracking_no, "failure_reason": r.failure_reason, "shipping_address": addr,
            "operator": r.operator_id.map(|id| json!({ "user_id": id, "display_name": r.operator_name })),
            "placed_at": r.placed_at, "shipped_at": r.shipped_at, "delivered_at": r.delivered_at,
            "created_at": r.created_at, "updated_at": r.updated_at,
        })));
    }
    Ok(page(out, lim))
}

// ---------- PATCH /admin/orders/{id} ----------
fn bad_transition(from: &str, to: &str) -> AppError {
    AppError::problem(409, "INVALID_STATE_TRANSITION", format!("採購單不能從 {from} 變更為 {to}"))
}

fn opt_text(m: &Map<String, Value>, k: &str, max: usize) -> R<Option<String>> {
    match m.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let s = crate::validate::text(s, &format!("/{k}"), false)?;
            if s.is_empty() { return Ok(None); }
            if s.chars().count() > max { return Err(AppError::invalid(&format!("/{k}"), "RANGE", &format!("{k} 至多 {max} 字"))); }
            Ok(Some(s))
        }
        _ => Err(AppError::invalid(&format!("/{k}"), "TYPE", &format!("{k} 必須是字串"))),
    }
}

async fn patch_order(State(st): State<AppState>, Staff(sid): Staff, Path(id): Path<Uuid>, Json(b): Json<Value>) -> R<Json<Value>> {
    let m = b.as_object().ok_or_else(|| AppError::problem(400, "BAD_REQUEST", "請求內容必須是 JSON 物件"))?;
    let to = m.get("status").and_then(Value::as_str).filter(|s| STATUSES.contains(s)).ok_or_else(|| AppError::invalid("/status", "ENUM", "status 必須是 pending | placed | shipped | delivered | failed | cancelled"))?;
    let merchant_order_id = opt_text(m, "merchant_order_id", 100)?;
    let tracking_no = opt_text(m, "tracking_no", 100)?;
    let failure_reason = opt_text(m, "failure_reason", 200)?;
    let amount = match m.get("amount") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().filter(|a| *a >= 1).ok_or_else(|| AppError::invalid("/amount", "RANGE", "amount 必須是正整數"))?),
    };

    let mut tx = st.pool.begin().await?;
    let (item_id, wid): (Uuid, Uuid) = sqlx::query_as("SELECT po.item_id, i.wishlist_id FROM purchase_orders po JOIN wishlist_items i ON i.id = po.item_id WHERE po.id = $1")
        .bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    // 鎖序：清單 → 捐贈者錢包（依 id 排序）→ 品項 → 採購單
    sqlx::query("SELECT id FROM wishlists WHERE id = $1 FOR SHARE").bind(wid).execute(&mut *tx).await?;
    let wallets: Vec<Uuid> = sqlx::query_scalar("SELECT DISTINCT wallet_id FROM contributions WHERE item_id = $1 AND status = 'captured'").bind(item_id).fetch_all(&mut *tx).await?;
    points::lock_wallets(&mut tx, &wallets).await?;
    let (from, target, deadline_future): (String, Option<i64>, bool) = sqlx::query_as(
        "SELECT po.status::text, i.target_points, i.funding_deadline > now() FROM wishlist_items i JOIN purchase_orders po ON po.item_id = i.id
          WHERE po.id = $1 FOR UPDATE OF i, po").bind(id).fetch_one(&mut *tx).await?;
    let allowed = matches!((from.as_str(), to), ("pending", "placed") | ("placed", "shipped") | ("shipped", "delivered") | ("pending" | "placed", "failed" | "cancelled"));
    if !allowed { return Err(bad_transition(&from, to)); }

    let mut refunded = 0i64;
    match to {
        "placed" => {
            let mo = merchant_order_id.as_deref().ok_or_else(|| AppError::invalid("/merchant_order_id", "REQUIRED", "標記已下單必須填 merchant_order_id"))?;
            let amount = amount.ok_or_else(|| AppError::invalid("/amount", "REQUIRED", "標記已下單必須填實際花費 amount"))?;
            let target = target.unwrap_or(0);
            if amount > target { return Err(AppError::invalid("/amount", "RANGE", &format!("實際花費 {amount} 超過目標 {target} 點；請改標記為 failed"))); }
            refunded = points::refund_partial(&mut tx, item_id, target - amount).await?; // 差額依認捐比例退回（4.3f）
            sqlx::query("UPDATE purchase_orders SET status = 'placed', merchant_order_id = $2, amount = $3, operator_user_id = $4, placed_at = now() WHERE id = $1")
                .bind(id).bind(mo).bind(amount).bind(sid).execute(&mut *tx).await?;
        }
        "shipped" => {
            sqlx::query("UPDATE purchase_orders SET status = 'shipped', tracking_no = COALESCE($2, tracking_no), operator_user_id = $3, shipped_at = now() WHERE id = $1")
                .bind(id).bind(&tracking_no).bind(sid).execute(&mut *tx).await?;
            crate::notify::enqueue_donors(&mut *tx, item_id, "order.shipped", &["captured"]).await?;
        }
        "delivered" => {
            sqlx::query("UPDATE purchase_orders SET status = 'delivered', tracking_no = COALESCE($2, tracking_no), operator_user_id = $3, delivered_at = now() WHERE id = $1")
                .bind(id).bind(&tracking_no).bind(sid).execute(&mut *tx).await?;
            sqlx::query("UPDATE wishlist_items SET funding_status = 'fulfilled' WHERE id = $1").bind(item_id).execute(&mut *tx).await?;
            crate::notify::enqueue_donors(&mut *tx, item_id, "order.delivered", &["captured"]).await?;
        }
        _ => { // failed | cancelled
            let reason = failure_reason.as_deref().ok_or_else(|| AppError::invalid("/failure_reason", "REQUIRED", "標記失敗 / 取消必須填 failure_reason"))?;
            let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM contributions WHERE item_id = $1 AND status = 'captured'").bind(item_id).fetch_all(&mut *tx).await?;
            crate::notify::enqueue_donors(&mut *tx, item_id, "order.failed", &["captured"]).await?; // release 之前：之後這些認捐就不是 captured 了
            for r in points::release(&mut tx, &ids, &["captured"]).await? { refunded += r.back; } // 扣除已部分退款的點數
            // 品項回 open（仍在期限內）或 expired（已過期限；donors 已全數退回，無 7 天選擇期可言）
            sqlx::query("UPDATE wishlist_items SET pledged_points = 0,
                           funding_status = CASE WHEN $2 THEN 'open'::funding_status ELSE 'expired'::funding_status END,
                           expired_at = CASE WHEN $2 THEN NULL ELSE now() END WHERE id = $1")
                .bind(item_id).bind(deadline_future).execute(&mut *tx).await?;
            sqlx::query("UPDATE purchase_orders SET status = $2::order_status, failure_reason = $3, operator_user_id = $4 WHERE id = $1")
                .bind(id).bind(to).bind(reason).bind(sid).execute(&mut *tx).await?;
        }
    }
    audit(&mut *tx, sid, "order.update", "purchase_orders", Some(id),
          json!({ "from": from, "to": to, "amount": amount, "refunded_points": refunded, "failure_reason": failure_reason, "tracking_no": tracking_no })).await?;
    crate::dashboard::notify(&mut *tx, wid).await?;
    let r: (Uuid, String, String, i64, Option<String>, Option<String>, Option<String>, Option<Uuid>, Option<String>, DateTime<Utc>) = sqlx::query_as(
        "SELECT po.id, po.fulfillment_type::text, po.status::text, po.amount, po.merchant_order_id, po.tracking_no, po.failure_reason, po.operator_user_id, ou.display_name, po.updated_at
           FROM purchase_orders po LEFT JOIN users ou ON ou.id = po.operator_user_id WHERE po.id = $1").bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": r.0, "fulfillment_type": r.1, "status": r.2, "amount": r.3, "merchant_order_id": r.4, "tracking_no": r.5, "failure_reason": r.6,
        "operator": r.7.map(|u| json!({ "user_id": u, "display_name": r.8 })), "refunded_points": refunded, "updated_at": r.9 })))
}

// ---------- 錢包 ----------
#[derive(FromRow)]
struct WalletRow { id: Uuid, user_id: Uuid, balance: i64, status: String }
impl WalletRow { fn json(&self) -> Value { json!({ "id": self.id, "user_id": self.user_id, "balance": self.balance, "status": self.status }) } }

/// GET /admin/wallets?q=：以 email / 暱稱 / user id 搜尋使用者，連同錢包（沒有錢包 = id null、餘額 0）
async fn list_wallets(State(st): State<AppState>, Staff(sid): Staff, Query(q): Query<PageQ>) -> R<Response> {
    let qs = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = qs { audit(&st.pool, sid, "admin.search", "point_wallets", None, json!({ "q": s })).await?; }
    let qid = qs.and_then(|s| s.parse::<Uuid>().ok());
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(
        "SELECT u.id, jsonb_build_object('user', jsonb_build_object('id', u.id, 'display_name', u.display_name, 'email', u.email),
                'wallet', jsonb_build_object('id', w.id, 'balance', COALESCE(w.balance, 0), 'status', COALESCE(w.status::text, 'active')))
           FROM users u LEFT JOIN point_wallets w ON w.user_id = u.id
          WHERE u.deleted_at IS NULL
            AND ($1::text IS NULL OR u.id = $2 OR strpos(lower(u.email), lower($1)) > 0 OR strpos(lower(u.display_name), lower($1)) > 0)
            AND ($3::uuid IS NULL OR u.id > $3)
          ORDER BY u.id LIMIT $4")
        .bind(qs).bind(qid).bind(q.cursor).bind(limit(&q) + 1).fetch_all(&st.pool).await?;
    Ok(page(rows, limit(&q)))
}

fn reason_of(b: &Value) -> R<String> {
    let s = b.get("reason").and_then(Value::as_str).ok_or_else(|| AppError::invalid("/reason", "REQUIRED", "reason 必填"))?;
    let s = crate::validate::text(s, "/reason", false)?;
    if s.is_empty() || s.chars().count() > 200 { return Err(AppError::invalid("/reason", "RANGE", "reason 需為 1–200 字")); }
    Ok(s)
}

/// POST /admin/wallets/grants：營運發點（正數 grant）或更正扣點（負數 adjustment，扣到負數 → 409 INSUFFICIENT_POINTS）。
/// Idempotency-Key 必填：占位與入帳同一交易，重放回原回應並帶 Idempotency-Replayed。
async fn grant(State(st): State<AppState>, Staff(sid): Staff, headers: HeaderMap, body: Bytes) -> R<Response> {
    let key = idempotency::key(&headers)?;
    let b: Value = serde_json::from_slice(&body).map_err(|_| AppError::problem(400, "BAD_REQUEST", "請求內容必須是 JSON"))?;
    let user_id: Uuid = b.get("user_id").and_then(Value::as_str).and_then(|s| s.parse().ok()).ok_or_else(|| AppError::invalid("/user_id", "REQUIRED", "user_id 必填（UUID）"))?;
    let pts = b.get("points").and_then(Value::as_i64).filter(|p| *p != 0 && p.abs() <= 1_000_000)
        .ok_or_else(|| AppError::invalid("/points", "RANGE", "points 必須是非 0 整數，且絕對值不超過 1,000,000"))?;
    let reason = reason_of(&b)?;
    let scope = format!("staff:{sid}:POST /admin/wallets/grants");
    let hash = idempotency::request_hash("POST /admin/wallets/grants", &body);
    let mut tx = st.pool.begin().await?;
    if let idempotency::Begin::Replay(s, body) = idempotency::begin(&mut tx, &scope, key, &hash).await? {
        let mut res = (StatusCode::from_u16(s as u16).unwrap_or(StatusCode::OK), Json(body)).into_response();
        res.headers_mut().insert("idempotency-replayed", HeaderValue::from_static("true"));
        return Ok(res);
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)").bind(user_id).fetch_one(&mut *tx).await?;
    if !exists { return Err(AppError::NotFound); }
    let wid = points::ensure_wallet(&mut tx, user_id).await?;
    points::lock_wallets(&mut tx, &[wid]).await?;
    let kind = if pts > 0 { "grant" } else { "adjustment" };
    points::post(&mut tx, wid, pts, points::Entry { kind, ref_type: "manual", ref_id: None, note: Some(&reason), actor: Some(sid) }).await?;
    let entry: (Uuid, i64, i64, String, Option<String>, Option<Uuid>, Option<String>, DateTime<Utc>) = sqlx::query_as(
        "SELECT id, delta, balance_after, entry_type::text, ref_type, ref_id, note, created_at FROM point_ledger WHERE wallet_id = $1 ORDER BY seq DESC LIMIT 1")
        .bind(wid).fetch_one(&mut *tx).await?;
    let w: WalletRow = sqlx::query_as("SELECT id, user_id, balance, status::text AS status FROM point_wallets WHERE id = $1").bind(wid).fetch_one(&mut *tx).await?;
    audit(&mut *tx, sid, "wallet.grant", "point_wallets", Some(wid), json!({ "user_id": user_id, "points": pts, "reason": reason, "ledger_id": entry.0 })).await?;
    let out = json!({ "wallet": w.json(), "entry": { "id": entry.0, "delta": entry.1, "balance_after": entry.2, "entry_type": entry.3,
        "ref_type": entry.4, "ref_id": entry.5, "note": entry.6, "created_at": entry.7 } });
    idempotency::finish(&mut tx, &scope, key, 201, &out).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, [(header::CACHE_CONTROL, "private, no-store")], Json(out)).into_response())
}

/// PATCH /admin/wallets/{id}：凍結 / 解凍（凍結後使用者不能再認捐；退點仍會入帳）
async fn set_wallet_status(State(st): State<AppState>, Staff(sid): Staff, Path(id): Path<Uuid>, Json(b): Json<Value>) -> R<Json<Value>> {
    let to = b.get("status").and_then(Value::as_str).filter(|s| matches!(*s, "active" | "frozen"))
        .ok_or_else(|| AppError::invalid("/status", "ENUM", "status 必須是 active | frozen"))?;
    let reason = reason_of(&b)?;
    let mut tx = st.pool.begin().await?;
    let from: String = sqlx::query_scalar("SELECT status::text FROM point_wallets WHERE id = $1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    let w: WalletRow = sqlx::query_as("UPDATE point_wallets SET status = $2::wallet_status WHERE id = $1 RETURNING id, user_id, balance, status::text AS status")
        .bind(id).bind(to).fetch_one(&mut *tx).await?;
    audit(&mut *tx, sid, "wallet.set_status", "point_wallets", Some(id), json!({ "from": from, "to": to, "reason": reason })).await?;
    tx.commit().await?;
    Ok(Json(w.json()))
}
