//! 認領逾期釋放（FR-08）與認領端點限流。
use axum::{body::Body, extract::ConnectInfo, http::{Request, StatusCode}, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wishsync_api::{claims, guest, AppState};

fn app(pool: &PgPool) -> Router {
    Router::new().nest("/api/v1", claims::routes().merge(guest::routes())).with_state(AppState { pool: pool.clone() })
}

async fn call(app: &Router, mut req: Request<Body>, ip: &str) -> (StatusCode, axum::http::HeaderMap, Value) {
    req.extensions_mut().insert(ConnectInfo::<std::net::SocketAddr>(format!("{ip}:1234").parse().unwrap()));
    let res = app.clone().oneshot(req).await.unwrap();
    let (st, h) = (res.status(), res.headers().clone());
    let b = res.into_body().collect().await.unwrap().to_bytes();
    (st, h, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

async fn wishlist(pool: &PgPool, slug: &str, items: usize, needed: i32) -> (Uuid, Vec<Uuid>) {
    let owner: Uuid = sqlx::query_scalar("INSERT INTO users (display_name) VALUES ('o') RETURNING id").fetch_one(pool).await.unwrap();
    let wl: Uuid = sqlx::query_scalar("INSERT INTO wishlists (owner_id, type, status, slug, title) VALUES ($1, 'registry', 'active', $2, 't') RETURNING id")
        .bind(owner).bind(slug).fetch_one(pool).await.unwrap();
    let mut v = vec![];
    for i in 0..items {
        v.push(sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, $2, $3) RETURNING id")
            .bind(wl).bind(format!("i{i}")).bind(needed).fetch_one(pool).await.unwrap());
    }
    (wl, v)
}

fn post_claim(item: Uuid, key: Uuid) -> Request<Body> {
    Request::post(format!("/api/v1/items/{item}/claims")).header("content-type", "application/json").header("idempotency-key", key.to_string())
        .body(Body::from(json!({"qty": 1, "display_name": "x"}).to_string())).unwrap()
}

/// 直接塞認領並同步 qty_claimed；回傳 claim id
async fn seed_claim(pool: &PgPool, item: Uuid, qty: i32, status: &str, expires: &str) -> Uuid {
    let g: Uuid = sqlx::query_scalar("INSERT INTO guests (guest_token_hash, display_name) VALUES ($1, 'g') RETURNING id")
        .bind(Uuid::new_v4().as_bytes().to_vec()).fetch_one(pool).await.unwrap();
    sqlx::query("UPDATE wishlist_items SET qty_claimed = qty_claimed + $2 WHERE id = $1").bind(item).bind(qty).execute(pool).await.unwrap();
    sqlx::query_scalar(&format!("INSERT INTO claims (item_id, guest_id, claimer_name, qty, status, expires_at) VALUES ($1, $2, 'g', $3, $4::claim_status, {expires}) RETURNING id"))
        .bind(item).bind(g).bind(qty).bind(status).fetch_one(pool).await.unwrap()
}

async fn qty(pool: &PgPool, item: Uuid) -> i32 {
    sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id = $1").bind(item).fetch_one(pool).await.unwrap()
}
async fn status(pool: &PgPool, c: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM claims WHERE id = $1").bind(c).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn expire_due_releases_only_overdue_reserved(pool: PgPool) {
    let (_, items) = wishlist(&pool, "ExpDue0001", 2, 10).await;
    let (a, b) = (items[0], items[1]);
    let due1 = seed_claim(&pool, a, 2, "reserved", "now() - interval '1 minute'").await;
    let due2 = seed_claim(&pool, a, 3, "reserved", "now() - interval '1 hour'").await;
    let live = seed_claim(&pool, a, 1, "reserved", "now() + interval '1 hour'").await;
    let no_ttl = seed_claim(&pool, a, 1, "reserved", "NULL").await;
    let bought = seed_claim(&pool, b, 4, "purchased", "now() - interval '1 day'").await; // 已購買：不動
    assert_eq!(qty(&pool, a).await, 7);
    assert_eq!(claims::expire_due(&pool).await.unwrap(), 2);
    assert_eq!(qty(&pool, a).await, 2); // 7 - 2 - 3
    assert_eq!(qty(&pool, b).await, 4);
    for (c, s) in [(due1, "expired"), (due2, "expired"), (live, "reserved"), (no_ttl, "reserved"), (bought, "purchased")] {
        assert_eq!(status(&pool, c).await, s);
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = 'claim.expire' AND actor_type = 'system'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 2);
    // 冪等：再跑不重複回補
    assert_eq!(claims::expire_due(&pool).await.unwrap(), 0);
    assert_eq!(qty(&pool, a).await, 2);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn expire_due_concurrent_and_vs_cancel(pool: PgPool) {
    let (_, items) = wishlist(&pool, "ExpRace001", 1, 10).await;
    let it = items[0];
    let c1 = seed_claim(&pool, it, 2, "reserved", "now() - interval '1 minute'").await;
    seed_claim(&pool, it, 3, "reserved", "now() - interval '1 minute'").await;
    // 多實例同時跑：總共只回補一次
    let rs = futures_util::future::join_all((0..4).map(|_| claims::expire_due(&pool))).await;
    assert_eq!(rs.into_iter().map(|r| r.unwrap()).sum::<usize>(), 2);
    assert_eq!(qty(&pool, it).await, 0);
    // 已 expired 的認領再被取消：狀態轉換被拒，不會二次回補
    let g: Uuid = sqlx::query_scalar("SELECT guest_id FROM claims WHERE id = $1").bind(c1).fetch_one(&pool).await.unwrap();
    sqlx::query("UPDATE guests SET guest_token_hash = $2 WHERE id = $1").bind(g).bind(sha(b"tok")).execute(&pool).await.unwrap();
    let r = Request::delete(format!("/api/v1/claims/{c1}")).header("x-guest-token", "tok").body(Body::empty()).unwrap();
    let (s, _, b) = call(&app(&pool), r, "9.9.9.9").await;
    assert_eq!((s, b["code"].as_str()), (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION")));
    assert_eq!(qty(&pool, it).await, 0);
}

fn sha(b: &[u8]) -> Vec<u8> { use sha2::{Digest, Sha256}; Sha256::digest(b).to_vec() }

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_post_ip_limit_and_replay_free(pool: PgPool) {
    let a = app(&pool);
    // 5 份清單各 7 個品項，避開每清單 15 次的限制
    let mut its = vec![];
    for w in 0..5 { its.extend(wishlist(&pool, &format!("RlIp{w:06}"), 7, 100).await.1); }
    let k = Uuid::new_v4();
    assert_eq!(call(&a, post_claim(its[0], k), "5.5.5.5").await.0, StatusCode::CREATED);
    // 同 key 重播 40 次：不消耗額度、不 429
    for _ in 0..40 { assert_eq!(call(&a, post_claim(its[0], k), "5.5.5.5").await.0, StatusCode::CREATED); }
    for it in &its[1..30] { assert_eq!(call(&a, post_claim(*it, Uuid::new_v4()), "5.5.5.5").await.0, StatusCode::CREATED); }
    let (s, h, b) = call(&a, post_claim(its[30], Uuid::new_v4()), "5.5.5.5").await;
    assert_eq!((s, b["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("RATE_LIMITED")));
    assert!(h.contains_key("retry-after"));
    assert_eq!(call(&a, post_claim(its[30], Uuid::new_v4()), "6.6.6.6").await.0, StatusCode::CREATED); // 另一 IP 不受影響
    // 超限後重播先前成功的請求仍可回放
    assert_eq!(call(&a, post_claim(its[0], k), "5.5.5.5").await.0, StatusCode::CREATED);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_post_per_wishlist_limit(pool: PgPool) {
    let a = app(&pool);
    let (_, its) = wishlist(&pool, "RlWl000001", 17, 100).await;
    for it in &its[..15] { assert_eq!(call(&a, post_claim(*it, Uuid::new_v4()), "7.7.7.7").await.0, StatusCode::CREATED); }
    let (s, h, b) = call(&a, post_claim(its[15], Uuid::new_v4()), "7.7.7.7").await;
    assert_eq!((s, b["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("RATE_LIMITED")));
    assert!(h.contains_key("retry-after"));
    assert_eq!(call(&a, post_claim(its[15], Uuid::new_v4()), "8.8.8.8").await.0, StatusCode::CREATED);
}
