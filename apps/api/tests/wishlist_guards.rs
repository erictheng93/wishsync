use axum::{body::Body, http::{Request, StatusCode}};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wishsync_api::{app, session::hash_token, AppState};

async fn call(pool: &PgPool, method: &str, uri: &str, hdr: &[(&str, String)], body: Option<Value>) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut r = Request::builder().method(method).uri(format!("/api/v1{uri}"));
    for (k, v) in hdr { r = r.header(*k, v); }
    let req = match body { Some(b) => r.header("content-type", "application/json").body(Body::from(b.to_string())), None => r.body(Body::empty()) }.unwrap();
    let res = app(AppState { pool: pool.clone() }).oneshot(req).await.unwrap();
    let (st, h) = (res.status(), res.headers().clone());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (st, h, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}
fn ck(t: &str) -> [(&'static str, String); 1] { [("cookie", format!("ws_session={t}"))] }
fn gt(t: &str) -> [(&'static str, String); 1] { [("x-guest-token", t.to_string())] }

async fn user(pool: &PgPool, staff: bool) -> (Uuid, String) {
    let id: Uuid = sqlx::query_scalar("INSERT INTO users (display_name, email, is_staff) VALUES ('u', $1, $2) RETURNING id")
        .bind(format!("{}@example.com", Uuid::new_v4())).bind(staff).fetch_one(pool).await.unwrap();
    let tok = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(id).bind(hash_token(&tok)).execute(pool).await.unwrap();
    (id, tok)
}

/// 清單 + 一個 qty_needed=3 的品項；event_offset_days>0 且 surprise → 鎖定
async fn list(pool: &PgPool, owner: Uuid, surprise: bool, off: i32) -> (Uuid, Uuid) {
    let slug: String = Uuid::new_v4().simple().to_string()[..10].to_string();
    let wid: Uuid = sqlx::query_scalar("INSERT INTO wishlists (owner_id, type, status, slug, title, surprise_mode, event_date)
        VALUES ($1, 'registry', 'active', $2, '清單', $3, current_date + $4::int) RETURNING id")
        .bind(owner).bind(slug).bind(surprise).bind(off).fetch_one(pool).await.unwrap();
    let it: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, '奶瓶', 3) RETURNING id").bind(wid).fetch_one(pool).await.unwrap();
    (wid, it)
}

/// 訪客認領；回傳 (guest_token, claim_id)
async fn claim(pool: &PgPool, item: Uuid, email: Option<&str>) -> (String, Uuid) {
    let (s, _, b) = call(pool, "POST", &format!("/items/{item}/claims"), &[("idempotency-key", Uuid::new_v4().to_string())],
        Some(json!({ "qty": 1, "display_name": "小明", "email": email }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    (b["guest_token"].as_str().unwrap().into(), b["claim"]["id"].as_str().unwrap().parse().unwrap())
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn surprise_lock_errors_do_not_reveal_claims(pool: PgPool) {
    let (o, t) = user(&pool, false).await;
    let (wid, claimed) = list(&pool, o, true, 30).await;
    let free: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1,'x',3) RETURNING id").bind(wid).fetch_one(&pool).await.unwrap();
    claim(&pool, claimed, None).await;
    for it in [claimed, free] {
        let (s, _, b) = call(&pool, "DELETE", &format!("/items/{it}?force=true"), &ck(&t), None).await;
        assert_eq!((s.as_u16(), b["code"].as_str()), (403, Some("FORBIDDEN")), "{b}");
        let (s, _, b) = call(&pool, "DELETE", &format!("/items/{it}"), &ck(&t), None).await;
        assert_eq!((s.as_u16(), b["code"].as_str()), (403, Some("FORBIDDEN")), "{b}");
        // 調降（無論低於或高於已認領）同樣回通用錯誤，回應完全相同
        for q in [1, 2] {
            let (s, _, b) = call(&pool, "PATCH", &format!("/items/{it}"), &ck(&t), Some(json!({ "qty_needed": q }))).await;
            assert_eq!((s.as_u16(), b["code"].as_str()), (403, Some("FORBIDDEN")), "{b}");
        }
        // 調升與其他欄位仍可改，且不含 qty_claimed
        let (s, _, b) = call(&pool, "PATCH", &format!("/items/{it}"), &ck(&t), Some(json!({ "qty_needed": 5, "title": "新" }))).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert!(b["qty_claimed"].is_null());
    }
    // GET / reorder 也不洩漏
    let (_, _, g) = call(&pool, "GET", &format!("/wishlists/{wid}"), &ck(&t), None).await;
    assert!(g["items"].as_array().unwrap().iter().all(|i| i["qty_claimed"].is_null() && i["updated_at"] == i["created_at"]));
    let ids: Vec<Value> = g["items"].as_array().unwrap().iter().map(|i| i["id"].clone()).collect();
    let (s, _, _) = call(&pool, "POST", &format!("/wishlists/{wid}/items/reorder"), &ck(&t), Some(json!({ "item_ids": ids }))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unlocked_keeps_specific_errors(pool: PgPool) {
    let (o, t) = user(&pool, false).await;
    let (_, it) = list(&pool, o, false, 30).await;
    claim(&pool, it, None).await;
    let (_, _, b) = call(&pool, "PATCH", &format!("/items/{it}"), &ck(&t), Some(json!({ "qty_needed": 0 }))).await;
    assert_eq!(b["code"], "VALIDATION_FAILED");
    sqlx::query("UPDATE wishlist_items SET qty_claimed = 3 WHERE id=$1").bind(it).execute(&pool).await.unwrap();
    let (_, _, b) = call(&pool, "PATCH", &format!("/items/{it}"), &ck(&t), Some(json!({ "qty_needed": 2 }))).await;
    assert_eq!(b["code"], "QTY_BELOW_CLAIMED");
    let (s, _, b) = call(&pool, "DELETE", &format!("/items/{it}"), &ck(&t), None).await;
    assert_eq!((s.as_u16(), b["code"].as_str()), (409, Some("ITEM_HAS_CLAIMS")));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn archive_is_idempotent(pool: PgPool) {
    let (o, t) = user(&pool, false).await;
    let (wid, _) = list(&pool, o, false, 30).await;
    let uri = format!("/wishlists/{wid}");
    assert_eq!(call(&pool, "DELETE", &uri, &ck(&t), None).await.0, StatusCode::NO_CONTENT);
    let (_, _, a) = call(&pool, "GET", &uri, &ck(&t), None).await;
    assert_eq!(call(&pool, "DELETE", &uri, &ck(&t), None).await.0, StatusCode::NO_CONTENT);
    let (s, _, b) = call(&pool, "PATCH", &uri, &ck(&t), Some(json!({ "status": "archived" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "archived");
    assert_eq!(b["updated_at"], a["wishlist"]["updated_at"], "重複封存不應推進 updated_at");
    // 其他修改仍被拒
    let (s, _, b) = call(&pool, "PATCH", &uri, &ck(&t), Some(json!({ "title": "x" }))).await;
    assert_eq!((s.as_u16(), b["code"].as_str()), (409, Some("WISHLIST_CLOSED")));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn optimistic_concurrency(pool: PgPool) {
    let (o, t) = user(&pool, false).await;
    let (wid, it) = list(&pool, o, false, 30).await;
    let uri = format!("/wishlists/{wid}");
    let (_, _, g) = call(&pool, "GET", &uri, &ck(&t), None).await;
    let v0 = g["wishlist"]["updated_at"].clone();
    let (s, _, b) = call(&pool, "PATCH", &uri, &ck(&t), Some(json!({ "title": "A", "expected_updated_at": v0 }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_ne!(b["updated_at"], v0);
    // 舊版本寫入 → 衝突且不覆蓋
    let (s, _, b) = call(&pool, "PATCH", &uri, &ck(&t), Some(json!({ "title": "B", "expected_updated_at": v0 }))).await;
    assert_eq!((s.as_u16(), b["code"].as_str()), (409, Some("STALE_VERSION")), "{b}");
    let (_, _, g) = call(&pool, "GET", &uri, &ck(&t), None).await;
    assert_eq!(g["wishlist"]["title"], "A");
    // 用新版本可成功；未帶則照舊
    let v1 = g["wishlist"]["updated_at"].clone();
    assert_eq!(call(&pool, "PATCH", &uri, &ck(&t), Some(json!({ "title": "C", "expected_updated_at": v1 }))).await.0, StatusCode::OK);
    assert_eq!(call(&pool, "PATCH", &uri, &ck(&t), Some(json!({ "title": "D" }))).await.0, StatusCode::OK);
    assert_eq!(call(&pool, "PATCH", &uri, &ck(&t), Some(json!({ "title": "E", "expected_updated_at": "bad" }))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    // 品項
    let (_, _, g) = call(&pool, "GET", &uri, &ck(&t), None).await;
    let iv = g["items"][0]["updated_at"].clone();
    let iu = format!("/items/{it}");
    assert_eq!(call(&pool, "PATCH", &iu, &ck(&t), Some(json!({ "title": "1", "expected_updated_at": iv }))).await.0, StatusCode::OK);
    let (s, _, b) = call(&pool, "PATCH", &iu, &ck(&t), Some(json!({ "title": "2", "expected_updated_at": iv }))).await;
    assert_eq!((s.as_u16(), b["code"].as_str()), (409, Some("STALE_VERSION")), "{b}");
}

