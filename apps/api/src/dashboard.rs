//! 公開頁 SSE 即時更新。（擁有者儀表板 GET /wishlists/{id}/dashboard 由 wishlists.rs 實作。）
//!
//! SSE 機制：認領 / 清單變更後呼叫 `dashboard::notify(executor, wishlist_id)`（Postgres
//! `pg_notify('wishlist_events', id)`，在交易內呼叫則 commit 後才送出）。每個 process 只開一條
//! PgListener（首位訂閱者出現時惰性啟動），轉成 tokio broadcast；各 SSE 連線收到自己清單的 id
//! 就重新查詢並只推「有變動」的品項 + wishlist.updated。事件只含數量彙總，不含認領者。
use crate::{error::AppError, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{header, request::Parts, HeaderValue},
    response::{sse::{Event, KeepAlive, Sse}, IntoResponse, Response},
    routing::get,
    Router,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::{collections::HashMap, convert::Infallible, time::Duration};
use tokio::sync::{broadcast, mpsc, OnceCell};
use uuid::Uuid;

const CHANNEL: &str = "wishlist_events";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/public/wishlists/{slug}/events", get(events))
}

/// 給 claims / wishlists 模組呼叫：通知 SSE 訂閱者此清單有變動。
pub async fn notify<'e, E: sqlx::PgExecutor<'e>>(ex: E, wishlist_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_notify($1, $2)").bind(CHANNEL).bind(wishlist_id.to_string()).execute(ex).await?;
    Ok(())
}


// ---------- SSE ----------

static BUS: OnceCell<broadcast::Sender<Uuid>> = OnceCell::const_new();

async fn bus(pool: &PgPool) -> &'static broadcast::Sender<Uuid> {
    BUS.get_or_init(|| async {
        let (tx, _) = broadcast::channel(256);
        let (pool, t) = (pool.clone(), tx.clone());
        tokio::spawn(async move {
            loop {
                if let Ok(mut l) = sqlx::postgres::PgListener::connect_with(&pool).await {
                    if l.listen(CHANNEL).await.is_ok() {
                        while let Ok(n) = l.recv().await { if let Ok(id) = n.payload().parse() { let _ = t.send(id); } }
                    }
                }
                tokio::time::sleep(Duration::from_secs(2)).await; // 斷線重連
            }
        });
        tx
    }).await
}

/// 單一品項的 SSE 比對單位：任何一項變動就推 item.updated（pledged_points / funding_status / display_status 為眾籌欄位）
#[derive(Clone, PartialEq)]
struct ItemSnap { needed: i32, claimed: i32, mode: String, pledged: i64, target: Option<i64>, funding_status: Option<String>, display_status: Option<String> }

struct Snap { status: String, title: String, hidden: bool, items: HashMap<Uuid, ItemSnap> }

