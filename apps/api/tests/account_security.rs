//! F-08 刪帳號取消認領、F-12 problem+json、F-18 安全標頭、F-24 登入鎖定、F-26 退訂 POST
use axum::{body::Body, extract::ConnectInfo, http::{header, Request, StatusCode}};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::net::SocketAddr;
use tower::ServiceExt;
use uuid::Uuid;
use wishsync_api::{app, session::hash_token, AppState};

async fn raw(pool: &PgPool, method: &str, uri: &str, ip: Option<&str>, ct: Option<&str>, hdr: &[(&str, String)], body: Vec<u8>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut r = Request::builder().method(method).uri(uri);
    if let Some(c) = ct { r = r.header(header::CONTENT_TYPE, c); }
    for (k, v) in hdr { r = r.header(*k, v); }
    let mut req = r.body(Body::from(body)).unwrap();
    if let Some(ip) = ip { req.extensions_mut().insert(ConnectInfo(format!("{ip}:1234").parse::<SocketAddr>().unwrap())); }
    let res = app(AppState { pool: pool.clone() }).oneshot(req).await.unwrap();
    let (s, h) = (res.status(), res.headers().clone());
    (s, h, res.into_body().collect().await.unwrap().to_bytes().to_vec())
}
async fn call(pool: &PgPool, method: &str, uri: &str, ip: Option<&str>, hdr: &[(&str, String)], body: Option<Value>) -> (StatusCode, axum::http::HeaderMap, Value) {
    let (s, h, b) = raw(pool, method, &format!("/api/v1{uri}"), ip, body.as_ref().map(|_| "application/json"), hdr, body.map(|b| b.to_string().into_bytes()).unwrap_or_default()).await;
    (s, h, serde_json::from_slice(&b).unwrap_or(Value::Null))
}
async fn user(pool: &PgPool) -> (Uuid, String) {
    let id: Uuid = sqlx::query_scalar("INSERT INTO users (display_name, email) VALUES ('u', $1) RETURNING id").bind(format!("{}@example.com", Uuid::new_v4())).fetch_one(pool).await.unwrap();
    let tok = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '1 day')").bind(id).bind(hash_token(&tok)).execute(pool).await.unwrap();
    (id, tok)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn delete_account_cancels_reserved_keeps_fulfilled(pool: PgPool) {
    let (owner, _) = user(&pool).await;
    let (me, tok) = user(&pool).await;
    let wid: Uuid = sqlx::query_scalar("INSERT INTO wishlists (owner_id, type, status, slug, title) VALUES ($1,'registry','active',$2,'L') RETURNING id")
        .bind(owner).bind(Uuid::new_v4().simple().to_string()[..10].to_string()).fetch_one(&pool).await.unwrap();
    let mut items = vec![];
    for (st, q) in [("reserved", 2), ("reserved", 1), ("purchased", 2), ("delivered", 1)] {
        let it: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed, qty_claimed) VALUES ($1,'i',10,$2) RETURNING id").bind(wid).bind(q).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO claims (item_id, user_id, claimer_name, qty, status) VALUES ($1,$2,'me',$3,$4::text::claim_status)")
            .bind(it).bind(me).bind(q).bind(st).execute(&pool).await.unwrap();
        items.push(it);
    }
    let (s, _, _) = call(&pool, "DELETE", "/me", None, &[("cookie", format!("ws_session={tok}"))], Some(json!({"confirm":"DELETE"}))).await;
    assert_eq!(s, StatusCode::OK);
    let sts: Vec<String> = sqlx::query_scalar("SELECT status::text FROM claims WHERE user_id=$1").bind(me).fetch_all(&pool).await.unwrap();
    assert_eq!(sts.iter().filter(|s| *s == "cancelled").count(), 2);
    assert_eq!(sts.iter().filter(|s| *s == "purchased" || *s == "delivered").count(), 2);
    let q: Vec<i32> = sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id = ANY($1) ORDER BY id").bind(&items).fetch_all(&pool).await.unwrap();
    assert_eq!(q, vec![0, 0, 2, 1], "reserved 回補，purchased/delivered 保留");
    let aud: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='claim.account_delete_cancel' AND actor_id=$1").bind(me).fetch_one(&pool).await.unwrap();
    assert_eq!(aud, 2);
    let names: i64 = sqlx::query_scalar("SELECT count(*) FROM claims WHERE user_id=$1 AND claimer_name='已刪除的使用者'").bind(me).fetch_one(&pool).await.unwrap();
    assert_eq!(names, 4);
}

