//! 認領：POST /items/{id}/claims、PATCH/DELETE /claims/{id}（契約 4.1 / 4.2、F4 / F5）。
//! 鎖序：wishlist_items（條件式 UPDATE / FOR UPDATE）→ claims。
use crate::{error::AppError, guest::{self, Actor, MaybeActor}, idempotency, ratelimit, AppState};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{patch, post},
    Router,
};
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/items/{id}/claims", post(create))
        .route("/claims/{id}", patch(update).delete(cancel))
}

/// claims 以別名 `c` 查詢
pub const CLAIM_COLS: &str = "c.id, c.item_id, c.qty, c.status::text AS status, c.claimer_name, c.note, c.expires_at,
    c.created_at, c.updated_at, c.purchased_at, c.delivered_at, c.cancelled_at";

#[derive(FromRow, Serialize)]
pub struct ClaimRow {
    pub id: Uuid, pub item_id: Uuid, pub qty: i32, pub status: String, pub claimer_name: String, pub note: Option<String>,
    pub expires_at: Option<DateTime<Utc>>, pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc>,
    pub purchased_at: Option<DateTime<Utc>>, pub delivered_at: Option<DateTime<Utc>>, pub cancelled_at: Option<DateTime<Utc>>,
}

fn parse<T: DeserializeOwned>(b: &[u8]) -> Result<T, AppError> {
    serde_json::from_slice(b).map_err(|_| AppError::invalid("/", "INVALID", "請求內容格式錯誤"))
}

fn no_store(status: StatusCode) -> axum::http::response::Builder {
    Response::builder().status(status).header(header::CACHE_CONTROL, "private, no-store")
}

fn json_resp(status: i32, body: &Value, replay: bool) -> Response {
    let mut b = no_store(StatusCode::from_u16(status as u16).unwrap()).header(header::CONTENT_TYPE, "application/json");
    if status == 201 {
        if let Some(id) = body["claim"]["id"].as_str() { b = b.header(header::LOCATION, format!("/api/v1/claims/{id}")); }
    }
    if let Some(t) = body["guest_token"].as_str() { b = b.header(header::SET_COOKIE, guest::cookie(t)); }
    if replay { b = b.header("Idempotency-Replayed", HeaderValue::from_static("true")); }
    b.body(axum::body::Body::from(body.to_string())).unwrap()
}

async fn audit(tx: &mut Transaction<'_, Postgres>, actor: Actor, action: &str, claim: Uuid, diff: Value) -> Result<(), AppError> {
    let (t, id) = match actor { Actor::Guest(g) => ("guest", g), Actor::User(u) => ("user", u) };
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff) VALUES ($1::actor_type, $2, $3, 'claims', $4, $5)")
        .bind(t).bind(id).bind(action).bind(claim).bind(diff).execute(&mut **tx).await?;
    Ok(())
}

fn item_json(id: Uuid, needed: i32, claimed: i32) -> Value {
    json!({ "id": id, "qty_needed": needed, "qty_claimed": claimed, "qty_remaining": needed - claimed })
}

fn full(remaining: i32, want: i32) -> AppError {
    AppError::Extra { status: 409, code: "ITEM_FULLY_CLAIMED", detail: format!("此品項剩餘 {remaining} 件，無法再認領 {want} 件。"),
        extra: json!({ "remaining": remaining }) }
}

// ---------- POST /items/{id}/claims ----------

#[derive(Deserialize)]
struct CreateReq { qty: i32, display_name: Option<String>, contact: Option<String>, email: Option<String>, note: Option<String> }

