//! 點數認捐（P2-A 捐贈者端）：POST /items/{id}/contributions、DELETE /contributions/{id}、POST /contributions/{id}/reallocate，
//! 以及背景 `tick_funding`（到期 → expired、選擇期結束 → 自動退點）。
//! 鎖序：wishlists（FOR SHARE）→ point_wallets（FOR UPDATE）→ wishlist_items → contributions；所有 balance 異動都走 `points::post`。
use crate::{error::AppError, idempotency, notify, points::{self, Entry}, ratelimit, session::CurrentUser, wishlists::completion_pct, AppState};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::Response,
    routing::{delete, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

type Tx<'a> = Transaction<'a, Postgres>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/items/{id}/contributions", post(create))
        .route("/contributions/{id}", delete(withdraw))
        .route("/contributions/{id}/reallocate", post(reallocate))
}

/// 認捐的 JSON 形狀 C（別名 `c` = contributions）；wallet.rs 共用
pub const C_JSON: &str = "jsonb_build_object('id', c.id, 'item_id', c.item_id, 'wishlist_id', c.wishlist_id, 'points', c.points,
    'refunded_points', c.refunded_points, 'status', c.status, 'message', c.message, 'is_anonymous', c.is_anonymous,
    'reallocated_to_item_id', c.reallocated_to_item_id, 'captured_at', c.captured_at, 'released_at', c.released_at,
    'created_at', c.created_at, 'updated_at', c.updated_at)";

fn parse<T: serde::de::DeserializeOwned>(b: &[u8]) -> Result<T, AppError> {
    serde_json::from_slice(b).map_err(|_| AppError::invalid("/", "INVALID", "請求內容格式錯誤"))
}

fn json_resp(status: u16, body: &Value, replay: bool) -> Response {
    let mut b = Response::builder().status(StatusCode::from_u16(status).unwrap())
        .header(header::CACHE_CONTROL, "private, no-store").header(header::CONTENT_TYPE, "application/json");
    if status == 201 {
        if let Some(id) = body["contribution"]["id"].as_str() { b = b.header(header::LOCATION, format!("/api/v1/contributions/{id}")); }
    }
    if replay { b = b.header("Idempotency-Replayed", HeaderValue::from_static("true")); }
    b.body(axum::body::Body::from(body.to_string())).unwrap()
}

async fn audit(tx: &mut Tx<'_>, actor: Option<Uuid>, action: &str, id: Uuid, diff: Value) -> Result<(), AppError> {
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff) VALUES ($1::actor_type, $2, $3, 'contributions', $4, $5)")
        .bind(if actor.is_some() { "user" } else { "system" }).bind(actor).bind(action).bind(id).bind(diff).execute(&mut **tx).await?;
    Ok(())
}

async fn contribution_json(tx: &mut Tx<'_>, id: Uuid) -> Result<Value, AppError> {
    Ok(sqlx::query_scalar(&format!("SELECT {C_JSON} FROM contributions c WHERE c.id = $1")).bind(id).fetch_one(&mut **tx).await?)
}

async fn item_funding(tx: &mut Tx<'_>, id: Uuid) -> Result<Value, AppError> {
    let (target, pledged, fs): (i64, i64, String) = sqlx::query_as(
        "SELECT target_points, pledged_points, funding_status::text FROM wishlist_items WHERE id = $1").bind(id).fetch_one(&mut **tx).await?;
    Ok(json!({ "id": id, "target_points": target, "pledged_points": pledged, "remaining_points": target - pledged,
               "funding_status": fs, "progress_percent": completion_pct(pledged, target) }))
}

async fn balance(tx: &mut Tx<'_>, wallet: Uuid) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar("SELECT balance FROM point_wallets WHERE id = $1").bind(wallet).fetch_one(&mut **tx).await?)
}

/// 達標後續：全部 pledged → captured、建採購單、通知所有捐贈者
async fn on_funded(tx: &mut Tx<'_>, item_id: Uuid, wishlist_id: Uuid) -> Result<(), AppError> {
    points::capture_and_order(tx, item_id).await?;
    notify::enqueue_donors(&mut **tx, item_id, "crowdfund.funded", &["captured"]).await?;
    audit(tx, None, "item.funded", item_id, json!({ "wishlist_id": wishlist_id })).await?;
    Ok(())
}

// ---------- POST /items/{id}/contributions ----------

#[derive(Deserialize)]
struct CreateReq { points: i64, message: Option<String>, is_anonymous: Option<bool> }

