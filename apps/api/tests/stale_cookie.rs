//! 失效的 guest token：header 帶的 → 401；只有 cookie 帶的（清不掉的殘留）→ 視為匿名，建立新訪客並覆蓋 cookie。
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



fn claim_req(item: Uuid, extra: (&str, &str)) -> Request<Body> {
    Request::post(format!("/api/v1/items/{item}/claims")).header("content-type", "application/json")
        .header("idempotency-key", Uuid::new_v4().to_string()).header(extra.0, extra.1)
        .body(Body::from(json!({"qty": 1, "display_name": "新訪客"}).to_string())).unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn stale_cookie_is_anonymous_but_stale_header_is_401(pool: PgPool) {
    let a = app(&pool);
    let (_, its) = wishlist(&pool, "StaleTok01", 2, 10).await;
    // header 明確帶了失效 token → 401
    let (s, _, b) = call(&a, claim_req(its[0], ("x-guest-token", "dead-token")), "2.2.2.2").await;
    assert_eq!((s, b["code"].as_str()), (StatusCode::UNAUTHORIZED, Some("UNAUTHORIZED")));
    // 只有失效的 ws_guest cookie → 視為匿名：建立新訪客、回新 token，並以 Set-Cookie 覆蓋舊的
    let (s, h, b) = call(&a, claim_req(its[1], ("cookie", "ws_guest=dead-cookie")), "2.2.2.2").await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert!(b["guest_token"].as_str().is_some());
    assert!(h["set-cookie"].to_str().unwrap().starts_with("ws_guest="));
}
