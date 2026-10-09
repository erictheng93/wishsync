//! 公開頁 SSE 即時更新。（擁有者儀表板 GET /wishlists/{id}/dashboard 由 wishlists.rs 實作。）
//!
//! SSE 機制：認領 / 清單變更後呼叫 `dashboard::notify(executor, wishlist_id)`（Postgres
//! `pg_notify('wishlist_events', id)`，在交易內呼叫則 commit 後才送出）。每個 process 只開一條
//! PgListener（首位訂閱者出現時惰性啟動），轉成 tokio broadcast；各 SSE 連線收到自己清單的 id
//! 就重新查詢並只推「有變動」的品項 + wishlist.updated。事件只含數量彙總，不含認領者。
use crate::{error::AppError, AppState};
use axum::{
    extract::{Path, State},
    http::{header, HeaderValue},
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

struct Snap { status: String, title: String, hidden: bool, items: HashMap<Uuid, (i32, i32)> }

async fn snapshot(pool: &PgPool, wid: Uuid) -> Result<Option<Snap>, sqlx::Error> {
    let w: Option<(String, String, String)> = sqlx::query_as(
        "SELECT status::text, title, moderation_status::text FROM wishlists WHERE id = $1 AND deleted_at IS NULL").bind(wid).fetch_optional(pool).await?;
    let Some((status, title, m)) = w else { return Ok(None) };
    let items: Vec<(Uuid, i32, i32)> = sqlx::query_as(
        "SELECT id, qty_needed, qty_claimed FROM wishlist_items WHERE wishlist_id = $1 AND deleted_at IS NULL").bind(wid).fetch_all(pool).await?;
    Ok(Some(Snap { status, title, hidden: m == "hidden", items: items.into_iter().map(|i| (i.0, (i.1, i.2))).collect() }))
}

fn item_event(id: Uuid, needed: i32, claimed: i32, deleted: bool) -> Value {
    let mut v = json!({ "item_id": id, "qty_needed": needed, "qty_claimed": claimed, "qty_remaining": (needed - claimed).max(0),
        "is_fully_claimed": claimed >= needed, "pledged_points": null, "target_points": null, "funding_status": null,
        "display_status": null, "updated_at": chrono::Utc::now() });
    if deleted { v["deleted"] = json!(true); }
    v
}

async fn events(State(st): State<AppState>, Path(slug): Path<String>) -> Result<Response, AppError> {
    let w: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, moderation_status::text FROM wishlists
          WHERE slug = $1 AND deleted_at IS NULL AND visibility <> 'private' AND status IN ('active', 'closed')")
        .bind(&slug).fetch_optional(&st.pool).await?;
    let (wid, m) = w.ok_or(AppError::NotFound)?;
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
                for (id, &(n, c)) in &s.items { if seen.get(id) != Some(&(n, c)) { evs.push(("item.updated", item_event(*id, n, c, false))); } }
                for (id, &(n, c)) in seen.iter() { if !s.items.contains_key(id) { evs.push(("item.updated", item_event(*id, n, c, true))); } }
            }
            let fulfilled = s.items.values().filter(|i| i.1 >= i.0).count();
            evs.push(("wishlist.updated", json!({ "status": s.status, "title": s.title,
                "completion": { "item_count": s.items.len(), "fulfilled_count": fulfilled, "completion_pct": crate::wishlists::completion_pct(s.items.values().map(|i| i.1 as i64).sum(), s.items.values().map(|i| i.0 as i64).sum()) },
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