fn assert_problem(s: StatusCode, h: &axum::http::HeaderMap, b: &[u8], want: StatusCode, code: &str) {
    assert_eq!(s, want);
    assert_eq!(h[header::CONTENT_TYPE], "application/problem+json");
    let v: Value = serde_json::from_slice(b).unwrap();
    assert_eq!(v["code"], code);
    assert_eq!(v["status"], want.as_u16());
    let t = String::from_utf8_lossy(b);
    assert!(!t.contains("LoginReq") && !t.contains("deserialize") && !t.contains("Cannot parse"), "洩漏內部資訊：{t}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn framework_errors_are_problem_json(pool: PgPool) {
    let login = "/api/v1/auth/login";
    let (s, h, b) = raw(&pool, "POST", login, None, Some("text/plain"), &[], b"{}".to_vec()).await;
    assert_problem(s, &h, &b, StatusCode::UNSUPPORTED_MEDIA_TYPE, "UNSUPPORTED_MEDIA_TYPE");
    let (s, h, b) = raw(&pool, "POST", login, None, Some("application/json"), &[], b"{oops".to_vec()).await;
    assert_problem(s, &h, &b, StatusCode::BAD_REQUEST, "BAD_REQUEST");
    let (s, h, b) = raw(&pool, "POST", login, None, Some("application/json"), &[], b"{}".to_vec()).await;
    assert_problem(s, &h, &b, StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    let (s, h, b) = raw(&pool, "POST", login, None, Some("application/json"), &[], br#"{"email":"\ud800","password":"x"}"#.to_vec()).await;
    assert_problem(s, &h, &b, StatusCode::BAD_REQUEST, "BAD_REQUEST");
    let (s, h, b) = raw(&pool, "PATCH", "/api/v1/claims/not-a-uuid", None, Some("application/json"), &[], b"{}".to_vec()).await;
    assert_eq!(h[header::CONTENT_TYPE], "application/problem+json");
    assert!(s.is_client_error() && !String::from_utf8_lossy(&b).contains("Cannot parse"), "{s} {}", String::from_utf8_lossy(&b));
    let (s, h, b) = raw(&pool, "POST", login, None, Some("application/json"), &[], vec![b' '; 3 * 1024 * 1024]).await;
    assert_problem(s, &h, &b, StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE");
    // 自家 problem+json 不被改寫
    let (s, h, b) = raw(&pool, "GET", "/api/v1/me", None, None, &[], vec![]).await;
    assert_problem(s, &h, &b, StatusCode::UNAUTHORIZED, "UNAUTHORIZED");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn security_headers_on_all_responses(pool: PgPool) {
    for (m, u) in [("GET", "/api/v1/me"), ("GET", "/api/v1/nope"), ("POST", "/api/v1/auth/login")] {
        let (_, h, _) = raw(&pool, m, u, None, None, &[], vec![]).await;
        assert_eq!(h["x-content-type-options"], "nosniff", "{u}");
        assert_eq!(h["referrer-policy"], "no-referrer");
        assert_eq!(h["cache-control"], "no-store");
    }
    let (_, tok) = user(&pool).await;
    let (_, h, _) = call(&pool, "GET", "/me/export", None, &[("cookie", format!("ws_session={tok}"))], None).await;
    assert_eq!(h["cache-control"], "private, no-store", "已設定者不覆蓋");
    assert_eq!(h["x-content-type-options"], "nosniff");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn login_lockout_is_per_email_ip_with_email_cap(pool: PgPool) {
    let email = format!("{}@example.com", Uuid::new_v4());
    let uid: Uuid = sqlx::query_scalar("INSERT INTO users (display_name, email) VALUES ('u',$1) RETURNING id").bind(&email).fetch_one(&pool).await.unwrap();
    use argon2::{password_hash::{rand_core::OsRng, SaltString}, Argon2, PasswordHasher};
    let h = Argon2::default().hash_password(b"correct-horse", &SaltString::generate(&mut OsRng)).unwrap().to_string();
    sqlx::query("INSERT INTO auth_identities (user_id, provider, provider_uid, email, password_hash) VALUES ($1,'email',$2,$2,$3)").bind(uid).bind(&email).bind(h).execute(&pool).await.unwrap();
    let login = |ip: &'static str, pw: &'static str| { let (p, e) = (pool.clone(), email.clone()); async move { call(&p, "POST", "/auth/login", Some(ip), &[], Some(json!({"email": e, "password": pw}))).await.0 } };
    for _ in 0..5 { assert_eq!(login("1.1.1.1", "bad-password").await, StatusCode::UNAUTHORIZED); }
    assert_eq!(login("1.1.1.1", "correct-horse").await, StatusCode::TOO_MANY_REQUESTS, "攻擊者自己的 (email, IP) 被鎖");
    assert_eq!(login("2.2.2.2", "correct-horse").await, StatusCode::OK, "受害者換 IP 不受影響");
    // 分散式：5 個 IP 各 4 次（每組 < 5）= 20 → email 總量鎖
    for i in 0..5 { for _ in 0..4 {
        sqlx::query("INSERT INTO login_failures (email, ip) VALUES ($1,$2)").bind(&email).bind(format!("10.0.0.{i}")).execute(&pool).await.unwrap();
    } }
    assert_eq!(login("3.3.3.3", "correct-horse").await, StatusCode::TOO_MANY_REQUESTS, "email 合計 20 次後鎖定");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn login_19_distributed_failures_not_locked(pool: PgPool) {
    let email = "dist@example.com";
    for i in 0..19 { sqlx::query("INSERT INTO login_failures (email, ip) VALUES ($1,$2)").bind(email).bind(format!("10.1.0.{}", i % 5)).execute(&pool).await.unwrap(); }
    let bad = |ip| { let p = pool.clone(); async move { call(&p, "POST", "/auth/login", Some(ip), &[], Some(json!({"email": email, "password": "bad-password"}))).await.0 } };
    assert_eq!(bad("4.4.4.4").await, StatusCode::UNAUTHORIZED); // 第 20 筆
    assert_eq!(bad("5.5.5.5").await, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unsubscribe_post_allowed_in_read_only(pool: PgPool) {
    let (u, _) = user(&pool).await;
    sqlx::query("INSERT INTO system_flags (key, value) VALUES ('read_only','true') ON CONFLICT (key) DO UPDATE SET value='true'").execute(&pool).await.unwrap();
    let tok = wishsync_api::account::unsub_token('u', u);
    assert_eq!(call(&pool, "POST", "/unsubscribe", None, &[], Some(json!({"token": tok}))).await.0, StatusCode::NO_CONTENT);
    assert_eq!(call(&pool, "POST", "/unsubscribe", None, &[], Some(json!({"token": "x"}))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
}
