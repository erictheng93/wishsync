//! 清單可見性：6 種 visibility × 擁有者/匿名/好友/非好友/名單/密碼；認領門檻、unlock、快取標頭、allowed-users
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
fn ck(t: &str) -> Vec<(&'static str, String)> { vec![("cookie", format!("ws_session={t}"))] }

async fn user(pool: &PgPool, name: &str) -> (Uuid, String) {
    let id: Uuid = sqlx::query_scalar("INSERT INTO users (display_name, email) VALUES ($1, $2) RETURNING id")
        .bind(name).bind(format!("{}@example.com", Uuid::new_v4())).fetch_one(pool).await.unwrap();
    let tok = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(id).bind(hash_token(&tok)).execute(pool).await.unwrap();
    (id, tok)
}
async fn befriend(pool: &PgPool, a: Uuid, b: Uuid) {
    let (x, y) = if a < b { (a, b) } else { (b, a) };
    sqlx::query("INSERT INTO friendships (user_a, user_b) VALUES ($1, $2)").bind(x).bind(y).execute(pool).await.unwrap();
}

struct Ctx { owner: (Uuid, String), friend: (Uuid, String), stranger: (Uuid, String), allowed: (Uuid, String) }
async fn ctx(pool: &PgPool) -> Ctx {
    let c = Ctx { owner: user(pool, "主人").await, friend: user(pool, "好友").await, stranger: user(pool, "路人").await, allowed: user(pool, "名單").await };
    befriend(pool, c.owner.0, c.friend.0).await;
    befriend(pool, c.owner.0, c.allowed.0).await;
    c
}

/// 經 API 建立 active 清單（含一個品項）；回 (id, slug, item_id)
async fn mk(pool: &PgPool, c: &Ctx, vis: &str, pw: Option<&str>) -> (Uuid, String, Uuid) {
    let mut b = json!({ "type": "personal", "title": "清單", "visibility": vis });
    if let Some(p) = pw { b["access_password"] = json!(p); }
    let (s, _, w) = call(pool, "POST", "/wishlists", &ck(&c.owner.1), Some(b)).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let id: Uuid = w["id"].as_str().unwrap().parse().unwrap();
    let it: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, '奶瓶', 3) RETURNING id").bind(id).fetch_one(pool).await.unwrap();
    sqlx::query("UPDATE wishlists SET status='active' WHERE id=$1").bind(id).execute(pool).await.unwrap();
    if vis == "selected" {
        let (s, _, v) = call(pool, "PUT", &format!("/wishlists/{id}/allowed-users"), &ck(&c.owner.1), Some(json!({ "user_ids": [c.allowed.0] }))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    (id, w["slug"].as_str().unwrap().into(), it)
}

async fn get(pool: &PgPool, slug: &str, hdr: &[(&str, String)]) -> (StatusCode, axum::http::HeaderMap, Value) {
    call(pool, "GET", &format!("/public/wishlists/{slug}"), hdr, None).await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_and_link_open_and_cacheable(pool: PgPool) {
    let c = ctx(&pool).await;
    for vis in ["public", "link"] {
        let (_, slug, _) = mk(&pool, &c, vis, None).await;
        let (s, h, v) = get(&pool, &slug, &[]).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["visibility"], vis);
        assert!(h["cache-control"].to_str().unwrap().starts_with("public"));
        let etag = h["etag"].to_str().unwrap().to_string();
        assert_eq!(get(&pool, &slug, &[("if-none-match", etag)]).await.0, StatusCode::NOT_MODIFIED);
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn private_only_owner(pool: PgPool) {
    let c = ctx(&pool).await;
    let (_, slug, _) = mk(&pool, &c, "private", None).await;
    assert_eq!(get(&pool, &slug, &[]).await.0, StatusCode::NOT_FOUND);
    assert_eq!(get(&pool, &slug, &ck(&c.friend.1)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(get(&pool, &slug, &ck(&c.owner.1)).await.0, StatusCode::OK);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn friends_list(pool: PgPool) {
    let c = ctx(&pool).await;
    let (_, slug, _) = mk(&pool, &c, "friends", None).await;
    let (s, _, v) = get(&pool, &slug, &[]).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("LOGIN_REQUIRED")));
    assert_eq!(v["visibility"], "friends");
    assert_eq!(v["owner"]["display_name"], "主人");
    let (s, _, v) = get(&pool, &slug, &ck(&c.stranger.1)).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("FRIENDS_ONLY")));
    for t in [&c.friend.1, &c.owner.1] {
        let (s, h, v) = get(&pool, &slug, &ck(t)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["visibility"], "friends");
        assert_eq!(h["cache-control"], "private, no-store");
        assert!(h.get("etag").is_none());
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn selected_list(pool: PgPool) {
    let c = ctx(&pool).await;
    let (_, slug, _) = mk(&pool, &c, "selected", None).await;
    assert_eq!(get(&pool, &slug, &[]).await.2["code"], "LOGIN_REQUIRED");
    // 好友但不在名單 / 路人 → NOT_ALLOWED
    for t in [&c.friend.1, &c.stranger.1] {
        let (s, _, v) = get(&pool, &slug, &ck(t)).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("NOT_ALLOWED")));
    }
    let (s, h, _) = get(&pool, &slug, &ck(&c.allowed.1)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["cache-control"], "private, no-store");
    assert_eq!(get(&pool, &slug, &ck(&c.owner.1)).await.0, StatusCode::OK);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn password_list_unlock_and_tokens(pool: PgPool) {
    let c = ctx(&pool).await;
    let (_, slug, _) = mk(&pool, &c, "password", Some("correct-horse")).await;
    let (s, _, v) = get(&pool, &slug, &[]).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("PASSWORD_REQUIRED")));
    assert_eq!(get(&pool, &slug, &[("x-list-access", "bogus".into())]).await.2["code"], "PASSWORD_REQUIRED");
    // 錯密碼 / 缺密碼
    let u = format!("/public/wishlists/{slug}/unlock");
    let (s, _, v) = call(&pool, "POST", &u, &[], Some(json!({ "password": "wrong-password" }))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("WRONG_PASSWORD")));
    let (s, _, v) = call(&pool, "POST", &u, &[], Some(json!({ "password": "correct-horse" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let tok = v["access_token"].as_str().unwrap().to_string();
    let (s, h, v) = get(&pool, &slug, &[("x-list-access", tok.clone())]).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((v["visibility"].as_str(), h["cache-control"].to_str().unwrap()), (Some("password"), "private, no-store"));
    assert!(h.get("etag").is_none());
    // 擁有者免密碼
    assert_eq!(get(&pool, &slug, &ck(&c.owner.1)).await.0, StatusCode::OK);
    // SSE 與檢舉走同一道門
    assert_eq!(call(&pool, "GET", &format!("/public/wishlists/{slug}/events"), &[], None).await.0, StatusCode::FORBIDDEN);
    // 通過時是串流，不能 collect body，只看狀態碼
    let req = Request::builder().uri(format!("/api/v1/public/wishlists/{slug}/events?access={tok}")).body(Body::empty()).unwrap();
    assert_eq!(app(AppState { pool: pool.clone() }).oneshot(req).await.unwrap().status(), StatusCode::OK);
    let rep = json!({ "reason": "other" });
    assert_eq!(call(&pool, "POST", &format!("/public/wishlists/{slug}/reports"), &[], Some(rep.clone())).await.0, StatusCode::FORBIDDEN);
    assert_eq!(call(&pool, "POST", &format!("/public/wishlists/{slug}/reports"), &[("x-list-access", tok.clone())], Some(rep)).await.0, StatusCode::CREATED);
    // 換密碼 → 舊權杖失效
    let id: Uuid = sqlx::query_scalar("SELECT id FROM wishlists WHERE slug=$1").bind(&slug).fetch_one(&pool).await.unwrap();
    let (s, _, _) = call(&pool, "PATCH", &format!("/wishlists/{id}"), &ck(&c.owner.1), Some(json!({ "access_password": "another-pass1" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(get(&pool, &slug, &[("x-list-access", tok)]).await.0, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unlock_non_password_404_and_rate_limited(pool: PgPool) {
    let c = ctx(&pool).await;
    let (_, link, _) = mk(&pool, &c, "link", None).await;
    assert_eq!(call(&pool, "POST", &format!("/public/wishlists/{link}/unlock"), &[], Some(json!({ "password": "whatever-123" }))).await.0, StatusCode::NOT_FOUND);
    let (_, slug, _) = mk(&pool, &c, "password", Some("correct-horse")).await;
    let u = format!("/public/wishlists/{slug}/unlock");
    assert_eq!(call(&pool, "POST", &u, &[], Some(json!({ "password": "wrong-password" }))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    // 直接把計數推到上限（不跑 10 次 argon2）：固定視窗對齊時鐘，慢的迴圈若跨過視窗邊界計數會歸零而偶發失敗
    sqlx::query("UPDATE rate_limits SET count = 10 WHERE key LIKE $1").bind(format!("unlock:{slug}:%")).execute(&pool).await.unwrap();
    let (s, _, v) = call(&pool, "POST", &u, &[], Some(json!({ "password": "correct-horse" }))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("RATE_LIMITED")));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_requires_access(pool: PgPool) {
    let c = ctx(&pool).await;
    let claim = |it: Uuid, mut h: Vec<(&'static str, String)>| { let pool = pool.clone(); async move {
        h.push(("idempotency-key", Uuid::new_v4().to_string()));
        call(&pool, "POST", &format!("/items/{it}/claims"), &h, Some(json!({ "qty": 1, "display_name": "小明" }))).await
    } };
    let (_, _, it) = mk(&pool, &c, "friends", None).await;
    let (s, _, v) = claim(it, vec![]).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("LOGIN_REQUIRED")));
    assert_eq!(claim(it, ck(&c.stranger.1)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(claim(it, ck(&c.friend.1)).await.0, StatusCode::CREATED);

    let (_, _, it) = mk(&pool, &c, "selected", None).await;
    assert_eq!(claim(it, ck(&c.friend.1)).await.2["code"], "NOT_ALLOWED");
    assert_eq!(claim(it, ck(&c.allowed.1)).await.0, StatusCode::CREATED);

    let (_, slug, it) = mk(&pool, &c, "password", Some("correct-horse")).await;
    assert_eq!(claim(it, vec![]).await.2["code"], "PASSWORD_REQUIRED");
    let (_, _, v) = call(&pool, "POST", &format!("/public/wishlists/{slug}/unlock"), &[], Some(json!({ "password": "correct-horse" }))).await;
    let tok = v["access_token"].as_str().unwrap().to_string();
    assert_eq!(claim(it, vec![("x-list-access", tok)]).await.0, StatusCode::CREATED);

    let (_, _, it) = mk(&pool, &c, "private", None).await;
    assert_eq!(claim(it, ck(&c.friend.1)).await.0, StatusCode::NOT_FOUND);
    let (_, _, it) = mk(&pool, &c, "public", None).await;
    assert_eq!(claim(it, vec![]).await.0, StatusCode::CREATED);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn owner_password_rules_and_no_hash_leak(pool: PgPool) {
    let c = ctx(&pool).await;
    let o = ck(&c.owner.1);
    let bad = |b: Value| { let (p, o) = (pool.clone(), o.clone()); async move { call(&p, "POST", "/wishlists", &o, Some(b)).await } };
    assert_eq!(bad(json!({ "type": "personal", "title": "x", "visibility": "password" })).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bad(json!({ "type": "personal", "title": "x", "visibility": "password", "access_password": "short" })).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bad(json!({ "type": "personal", "title": "x", "visibility": "nope" })).await.0, StatusCode::UNPROCESSABLE_ENTITY);

    let (id, _, _) = mk(&pool, &c, "password", Some("correct-horse")).await;
    let (_, _, w) = call(&pool, "GET", &format!("/wishlists/{id}"), &o, None).await;
    assert_eq!(w["wishlist"]["has_password"], true);
    assert!(!w.to_string().contains("argon2"));
    // 離開 password → 清空雜湊；再進入須重新給密碼
    let (s, _, w) = call(&pool, "PATCH", &format!("/wishlists/{id}"), &o, Some(json!({ "visibility": "link" }))).await;
    assert_eq!((s, &w["has_password"]), (StatusCode::OK, &json!(false)));
    let h: Option<String> = sqlx::query_scalar("SELECT access_password_hash FROM wishlists WHERE id=$1").bind(id).fetch_one(&pool).await.unwrap();
    assert!(h.is_none());
    assert_eq!(call(&pool, "PATCH", &format!("/wishlists/{id}"), &o, Some(json!({ "visibility": "password" }))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _, w) = call(&pool, "PATCH", &format!("/wishlists/{id}"), &o, Some(json!({ "visibility": "password", "access_password": "new-password-1" }))).await;
    assert_eq!((s, &w["has_password"]), (StatusCode::OK, &json!(true)));
    // 已有密碼時改其他欄位不需重給
    assert_eq!(call(&pool, "PATCH", &format!("/wishlists/{id}"), &o, Some(json!({ "title": "新標題" }))).await.0, StatusCode::OK);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn allowed_users_only_friends(pool: PgPool) {
    let c = ctx(&pool).await;
    let (id, _, _) = mk(&pool, &c, "selected", None).await;
    let u = format!("/wishlists/{id}/allowed-users");
    let (s, _, v) = call(&pool, "PUT", &u, &ck(&c.owner.1), Some(json!({ "user_ids": [c.friend.0, c.stranger.0] }))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("NOT_FRIEND")));
    // 失敗不改動既有名單
    let (_, _, v) = call(&pool, "GET", &u, &ck(&c.owner.1), None).await;
    assert_eq!(v["users"].as_array().unwrap().len(), 1);
    let (s, _, v) = call(&pool, "PUT", &u, &ck(&c.owner.1), Some(json!({ "user_ids": [c.friend.0, c.allowed.0, c.friend.0] }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["users"].as_array().unwrap().len(), 2);
    assert_eq!(call(&pool, "PUT", &u, &ck(&c.owner.1), Some(json!({ "user_ids": [] }))).await.2["users"].as_array().unwrap().len(), 0);
    // 非擁有者 → 404
    assert_eq!(call(&pool, "GET", &u, &ck(&c.friend.1), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "PUT", &u, &ck(&c.friend.1), Some(json!({ "user_ids": [] }))).await.0, StatusCode::NOT_FOUND);
}