async fn create(State(st): State<AppState>, Path(item_id): Path<Uuid>, user: CurrentUser, headers: HeaderMap, body: Bytes) -> Result<Response, AppError> {
    let key = idempotency::key(&headers)?;
    let req: CreateReq = parse(&body)?;
    if !(1..=10_000_000).contains(&req.points) { return Err(AppError::invalid("/points", "RANGE", "points 必須介於 1 與 10000000")); }
    let message = match req.message.as_deref() {
        Some(m) => Some(crate::validate::text(m, "/message", true)?).filter(|m| !m.is_empty()),
        None => None,
    };
    if message.as_deref().is_some_and(|m| m.chars().count() > 200) { return Err(AppError::invalid("/message", "RANGE", "留言至多 200 字")); }
    let anon = req.is_anonymous.unwrap_or(false);

    let scope = format!("user:{}:POST /items/{{id}}/contributions", user.id);
    let hash = idempotency::request_hash(&format!("POST /items/{item_id}/contributions"), &body);
    let mut tx = st.pool.begin().await?;
    if let idempotency::Begin::Replay(s, b) = idempotency::begin(&mut tx, &scope, key, &hash).await? {
        return Ok(json_resp(s as u16, &b, true));
    }
    // 限流在重播判斷之後；超限 rollback 連計數一併撤銷（同 claims）
    ratelimit::check(&mut *tx, &format!("pledge:{}", user.id), 30, 60).await?;

    // ⓪ 清單共享鎖
    let wl: Option<Uuid> = sqlx::query_scalar("SELECT wishlist_id FROM wishlist_items WHERE id = $1 AND deleted_at IS NULL").bind(item_id).fetch_optional(&mut *tx).await?;
    let locked: Option<Uuid> = match wl {
        Some(w) => sqlx::query_scalar(
            "SELECT id FROM wishlists WHERE id = $1 AND status = 'active' AND deleted_at IS NULL AND moderation_status = 'ok' AND visibility <> 'private' FOR SHARE")
            .bind(w).fetch_optional(&mut *tx).await?,
        None => None,
    };
    let Some(wl) = locked else { tx.rollback().await?; return Err(diagnose(&st.pool, item_id, req.points).await?.unwrap_or(AppError::NotFound)); };

    // ① 錢包 FOR UPDATE；凍結 → 403
    let wallet = points::ensure_wallet(&mut tx, user.id).await?;
    let wstatus: String = sqlx::query_scalar("SELECT status::text FROM point_wallets WHERE id = $1 FOR UPDATE").bind(wallet).fetch_one(&mut *tx).await?;
    if wstatus == "frozen" { return Err(AppError::problem(403, "WALLET_FROZEN", "錢包已凍結，無法認捐，請聯絡客服")); }

    // ② 條件式 UPDATE 品項（硬上限；達標同句改 funded）
    let upd: Option<String> = sqlx::query_scalar(
        "UPDATE wishlist_items i SET pledged_points = i.pledged_points + $2,
           funding_status = CASE WHEN i.pledged_points + $2 = i.target_points THEN 'funded'::funding_status ELSE i.funding_status END
         WHERE i.id = $1 AND i.deleted_at IS NULL AND i.funding_mode = 'crowdfund' AND i.funding_status = 'open'
           AND i.funding_deadline > now() AND i.pledged_points + $2 <= i.target_points
         RETURNING i.funding_status::text")
        .bind(item_id).bind(req.points).fetch_optional(&mut *tx).await?;
    let Some(fs) = upd else { tx.rollback().await?; return Err(diagnose(&st.pool, item_id, req.points).await?.unwrap_or(AppError::NotFound)); };

    // ③ 認捐紀錄 ④ 扣點（post 於餘額不足回 409 INSUFFICIENT_POINTS，整筆 rollback）
    let donor: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1").bind(user.id).fetch_one(&mut *tx).await?;
    let cid: Uuid = sqlx::query_scalar(
        "INSERT INTO contributions (item_id, wishlist_id, user_id, wallet_id, donor_name, points, message, is_anonymous)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id")
        .bind(item_id).bind(wl).bind(user.id).bind(wallet).bind(&donor).bind(req.points).bind(&message).bind(anon).fetch_one(&mut *tx).await?;
    let bal = points::post(&mut tx, wallet, -req.points, Entry::contribution("pledge", cid)).await?;

    // ⑤ 剛好達標
    let funded = fs == "funded";
    if funded { on_funded(&mut tx, item_id, wl).await?; }
    audit(&mut tx, Some(user.id), "contribution.create", cid, json!({ "item_id": item_id, "points": req.points, "funded": funded })).await?;
    let resp = json!({ "contribution": contribution_json(&mut tx, cid).await?, "item": item_funding(&mut tx, item_id).await?,
                       "wallet": { "balance": bal }, "funded": funded });
    idempotency::finish(&mut tx, &scope, key, 201, &resp).await?;
    crate::dashboard::notify(&mut *tx, wl).await?;
    tx.commit().await?;
    Ok(json_resp(201, &resp, false))
}

/// 失敗歸因（rollback 後查詢）。None = 品項本身可接受此額度（呼叫端自行處理）
async fn diagnose(pool: &PgPool, item_id: Uuid, want: i64) -> Result<Option<AppError>, AppError> {
    let r: Option<(bool, String, String, String, String, Option<String>, i64, i64, bool)> = sqlx::query_as(
        "SELECT (i.deleted_at IS NOT NULL OR w.deleted_at IS NOT NULL), w.status::text, w.visibility::text, w.moderation_status::text,
                i.funding_mode::text, i.funding_status::text, COALESCE(i.target_points, 0), i.pledged_points,
                COALESCE(i.funding_deadline <= now(), false)
         FROM wishlist_items i JOIN wishlists w ON w.id = i.wishlist_id WHERE i.id = $1")
        .bind(item_id).fetch_optional(pool).await?;
    Ok(Some(match r {
        None | Some((true, ..)) => AppError::NotFound,
        Some((_, _, _, m, ..)) if m == "hidden" => AppError::WishlistRemoved,
        Some((_, s, v, ..)) if s == "draft" || v == "private" => AppError::NotFound,
        Some((_, s, ..)) if s != "active" => AppError::problem(409, "WISHLIST_CLOSED", "此清單已結束，不再接受認捐"),
        Some((_, _, _, _, mode, ..)) if mode != "crowdfund" => AppError::problem(409, "FUNDING_MODE_MISMATCH", "此品項不是點數眾籌"),
        Some((_, _, _, _, _, Some(fs), ..)) if fs == "funded" || fs == "fulfilled" => AppError::problem(409, "ITEM_FUNDED", "此品項已達標"),
        Some((_, _, _, _, _, fs, _, _, past)) if fs.as_deref() == Some("expired") || past => AppError::problem(409, "FUNDING_EXPIRED", "此品項的眾籌已截止"),
        Some((_, _, _, _, _, _, target, pledged, _)) if pledged + want > target => {
            let remaining = target - pledged;
            AppError::Extra { status: 409, code: "CROWDFUND_TARGET_EXCEEDED", detail: format!("此品項還差 {remaining} 點，無法認捐 {want} 點。"),
                extra: json!({ "remaining_points": remaining }) }
        }
        _ => return Ok(None),
    }))
}

// ---------- DELETE /contributions/{id} ----------

fn locked() -> AppError { AppError::problem(409, "CONTRIBUTION_LOCKED", "此筆認捐已達標或已過選擇期，無法撤回") }

async fn withdraw(State(st): State<AppState>, Path(id): Path<Uuid>, user: CurrentUser) -> Result<Response, AppError> {
    let mut tx = st.pool.begin().await?;
    let c: Option<(Uuid, Uuid, Uuid, Uuid)> = sqlx::query_as("SELECT user_id, item_id, wishlist_id, wallet_id FROM contributions WHERE id = $1")
        .bind(id).fetch_optional(&mut *tx).await?;
    let (owner, item_id, wl, wallet) = c.ok_or(AppError::NotFound)?;
    if owner != user.id { return Err(AppError::problem(403, "FORBIDDEN", "只有認捐者本人可以撤回")); }
    // 鎖序：清單 → 錢包 → 品項 → 認捐
    sqlx::query("SELECT id FROM wishlists WHERE id = $1 FOR SHARE").bind(wl).execute(&mut *tx).await?;
    points::lock_wallets(&mut tx, &[wallet]).await?;
    let (can,): (bool,) = sqlx::query_as(
        "SELECT funding_status = 'open' OR (funding_status = 'expired' AND now() < expired_at + interval '7 days')
           FROM wishlist_items WHERE id = $1 FOR UPDATE").bind(item_id).fetch_one(&mut *tx).await?;
    let released = if can { points::release(&mut tx, &[id], &["pledged"]).await? } else { vec![] };
    let Some(r) = released.first() else {
        // 已 released → 冪等回 200；其餘（captured / reallocated / 過選擇期）→ 409
        let cur: String = sqlx::query_scalar("SELECT status::text FROM contributions WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
        if cur != "released" { return Err(locked()); }
        let resp = json!({ "contribution": contribution_json(&mut tx, id).await?, "item": item_funding(&mut tx, item_id).await?,
                           "wallet": { "balance": balance(&mut tx, wallet).await? } });
        return Ok(json_resp(200, &resp, false));
    };
    // 品項 pledged_points 扣回的是整筆 points（pledged 狀態下 refunded_points 恆為 0）
    sqlx::query("UPDATE wishlist_items SET pledged_points = pledged_points - $2 WHERE id = $1").bind(item_id).bind(r.back).execute(&mut *tx).await?;
    audit(&mut tx, Some(user.id), "contribution.withdraw", id, json!({ "item_id": item_id, "points": r.back })).await?;
    crate::dashboard::notify(&mut *tx, wl).await?;
    let resp = json!({ "contribution": contribution_json(&mut tx, id).await?, "item": item_funding(&mut tx, item_id).await?,
                       "wallet": { "balance": balance(&mut tx, wallet).await? } });
    tx.commit().await?;
    Ok(json_resp(200, &resp, false))
}

// ---------- POST /contributions/{id}/reallocate ----------

#[derive(Deserialize)]
struct ReallocReq { target_item_id: Uuid }

fn not_allowed() -> AppError { AppError::problem(409, "REALLOCATION_NOT_ALLOWED", "無法轉投此品項（來源須為截止後選擇期內，目標須為同清單、未截止且額度足夠的眾籌品項）") }

async fn reallocate(State(st): State<AppState>, Path(id): Path<Uuid>, user: CurrentUser, headers: HeaderMap, body: Bytes) -> Result<Response, AppError> {
    let key = idempotency::key(&headers)?;
    let req: ReallocReq = parse(&body)?;
    let scope = format!("user:{}:POST /contributions/{{id}}/reallocate", user.id);
    let hash = idempotency::request_hash(&format!("POST /contributions/{id}/reallocate"), &body);
    let mut tx = st.pool.begin().await?;
    if let idempotency::Begin::Replay(s, b) = idempotency::begin(&mut tx, &scope, key, &hash).await? { return Ok(json_resp(s as u16, &b, true)); }
    ratelimit::check(&mut *tx, &format!("pledge:{}", user.id), 30, 60).await?;

    let c: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as("SELECT user_id, item_id, wishlist_id FROM contributions WHERE id = $1").bind(id).fetch_optional(&mut *tx).await?;
    let (owner, src, wl) = c.ok_or(AppError::NotFound)?;
    if owner != user.id { return Err(AppError::problem(403, "FORBIDDEN", "只有認捐者本人可以轉投")); }
    let tgt = req.target_item_id;
    if tgt == src { return Err(not_allowed()); }
    let wl_active: Option<String> = sqlx::query_scalar("SELECT status::text FROM wishlists WHERE id = $1 AND deleted_at IS NULL FOR SHARE").bind(wl).fetch_optional(&mut *tx).await?;
    if wl_active.as_deref() != Some("active") { return Err(not_allowed()); }
    // 兩品項依 id 排序鎖定，再鎖認捐
    sqlx::query("SELECT id FROM wishlist_items WHERE id IN ($1, $2) ORDER BY id FOR UPDATE").bind(src).bind(tgt).execute(&mut *tx).await?;
    let pts: Option<i64> = sqlx::query_scalar("SELECT points FROM contributions WHERE id = $1 AND status = 'pledged' FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
    let pts = pts.ok_or_else(not_allowed)?;
    let upd: Option<String> = sqlx::query_scalar(
        "UPDATE wishlist_items t SET pledged_points = t.pledged_points + $3,
           funding_status = CASE WHEN t.pledged_points + $3 = t.target_points THEN 'funded'::funding_status ELSE t.funding_status END
          FROM wishlist_items s
         WHERE t.id = $2 AND s.id = $1 AND s.wishlist_id = t.wishlist_id AND s.id <> t.id AND t.deleted_at IS NULL
           AND s.funding_status = 'expired' AND s.expired_at + interval '7 days' > now()
           AND t.funding_mode = 'crowdfund' AND t.funding_status = 'open' AND t.funding_deadline > now()
           AND t.pledged_points + $3 <= t.target_points
         RETURNING t.funding_status::text")
        .bind(src).bind(tgt).bind(pts).fetch_optional(&mut *tx).await?;
    let fs = upd.ok_or_else(not_allowed)?;
    sqlx::query("UPDATE wishlist_items SET pledged_points = pledged_points - $2 WHERE id = $1").bind(src).bind(pts).execute(&mut *tx).await?;
    sqlx::query("UPDATE contributions SET status = 'reallocated', reallocated_to_item_id = $2 WHERE id = $1").bind(id).bind(tgt).execute(&mut *tx).await?;
    let new_id: Uuid = sqlx::query_scalar(
        "INSERT INTO contributions (item_id, wishlist_id, user_id, wallet_id, donor_name, points, message, is_anonymous)
         SELECT $2, wishlist_id, user_id, wallet_id, donor_name, points, message, is_anonymous FROM contributions WHERE id = $1 RETURNING id")
        .bind(id).bind(tgt).fetch_one(&mut *tx).await?;
    let funded = fs == "funded";
    if funded { on_funded(&mut tx, tgt, wl).await?; }
    audit(&mut tx, Some(user.id), "contribution.reallocate", id, json!({ "from_item_id": src, "to_item_id": tgt, "new_contribution_id": new_id, "points": pts })).await?;
    let resp = json!({ "original": contribution_json(&mut tx, id).await?, "contribution": contribution_json(&mut tx, new_id).await?,
                       "item": item_funding(&mut tx, tgt).await?, "funded": funded });
    idempotency::finish(&mut tx, &scope, key, 201, &resp).await?;
    crate::dashboard::notify(&mut *tx, wl).await?;
    tx.commit().await?;
    Ok(json_resp(201, &resp, false))
}

// ---------- 背景 job ----------

/// 4.3d：(1) open 且過截止 → expired；(2) expired 滿 7 天的 pledged → 退點。冪等；多實例同時執行安全
/// （每一步都在列鎖下重新檢查條件，已處理者條件不再成立）。回傳 (轉 expired 的品項數, 自動退點的認捐數)。
pub async fn tick_funding(pool: &PgPool) -> Result<(usize, usize), AppError> {
    let mut tx = pool.begin().await?;
    let due: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "UPDATE wishlist_items SET funding_status = 'expired', expired_at = now()
         WHERE id IN (SELECT id FROM wishlist_items WHERE funding_mode = 'crowdfund' AND funding_status = 'open' AND funding_deadline <= now() AND deleted_at IS NULL
                      ORDER BY id FOR UPDATE SKIP LOCKED)
           AND funding_status = 'open'
         RETURNING id, wishlist_id").fetch_all(&mut *tx).await?;
    for (item, wl) in &due {
        notify::enqueue_donors(&mut *tx, *item, "funding.expired", &["pledged"]).await?;
        audit(&mut tx, None, "item.funding_expired", *item, json!({ "wishlist_id": wl })).await?;
        crate::dashboard::notify(&mut *tx, *wl).await?;
    }
    tx.commit().await?;

    let items: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM wishlist_items WHERE funding_status = 'expired' AND expired_at + interval '7 days' <= now() AND pledged_points > 0").fetch_all(pool).await?;
    let mut released = 0;
    for item in items {
        let mut tx = pool.begin().await?;
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as("SELECT id, wallet_id FROM contributions WHERE item_id = $1 AND status = 'pledged'").bind(item).fetch_all(&mut *tx).await?;
        let (ids, mut wallets): (Vec<Uuid>, Vec<Uuid>) = rows.into_iter().unzip();
        wallets.sort(); wallets.dedup();
        points::lock_wallets(&mut tx, &wallets).await?;   // 錢包先、品項後
        let row: Option<Uuid> = sqlx::query_scalar(
            "SELECT wishlist_id FROM wishlist_items WHERE id = $1 AND funding_status = 'expired' AND expired_at + interval '7 days' <= now() FOR UPDATE")
            .bind(item).fetch_optional(&mut *tx).await?;
        let Some(wl) = row else { continue };
        let rel = points::release(&mut tx, &ids, &["pledged"]).await?;
        if rel.is_empty() { continue; }
        let back: i64 = rel.iter().map(|r| r.back).sum();
        sqlx::query("UPDATE wishlist_items SET pledged_points = pledged_points - $2 WHERE id = $1").bind(item).bind(back).execute(&mut *tx).await?;
        for r in &rel { audit(&mut tx, None, "contribution.auto_release", r.contribution_id, json!({ "item_id": item, "points": r.back })).await?; }
        crate::dashboard::notify(&mut *tx, wl).await?;
        tx.commit().await?;
        released += rel.len();
    }
    Ok((due.len(), released))
}
