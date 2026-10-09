use axum::{body::Body, http::{Request, StatusCode}, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wishsync_api::{claims, guest, public, AppState};

async fn call(app: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let (st, h) = (res.status(), res.headers().clone());
    let b = res.into_body().collect().await.unwrap().to_bytes();
    (st, h, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

fn get(uri: &str) -> Request<Body> { Request::get(uri).body(Body::empty()).unwrap() }

fn post_claim(item: Uuid, key: Uuid, token: Option<&str>, body: Value) -> Request<Body> {
    let mut r = Request::post(format!("/api/v1/items/{item}/claims"))
        .header("content-type", "application/json").header("idempotency-key", key.to_string());
    if let Some(t) = token { r = r.header("x-guest-token", t); }
    r.body(Body::from(body.to_string())).unwrap()
}

struct Fx { slug: String, wl: Uuid }

/// 建立 owner + 清單（active）；回傳 slug / id
async fn wishlist(pool: &PgPool, slug: &str, extra: &str) -> Fx {
    let owner: Uuid = sqlx::query_scalar("INSERT INTO users (display_name) VALUES ('小米') RETURNING id").fetch_one(pool).await.unwrap();
    let wl: Uuid = sqlx::query_scalar(&format!(
        "INSERT INTO wishlists (owner_id, type, status, slug, title {}) VALUES ($1, 'registry', 'active', $2, '寶寶清單' {}) RETURNING id",
        if extra.is_empty() { "" } else { ", moderation_status, moderation_reason, show_claimer_names" },
        if extra.is_empty() { "" } else { extra })).bind(owner).bind(slug).fetch_one(pool).await.unwrap();
    Fx { slug: slug.into(), wl }
}

async fn item(pool: &PgPool, wl: Uuid, title: &str, needed: i32) -> Uuid {
    sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, $2, $3) RETURNING id")
        .bind(wl).bind(title).bind(needed).fetch_one(pool).await.unwrap()
}

/// 只掛本切片的路由，不受其他模組影響
fn test_app(pool: &PgPool) -> Router {
    Router::new()
        .nest("/api/v1", public::routes().merge(guest::routes()).merge(claims::routes()))
        .with_state(AppState { pool: pool.clone() })
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_page_happy_and_masking(pool: PgPool) {
    let app = test_app(&pool);
    let f = wishlist(&pool, "Ab3dE5gH7k", ", 'ok', NULL, true").await;
    let bottle = item(&pool, f.wl, "玻璃奶瓶", 10).await;
    item(&pool, f.wl, "推車", 2).await;
    let (st, _, c) = call(&app, post_claim(bottle, Uuid::new_v4(), None, json!({"qty": 5, "display_name": "小明"}))).await;
    assert_eq!(st, StatusCode::CREATED, "{c}");

    let (st, h, b) = call(&app, get("/api/v1/public/wishlists/Ab3dE5gH7k")).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h["cache-control"].to_str().unwrap().starts_with("public, s-maxage=10"));
    assert_eq!(b["title"], "寶寶清單");
    assert_eq!(b["claimers_visible"], true);
    assert_eq!(b["completion"]["completion_pct"], 42); // 5 / 12
    assert_eq!(b["items"][0]["qty_claimed"], 5);
    assert_eq!(b["items"][0]["qty_remaining"], 5);
    assert_eq!(b["items"][0]["claimers"][0], json!({"display_name": "小明", "qty": 5}));
    assert!(b.to_string().find("contact").is_none());

    // 驚喜鎖定（未來 event_date）→ 即使 show_claimer_names 也不輸出
    sqlx::query("UPDATE wishlists SET surprise_mode = true, event_date = current_date + 30 WHERE id = $1").bind(f.wl).execute(&pool).await.unwrap();
    let (_, _, b) = call(&app, get("/api/v1/public/wishlists/Ab3dE5gH7k")).await;
    assert_eq!(b["surprise_mode"], true);
    assert_eq!(b["claimers_visible"], false);
    assert!(b["items"][0].get("claimers").is_none());
    assert_eq!(b["items"][0]["qty_claimed"], 5);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_page_404_and_410(pool: PgPool) {
    let app = test_app(&pool);
    let get_status = |slug: &'static str| { let app = app.clone(); async move { call(&app, get(&format!("/api/v1/public/wishlists/{slug}"))).await } };
    assert_eq!(get_status("Zz9999999z").await.0, StatusCode::NOT_FOUND);

    let deleted = wishlist(&pool, "Deleted001", "").await;
    sqlx::query("UPDATE wishlists SET deleted_at = now() WHERE id = $1").bind(deleted.wl).execute(&pool).await.unwrap();
    assert_eq!(get_status("Deleted001").await.0, StatusCode::NOT_FOUND);

    let draft = wishlist(&pool, "Draft00001", "").await;
    sqlx::query("UPDATE wishlists SET status = 'draft' WHERE id = $1").bind(draft.wl).execute(&pool).await.unwrap();
    assert_eq!(get_status("Draft00001").await.0, StatusCode::NOT_FOUND);

    wishlist(&pool, "Hidden0001", ", 'hidden', '詐騙', false").await;
    let (st, _, b) = get_status("Hidden0001").await;
    assert_eq!((st, b["code"].as_str().unwrap()), (StatusCode::GONE, "WISHLIST_REMOVED"));
    assert!(!b.to_string().contains("詐騙"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn oversell_only_one_wins(pool: PgPool) {
    let app = test_app(&pool);
    let f = wishlist(&pool, "Race000001", "").await;
    let it = item(&pool, f.wl, "最後一件", 1).await;
    let tasks: Vec<_> = (0..8).map(|n| {
        let app = app.clone();
        tokio::spawn(async move { call(&app, post_claim(it, Uuid::new_v4(), None, json!({"qty": 1, "display_name": format!("g{n}")}))).await })
    }).collect();
    let mut ok = 0;
    for t in tasks {
        let (st, _, b) = t.await.unwrap();
        match st {
            StatusCode::CREATED => ok += 1,
            StatusCode::CONFLICT => { assert_eq!(b["code"], "ITEM_FULLY_CLAIMED"); assert_eq!(b["remaining"], 0); }
            other => panic!("unexpected {other} {b}"),
        }
    }
    assert_eq!(ok, 1);
    let (claimed, n): (i32, i64) = sqlx::query_as("SELECT i.qty_claimed, (SELECT count(*) FROM claims WHERE item_id = i.id) FROM wishlist_items i WHERE i.id = $1")
        .bind(it).fetch_one(&pool).await.unwrap();
    assert_eq!((claimed, n), (1, 1));
    // 失敗請求不留下孤兒 guest
    let guests: i64 = sqlx::query_scalar("SELECT count(*) FROM guests").fetch_one(&pool).await.unwrap();
    assert_eq!(guests, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn idempotency_key_replay_and_conflict(pool: PgPool) {
    let app = test_app(&pool);
    let f = wishlist(&pool, "Idem000001", "").await;
    let it = item(&pool, f.wl, "奶瓶", 5).await;
    let key = Uuid::new_v4();
    let body = json!({"qty": 2, "display_name": "小明"});

    let (s1, h1, b1) = call(&app, post_claim(it, key, None, body.clone())).await;
    assert_eq!(s1, StatusCode::CREATED);
    assert!(h1.contains_key("set-cookie") && h1.contains_key("location"));
    let (s2, h2, b2) = call(&app, post_claim(it, key, None, body.clone())).await;
    assert_eq!(s2, StatusCode::CREATED);
    assert_eq!(h2["idempotency-replayed"], "true");
    // 重播：同 claim，但不再回明文 guest_token（DB 只存雜湊，idempotency_keys 也不得存）、也不 Set-Cookie
    assert!(b1["guest_token"].is_string() && b2.get("guest_token").is_none() && !h2.contains_key("set-cookie"));
    assert_eq!(b1["claim"], b2["claim"]);
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM idempotency_keys WHERE response_body::text LIKE '%guest_token%'").fetch_one(&pool).await.unwrap();
    assert_eq!(stored, 0);
    let (n, q): (i64, i32) = sqlx::query_as("SELECT (SELECT count(*) FROM claims), qty_claimed FROM wishlist_items WHERE id = $1").bind(it).fetch_one(&pool).await.unwrap();
    assert_eq!((n, q), (1, 2));

    let (s3, _, b3) = call(&app, post_claim(it, key, None, json!({"qty": 3, "display_name": "小明"}))).await;
    assert_eq!((s3, b3["code"].as_str().unwrap()), (StatusCode::CONFLICT, "IDEMPOTENCY_CONFLICT"));

    // 缺 key
    let r = Request::post(format!("/api/v1/items/{it}/claims")).body(Body::from(body.to_string())).unwrap();
    let (s4, _, b4) = call(&app, r).await;
    assert_eq!((s4, b4["code"].as_str().unwrap()), (StatusCode::BAD_REQUEST, "IDEMPOTENCY_KEY_REQUIRED"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_lifecycle(pool: PgPool) {
    let app = test_app(&pool);
    let f = wishlist(&pool, "Life000001", "").await;
    let it = item(&pool, f.wl, "奶瓶", 3).await;
    let (_, _, b) = call(&app, post_claim(it, Uuid::new_v4(), None, json!({"qty": 1, "display_name": "小明"}))).await;
    let tok = b["guest_token"].as_str().unwrap().to_string();
    let cid = b["claim"]["id"].as_str().unwrap().to_string();
    let req = |m: &str, uri: &str, body: Value, t: Option<&str>| {
        let mut r = Request::builder().method(m).uri(uri).header("content-type", "application/json");
        if let Some(t) = t { r = r.header("x-guest-token", t); }
        r.body(Body::from(body.to_string())).unwrap()
    };

    // 重複認領 → CLAIM_ALREADY_EXISTS（附 claim_id）
    let (s, _, e) = call(&app, post_claim(it, Uuid::new_v4(), Some(&tok), json!({"qty": 1}))).await;
    assert_eq!((s, e["code"].as_str().unwrap(), e["claim_id"].as_str().unwrap()), (StatusCode::CONFLICT, "CLAIM_ALREADY_EXISTS", cid.as_str()));
    // 改量超過 → ITEM_FULLY_CLAIMED；合法改量
    let url = format!("/api/v1/claims/{cid}");
    let (s, _, e) = call(&app, req("PATCH", &url, json!({"qty": 4}), Some(&tok))).await;
    assert_eq!((s, e["code"].as_str().unwrap()), (StatusCode::CONFLICT, "ITEM_FULLY_CLAIMED"));
    let (s, _, o) = call(&app, req("PATCH", &url, json!({"qty": 3, "status": "purchased"}), Some(&tok))).await;
    assert_eq!(s, StatusCode::OK, "{o}");
    assert_eq!((o["claim"]["status"].as_str().unwrap(), o["item"]["qty_claimed"].as_i64().unwrap()), ("purchased", 3));
    // 他人不可改；無 token 401
    let (s, _, _) = call(&app, req("PATCH", &url, json!({"note": "x"}), None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // GET /guest/me
    let (s, h, me) = call(&app, Request::get("/api/v1/guest/me").header("x-guest-token", &tok).body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["cache-control"], "private, no-store");
    assert_eq!(me["claims"][0]["item"]["title"], "奶瓶");
    assert_eq!(me["claims"][0]["wishlist"]["slug"], "Life000001");
    // 取消回補，重複 DELETE 冪等
    for _ in 0..2 {
        let (s, _, _) = call(&app, req("DELETE", &url, Value::Null, Some(&tok))).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
    }
    let q: i32 = sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id = $1").bind(it).fetch_one(&pool).await.unwrap();
    assert_eq!(q, 0);
    // 取消後可再認領（partial unique 只算有效）
    let (s, _, _) = call(&app, post_claim(it, Uuid::new_v4(), Some(&tok), json!({"qty": 1}))).await;
    assert_eq!(s, StatusCode::CREATED);
}