async fn snapshot(pool: &PgPool, wid: Uuid) -> Result<Option<Snap>, sqlx::Error> {
    let w: Option<(String, String, String)> = sqlx::query_as(
        "SELECT status::text, title, moderation_status::text FROM wishlists WHERE id = $1 AND deleted_at IS NULL").bind(wid).fetch_optional(pool).await?;
    let Some((status, title, m)) = w else { return Ok(None) };
    let items: Vec<(Uuid, i32, i32, String, i64, Option<i64>, Option<String>, Option<String>)> = sqlx::query_as(
        &format!("SELECT i.id, i.qty_needed, i.qty_claimed, i.funding_mode::text, i.pledged_points, i.target_points, i.funding_status::text, ({}) AS display_status
                    FROM wishlist_items i LEFT JOIN purchase_orders po ON po.item_id = i.id WHERE i.wishlist_id = $1 AND i.deleted_at IS NULL", crate::points::DISPLAY_STATUS_SQL))
        .bind(wid).fetch_all(pool).await?;
    Ok(Some(Snap { status, title, hidden: m == "hidden", items: items.into_iter().map(|i|
        (i.0, ItemSnap { needed: i.1, claimed: i.2, mode: i.3, pledged: i.4, target: i.5, funding_status: i.6, display_status: i.7 })).collect() }))
}

fn units(s: &ItemSnap) -> (i64, i64) { crate::wishlists::units(&s.mode, s.needed, s.claimed, s.funding_status.as_deref()) }

fn item_event(id: Uuid, s: &ItemSnap, deleted: bool) -> Value {
    let cf = s.mode == "crowdfund";
    let (needed, claimed) = (s.needed, s.claimed);
    let mut v = json!({ "item_id": id, "qty_needed": needed, "qty_claimed": claimed, "qty_remaining": (needed - claimed).max(0),
        "is_fully_claimed": if cf { units(s).0 >= 1 } else { claimed >= needed },
        "pledged_points": cf.then_some(s.pledged), "target_points": s.target, "remaining_points": s.target.filter(|_| cf).map(|t| (t - s.pledged).max(0)),
        "funding_status": s.funding_status, "display_status": s.display_status, "updated_at": chrono::Utc::now() });
    if deleted { v["deleted"] = json!(true); }
    v
}

#[derive(serde::Deserialize)]
struct EventsQ { access: Option<String> }

async fn events(State(st): State<AppState>, Path(slug): Path<String>, Query(q): Query<EventsQ>, parts: Parts) -> Result<Response, AppError> {
    let w: Option<(Uuid, String, Uuid, String, Option<String>)> = sqlx::query_as(
        "SELECT id, moderation_status::text, owner_id, visibility::text, access_password_hash FROM wishlists
          WHERE slug = $1 AND deleted_at IS NULL AND status IN ('active', 'closed')")
        .bind(&slug).fetch_optional(&st.pool).await?;
    let (wid, m, owner_id, vis, pw) = w.ok_or(AppError::NotFound)?;
    let viewer = crate::access::viewer(&parts, &st).await?;
    crate::access::check_wishlist(&st.pool, &crate::access::ListAccess { wishlist_id: wid, owner_id, visibility: &vis, pw_hash: pw.as_deref() }, viewer, q.access.as_deref()).await?;
    if m == "hidden" { return Err(AppError::WishlistRemoved); }
    let mut rx = bus(&st.pool).await.subscribe(); // 先訂閱再取快照，避免漏事件
    let mut seen = snapshot(&st.pool, wid).await?.map(|s| s.items).unwrap_or_default();
    let (tx, out) = mpsc::channel::<Result<Event, Infallible>>(16);
    let pool = st.pool.clone();
    tokio::spawn(async move {
        let deadline = tokio::time::sleep(Duration::from_secs(30 * 60)); // 每條連線最長 30 分鐘
        tokio::pin!(deadline);
        let mut seq = 0u64;
        loop {
            tokio::select! {
                _ = &mut deadline => break,
                _ = tx.closed() => break,
                r = rx.recv() => match r {
                    Ok(id) if id != wid => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                    _ => {}
                },
            }
            let Ok(Some(s)) = snapshot(&pool, wid).await else { continue };
            let mut evs: Vec<(&str, Value)> = vec![];
            if !s.hidden {
                for (id, cur) in &s.items { if seen.get(id) != Some(cur) { evs.push(("item.updated", item_event(*id, cur, false))); } }
                for (id, old) in seen.iter() { if !s.items.contains_key(id) { evs.push(("item.updated", item_event(*id, old, true))); } }
            }
            let fulfilled = s.items.values().filter(|i| { let (c, n) = units(i); c >= n }).count();
            evs.push(("wishlist.updated", json!({ "status": s.status, "title": s.title,
                "completion": { "item_count": s.items.len(), "fulfilled_count": fulfilled, "completion_pct": crate::wishlists::completion_pct(s.items.values().map(|i| units(i).0).sum(), s.items.values().map(|i| units(i).1).sum()) },
                "updated_at": chrono::Utc::now() })));
            seen = s.items;
            for (name, data) in evs {
                seq += 1;
                let ev = Event::default().id(format!("{}-{seq}", chrono::Utc::now().timestamp())).event(name).data(data.to_string());
                if tx.send(Ok(ev)).await.is_err() { return; }
            }
            if s.hidden { break; } // 下架：廣播後關閉連線
        }
    });
    // 連線一建立就先送一個 retry 事件：代理（Cloudflare Pages 同源代理等）會緩衝到第一個位元組才轉送，
    // 沒有它，要等 15 秒的第一次心跳瀏覽器才收得到回應標頭。
    let first = futures_util::stream::once(async { Ok::<_, std::convert::Infallible>(Event::default().retry(Duration::from_secs(5))) });
    let rest = futures_util::stream::unfold(out, |mut rx| async move { rx.recv().await.map(|e| (e, rx)) });
    let stream = futures_util::StreamExt::chain(first, rest);
    let mut res = Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("heartbeat")).into_response();
    let h = res.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache, no-transform"));
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    Ok(res)
}