async fn create(State(st): State<AppState>, Path(item_id): Path<Uuid>, MaybeActor(actor): MaybeActor, peer: ratelimit::Peer, headers: HeaderMap, body: Bytes)
    -> Result<Response, AppError> {
    let key = idempotency::key(&headers)?;
    let req: CreateReq = parse(&body)?;
    if !(1..=99).contains(&req.qty) { return Err(AppError::invalid("/qty", "RANGE", "qty 必須介於 1 與 99")); }
    if req.note.as_deref().is_some_and(|n| n.chars().count() > 200) { return Err(AppError::invalid("/note", "RANGE", "備註至多 200 字")); }
    if req.contact.as_deref().is_some_and(|n| n.chars().count() > 100) { return Err(AppError::invalid("/contact", "RANGE", "聯絡方式至多 100 字")); }
    let email = req.email.as_deref().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty());
    if email.as_deref().is_some_and(|e| !e.contains('@') || e.len() > 200) { return Err(AppError::invalid("/email", "FORMAT", "Email 格式不正確")); }
    let dn = req.display_name.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if dn.is_some_and(|s| s.chars().count() > 30) { return Err(AppError::invalid("/display_name", "RANGE", "暱稱需為 1–30 字")); }

    let route = "POST /items/{id}/claims";
    let scope = match actor {
        Some(Actor::Guest(g)) => format!("guest:{g}:{route}"),
        Some(Actor::User(u)) => format!("user:{u}:{route}"),
        None => format!("anon:{route}"),
    };
    let hash = idempotency::request_hash(&format!("POST /items/{item_id}/claims"), &body);

    let mut tx = st.pool.begin().await?;
    if let idempotency::Begin::Replay(s, b) = idempotency::begin(&mut tx, &scope, key, &hash).await? {
        return Ok(json_resp(s, &b, true));
    }
    // 限流放在重播判斷之後：重播只回放已存的回應、不產生新認領，不消耗額度（也讓網路重試不會被誤擋）。
    // 超限時 tx 隨 return 回滾，idempotency key 不會被占用，額度重置後可用同 key 重試。
    // 計數與認領同一個 tx（不另佔連線，避免併發時 pool 耗盡死結；429 時計數一併回滾，只算成功放行的請求）。
    ratelimit::check(&mut *tx, &format!("claim_post_ip:{}", peer.0), 30, 3600).await?;
    let wl: Option<Uuid> = sqlx::query_scalar("SELECT wishlist_id FROM wishlist_items WHERE id = $1").bind(item_id).fetch_optional(&mut *tx).await?;
    if let Some(w) = wl { ratelimit::check(&mut *tx, &format!("claim_post_wl:{w}:{}", peer.0), 15, 3600).await?; }

    // 身分：user / 既有 guest / 首次建立 guest（同交易，失敗一併 rollback）
    let (mut guest_id, mut user_id, mut new_token) = (None, None, None);
    let created_guest = actor.is_none();
    let name: String = match actor {
        Some(Actor::User(u)) => {
            user_id = Some(u);
            sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1").bind(u).fetch_one(&mut *tx).await?
        }
        Some(Actor::Guest(g)) => {
            guest_id = Some(g);
            sqlx::query_scalar("SELECT display_name FROM guests WHERE id = $1").bind(g).fetch_one(&mut *tx).await?
        }
        None => {
            let name = dn.ok_or_else(|| AppError::invalid("/display_name", "REQUIRED", "首次認領需填寫暱稱"))?.to_owned();
            let (tok, h) = guest::new_token();
            guest_id = Some(sqlx::query_scalar("INSERT INTO guests (guest_token_hash, display_name, contact, email) VALUES ($1, $2, $3, $4) RETURNING id")
                .bind(h).bind(&name).bind(&req.contact).bind(&email).fetch_one(&mut *tx).await?);
            new_token = Some(tok);
            name
        }
    };

    // 4.1 (b)：條件式 UPDATE 取得 item row lock 並驗證剩餘量。
    // 偏差：加 moderation_status='ok' 與 visibility<>'private'，下架/私人清單不可認領。
    let upd: Option<(i32, i32, Option<i32>)> = sqlx::query_as(
        "UPDATE wishlist_items i SET qty_claimed = i.qty_claimed + $2
         FROM wishlists w
         WHERE i.id = $1 AND w.id = i.wishlist_id
           AND w.status = 'active' AND w.deleted_at IS NULL AND w.moderation_status = 'ok' AND w.visibility <> 'private'
           AND i.deleted_at IS NULL AND i.funding_mode = 'quantity' AND i.qty_claimed + $2 <= i.qty_needed
         RETURNING i.qty_needed, i.qty_claimed, w.claim_ttl_hours")
        .bind(item_id).bind(req.qty).fetch_optional(&mut *tx).await?;
    let Some((needed, claimed, ttl)) = upd else {
        tx.rollback().await?;
        return Err(diagnose(&st, item_id, req.qty).await?);
    };

    let ins = sqlx::query_as::<_, ClaimRow>(&format!(
        "INSERT INTO claims AS c (item_id, guest_id, user_id, claimer_name, qty, note, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, CASE WHEN $7::int IS NULL THEN NULL ELSE now() + $7::int * interval '1 hour' END)
         RETURNING {CLAIM_COLS}"))
        .bind(item_id).bind(guest_id).bind(user_id).bind(&name).bind(req.qty).bind(&req.note).bind(ttl)
        .fetch_one(&mut *tx).await;
    let claim = match ins {
        Ok(c) => c,
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23505") => {
            drop(tx);
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM claims WHERE item_id = $1 AND (guest_id = $2 OR user_id = $3) AND status IN ('reserved','purchased','delivered')")
                .bind(item_id).bind(guest_id).bind(user_id).fetch_optional(&st.pool).await?;
            return Err(AppError::Extra { status: 409, code: "CLAIM_ALREADY_EXISTS", detail: "你已認領過此品項，請改用修改數量".into(),
                extra: json!({ "claim_id": id }) });
        }
        Err(e) => return Err(e.into()),
    };

    let actor = guest_id.map(Actor::Guest).or(user_id.map(Actor::User)).unwrap();
    audit(&mut tx, actor, "claim.create", claim.id, json!({ "qty": req.qty })).await?;
    let mut resp = json!({ "claim": claim, "item": item_json(item_id, needed, claimed) });
    if let Some(t) = new_token { resp["guest_token"] = json!(t); }
    // 重播不得再給明文 token：存進 idempotency_keys 的版本移除 guest_token
    let mut stored = resp.clone();
    if let Some(o) = stored.as_object_mut() { o.remove("guest_token"); }
    idempotency::finish(&mut tx, &scope, key, 201, &stored).await?;
    let wid = wishlist_of(&mut tx, item_id).await?;
    // 訪客有留 Email → 確認信（恢復權杖於寄信時才產生，明文不落庫）。
    // 濫用限流：超限只是不寄信，認領本身照常成功。
    let guest_email: Option<String> = match guest_id {
        Some(g) => sqlx::query_scalar("SELECT email FROM guests WHERE id = $1 AND deleted_at IS NULL").bind(g).fetch_one(&mut *tx).await?,
        None => None,
    };
    let send_mail = match (&guest_email, guest_id) {
        (Some(e), Some(g)) => {
            (!created_guest || ratelimit::allow(&st.pool, &format!("claim_ip:{}", peer.0), 20, 3600).await?)
                && ratelimit::allow(&st.pool, &format!("claim_mail:{e}"), 3, 3600).await?
                && ratelimit::allow(&st.pool, &format!("claim_guest:{g}"), 10, 86400).await?
        }
        _ => false,
    };
    if let (true, Some(g)) = (send_mail, guest_id) {
        sqlx::query("INSERT INTO notifications (guest_id, channel, kind, payload)
                     SELECT id, 'email', 'claim.confirmation', jsonb_build_object('wishlist_id', $2::uuid, 'claim_id', $3::uuid)
                       FROM guests WHERE id = $1 AND email IS NOT NULL AND deleted_at IS NULL")
            .bind(g).bind(wid).bind(claim.id).execute(&mut *tx).await?;
    }
    crate::notify::enqueue_claim(&mut *tx, wid).await?;
    crate::dashboard::notify(&mut *tx, wid).await?;
    tx.commit().await?;
    Ok(json_resp(201, &resp, false))
}

/// 4.1 (d)：失敗歸因（rollback 後獨立查詢）
async fn diagnose(st: &AppState, item_id: Uuid, want: i32) -> Result<AppError, AppError> {
    let r: Option<(bool, String, String, String, String, i32)> = sqlx::query_as(
        "SELECT (i.deleted_at IS NOT NULL OR w.deleted_at IS NOT NULL), w.status::text, w.visibility::text, w.moderation_status::text,
                i.funding_mode::text, i.qty_needed - i.qty_claimed
         FROM wishlist_items i JOIN wishlists w ON w.id = i.wishlist_id WHERE i.id = $1")
        .bind(item_id).fetch_optional(&st.pool).await?;
    Ok(match r {
        None | Some((true, ..)) => AppError::NotFound,
        Some((_, _, _, m, ..)) if m == "hidden" => AppError::WishlistRemoved,
        Some((_, s, v, ..)) if s == "draft" || v == "private" => AppError::NotFound,
        Some((_, s, ..)) if s != "active" => AppError::problem(409, "WISHLIST_CLOSED", "此清單已結束，不再接受認領"),
        Some((_, _, _, _, f, _)) if f != "quantity" => AppError::problem(409, "FUNDING_MODE_MISMATCH", "此品項不是數量型認領"),
        Some((.., remaining)) => full(remaining, want),
    })
}

// ---------- PATCH / DELETE /claims/{id} ----------

#[derive(Deserialize)]
struct PatchReq { qty: Option<i32>, note: Option<String>, status: Option<String> }

async fn update(State(st): State<AppState>, Path(id): Path<Uuid>, MaybeActor(actor): MaybeActor, body: Bytes) -> Result<Response, AppError> {
    let p: PatchReq = parse(&body)?;
    if p.qty.is_none() && p.note.is_none() && p.status.is_none() { return Err(AppError::invalid("/", "REQUIRED", "至少需提供 qty、note 或 status")); }
    if p.qty.is_some_and(|q| !(1..=99).contains(&q)) { return Err(AppError::invalid("/qty", "RANGE", "qty 必須介於 1 與 99")); }
    if p.note.as_deref().is_some_and(|n| n.chars().count() > 200) { return Err(AppError::invalid("/note", "RANGE", "備註至多 200 字")); }
    if p.status.as_deref().is_some_and(|s| !["purchased", "delivered", "cancelled"].contains(&s)) {
        return Err(AppError::invalid("/status", "ENUM", "status 只能是 purchased / delivered / cancelled"));
    }
    let (claim, item) = apply(&st, id, actor, p).await?;
    Ok(json_resp(200, &json!({ "claim": claim, "item": item }), false))
}

/// 已 cancelled 的重複 DELETE 回 204（冪等）
async fn cancel(State(st): State<AppState>, Path(id): Path<Uuid>, MaybeActor(actor): MaybeActor) -> Result<Response, AppError> {
    apply(&st, id, actor, PatchReq { qty: None, note: None, status: Some("cancelled".into()) }).await?;
    Ok(no_store(StatusCode::NO_CONTENT).body(axum::body::Body::empty()).unwrap().into_response())
}

async fn apply(st: &AppState, id: Uuid, actor: Option<Actor>, p: PatchReq) -> Result<(ClaimRow, Value), AppError> {
    let actor = actor.ok_or(AppError::Unauthorized)?;
    let mut tx = st.pool.begin().await?;
    // F5 交易邊界：先（不加鎖）取 item_id → 鎖 item → 鎖 claim
    let item_id: Uuid = sqlx::query_scalar("SELECT item_id FROM claims WHERE id = $1").bind(id).fetch_optional(&mut *tx).await?
        .ok_or(AppError::NotFound)?;
    let (needed, claimed, wl_status, owner_id, locked): (i32, i32, String, Uuid, bool) = sqlx::query_as(
        "SELECT i.qty_needed, i.qty_claimed, w.status::text, w.owner_id,
                (w.surprise_mode AND w.event_date IS NOT NULL AND now() < (w.event_date::timestamp AT TIME ZONE 'Asia/Taipei'))
         FROM wishlist_items i JOIN wishlists w ON w.id = i.wishlist_id
         WHERE i.id = $1 FOR UPDATE OF i").bind(item_id).fetch_one(&mut *tx).await?;
    let (guest_id, user_id, cur, old_qty): (Option<Uuid>, Option<Uuid>, String, i32) = sqlx::query_as(
        "SELECT guest_id, user_id, status::text, qty FROM claims WHERE id = $1 FOR UPDATE").bind(id).fetch_one(&mut *tx).await?;
    let mine = match actor { Actor::Guest(g) => guest_id == Some(g), Actor::User(u) => user_id == Some(u) };
    // 清單擁有者（F5）：只能推進 delivered 或取消；驚喜鎖定期間看不到認領，一律禁止
    let by_owner = !mine && matches!(actor, Actor::User(u) if u == owner_id);
    if !mine && !by_owner { return Err(AppError::problem(403, "FORBIDDEN", "只有認領者本人可以操作")); }
    if by_owner && (locked || p.qty.is_some() || p.note.is_some() || !matches!(p.status.as_deref(), Some("delivered" | "cancelled"))) {
        return Err(AppError::problem(403, "FORBIDDEN", "清單擁有者只能在驚喜解鎖後標記已送達或取消認領"));
    }

    let bad = || AppError::problem(409, "INVALID_STATE_TRANSITION", "不允許的狀態轉換");
    let mut new_status = cur.clone();
    if let Some(s) = p.status.as_deref() {
        match (cur.as_str(), s) {
            ("cancelled", "cancelled") => {} // 冪等 no-op
            ("reserved", "purchased") | ("reserved" | "purchased", "delivered") | ("reserved" | "purchased", "cancelled") => new_status = s.into(),
            _ => return Err(bad()),
        }
    }
    let cancelling = new_status == "cancelled";
    let mut new_qty = None;
    if let (Some(q), false) = (p.qty, cancelling) {
        if !["reserved", "purchased"].contains(&cur.as_str()) { return Err(bad()); }
        if wl_status != "active" { return Err(AppError::problem(409, "WISHLIST_CLOSED", "此清單已結束，無法修改數量")); }
        let delta = q - old_qty;
        if delta != 0 {
            let r: Option<(i32,)> = sqlx::query_as("UPDATE wishlist_items SET qty_claimed = qty_claimed + $2 WHERE id = $1 AND qty_claimed + $2 <= qty_needed RETURNING qty_claimed")
                .bind(item_id).bind(delta).fetch_optional(&mut *tx).await?;
            if r.is_none() { return Err(full(needed - claimed, delta)); }
            new_qty = Some(q);
        }
    }
    if cancelling && cur != "cancelled" {
        let r = sqlx::query("UPDATE wishlist_items SET qty_claimed = qty_claimed - $2 WHERE id = $1 AND qty_claimed >= $2")
            .bind(item_id).bind(old_qty).execute(&mut *tx).await?;
        if r.rows_affected() == 0 { tracing::error!(%id, "qty_claimed drift"); return Err(AppError::Db(sqlx::Error::RowNotFound)); }
    }
    let claim: ClaimRow = if new_status == cur && new_qty.is_none() && p.note.is_none() {
        sqlx::query_as(&format!("SELECT {CLAIM_COLS} FROM claims c WHERE c.id = $1")).bind(id).fetch_one(&mut *tx).await?
    } else {
        sqlx::query_as(&format!(
            "UPDATE claims c SET qty = COALESCE($2, qty), note = COALESCE($3, note), status = $4::text::claim_status,
               purchased_at = CASE WHEN $4::text = 'purchased' THEN now() ELSE purchased_at END,
               delivered_at = CASE WHEN $4::text = 'delivered' THEN now() ELSE delivered_at END,
               cancelled_at = CASE WHEN $4::text = 'cancelled' THEN now() ELSE cancelled_at END,
               expires_at = CASE WHEN $4::text <> 'reserved' THEN NULL ELSE expires_at END
             WHERE c.id = $1 RETURNING {CLAIM_COLS}"))
            .bind(id).bind(new_qty).bind(&p.note).bind(&new_status).fetch_one(&mut *tx).await?
    };
    let (n, c): (i32, i32) = sqlx::query_as("SELECT qty_needed, qty_claimed FROM wishlist_items WHERE id = $1").bind(item_id).fetch_one(&mut *tx).await?;
    if new_status != cur || new_qty.is_some() {
        let action = if by_owner { "claim.owner_update" } else { "claim.update" };
        audit(&mut tx, actor, action, id, json!({ "status": [cur, new_status], "qty": [old_qty, claim.qty] })).await?;
    }
    let wid = wishlist_of(&mut tx, item_id).await?;
    crate::dashboard::notify(&mut *tx, wid).await?;
    tx.commit().await?;
    Ok((claim, item_json(item_id, n, c)))
}

async fn wishlist_of(tx: &mut Transaction<'_, Postgres>, item_id: Uuid) -> Result<Uuid, AppError> {
    Ok(sqlx::query_scalar("SELECT wishlist_id FROM wishlist_items WHERE id = $1").bind(item_id).fetch_one(&mut **tx).await?)
}

/// FR-08：把逾期的 reserved 認領設為 expired 並回補 qty_claimed，回傳釋放的認領數。
/// 鎖序同 apply()：先鎖 item（依 id 排序）再動 claim；UPDATE ... WHERE status='reserved' 保證多實例 / 與取消競態時只回補一次。
pub async fn expire_due(pool: &sqlx::PgPool) -> Result<usize, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT id FROM wishlist_items WHERE id IN (SELECT item_id FROM claims WHERE status = 'reserved' AND expires_at <= now())
                 ORDER BY id FOR UPDATE").execute(&mut *tx).await?;
    let rows: Vec<(Uuid, i64)> = sqlx::query_as(
        "WITH x AS (
           UPDATE claims SET status = 'expired', expires_at = NULL
           WHERE status = 'reserved' AND expires_at <= now() RETURNING id, item_id, qty),
         r AS (
           UPDATE wishlist_items i SET qty_claimed = i.qty_claimed - s.q
           FROM (SELECT item_id, sum(qty)::int AS q FROM x GROUP BY item_id) s
           WHERE i.id = s.item_id RETURNING i.wishlist_id),
         a AS (
           INSERT INTO audit_logs (actor_type, action, entity, entity_id, diff)
           SELECT 'system'::actor_type, 'claim.expire', 'claims', id, jsonb_build_object('qty', qty) FROM x)
         SELECT DISTINCT wishlist_id, (SELECT count(*) FROM x) FROM r")
        .fetch_all(&mut *tx).await?;
    for (w, _) in &rows { crate::dashboard::notify(&mut *tx, *w).await?; }
    tx.commit().await?;
    Ok(rows.first().map_or(0, |r| r.1 as usize))
}
