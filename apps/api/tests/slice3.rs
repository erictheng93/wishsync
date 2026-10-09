//! 切片 3 整合測試：需要 DATABASE_URL（docker compose 的 db）。不依賴 Mailpit / MinIO / LINE。
use axum::{body::Body, http::{header, Request, StatusCode}, routing::{get, post}, Json, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wishsync_api::{app, AppState};

fn setup(pool: PgPool) -> (Router, PgPool) {
    (app(AppState { pool: pool.clone() }), pool)
}

async fn call(app: &Router, method: &str, uri: &str, cookie: Option<&str>, body: Option<Value>) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut rb = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie { rb = rb.header(header::COOKIE, c); }
    let req = match body {
        Some(b) => rb.header(header::CONTENT_TYPE, "application/json").body(Body::from(b.to_string())).unwrap(),
        None => rb.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let (st, h) = (res.status(), res.headers().clone());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (st, h, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// 直接建 user + session，回傳 cookie
async fn login(pool: &PgPool) -> (Uuid, String) {
    let id: Uuid = sqlx::query_scalar("INSERT INTO users (display_name) VALUES ('t') RETURNING id").fetch_one(pool).await.unwrap();
    let tok = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1,$2, now() + interval '1 day')")
        .bind(id).bind(Sha256::digest(tok.as_bytes()).to_vec()).execute(pool).await.unwrap();
    (id, format!("ws_session={tok}"))
}

/// code_hash 是 SHA256(code||pepper)，測試以暴力枚舉 6 位數還原（不需 Mailpit）
async fn recover_code(pool: &PgPool, email: &str) -> String {
    let pepper = std::env::var("OTP_PEPPER").unwrap_or_else(|_| "dev-pepper".into());
    let h: Vec<u8> = sqlx::query_scalar("SELECT code_hash FROM otp_challenges WHERE email=$1 ORDER BY created_at DESC LIMIT 1").bind(email).fetch_one(pool).await.unwrap();
    (0..1_000_000u32).map(|n| format!("{n:06}")).find(|c| Sha256::digest(format!("{c}{pepper}").as_bytes()).as_slice() == h.as_slice()).unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn otp_full_flow(pool: PgPool) {
    let (app, pool) = setup(pool);
    let email = format!("otp-{}@Example.com", Uuid::new_v4());
    let lower = email.to_lowercase();
    let (st, _, b) = call(&app, "POST", "/api/v1/auth/otp/request", None, Some(json!({ "email": email }))).await;
    assert_eq!(st, 202); assert_eq!(b["expires_in"], 600);
    // 60 秒內重送 → 429
    let (st, h, b) = call(&app, "POST", "/api/v1/auth/otp/request", None, Some(json!({ "email": email }))).await;
    assert_eq!(st, 429); assert_eq!(b["code"], "RATE_LIMITED"); assert!(h.contains_key("retry-after"));
    let code = recover_code(&pool, &lower).await;
    let wrong = if code == "000000" { "000001" } else { "000000" };
    let (st, _, b) = call(&app, "POST", "/api/v1/auth/otp/verify", None, Some(json!({ "email": email, "code": wrong }))).await;
    assert_eq!((st, b["code"].as_str()), (StatusCode::BAD_REQUEST, Some("OTP_INVALID")));
    let (st, h, b) = call(&app, "POST", "/api/v1/auth/otp/verify", None, Some(json!({ "email": email, "code": code }))).await;
    assert_eq!(st, 200, "{b}"); assert_eq!(b["is_new_user"], true); assert_eq!(b["user"]["email"], lower);
    let sc = h.get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();
    assert!(sc.starts_with("ws_session=") && sc.contains("HttpOnly") && sc.contains("SameSite=Lax"), "{sc}");
    let cookie = sc.split(';').next().unwrap().to_string();
    // 驗證碼只能用一次
    let (st, ..) = call(&app, "POST", "/api/v1/auth/otp/verify", None, Some(json!({ "email": email, "code": code }))).await;
    assert_eq!(st, 400);
    // /me
    let (st, _, me) = call(&app, "GET", "/api/v1/me", Some(&cookie), None).await;
    assert_eq!(st, 200); assert_eq!(me["identities"][0]["provider"], "email"); assert_eq!(me["notification_prefs"]["email_claims"], true);
    let (st, _, me) = call(&app, "PATCH", "/api/v1/me", Some(&cookie), Some(json!({ "display_name": "小米", "notification_prefs": { "email_claims": false } }))).await;
    assert_eq!(st, 200); assert_eq!(me["display_name"], "小米"); assert_eq!(me["notification_prefs"]["email_claims"], false);
    let (st, ..) = call(&app, "PATCH", "/api/v1/me", Some(&cookie), Some(json!({ "notification_prefs": { "evil": true } }))).await;
    assert_eq!(st, 422);
    // 登出後 session 失效
    let (st, h, _) = call(&app, "POST", "/api/v1/auth/logout", Some(&cookie), None).await;
    assert_eq!(st, 204); assert!(h.get(header::SET_COOKIE).unwrap().to_str().unwrap().contains("Max-Age=0"));
    let (st, ..) = call(&app, "GET", "/api/v1/me", Some(&cookie), None).await;
    assert_eq!(st, 401);
    // 再次登入同 email → 同一 user，非新使用者（先把上一張挑戰時間往前挪以略過 60 秒限制）
    sqlx::query("UPDATE otp_challenges SET created_at = created_at - interval '2 minutes' WHERE email=$1").bind(&lower).execute(&pool).await.unwrap();
    call(&app, "POST", "/api/v1/auth/otp/request", None, Some(json!({ "email": email }))).await;
    let code = recover_code(&pool, &lower).await;
    let (_, _, b) = call(&app, "POST", "/api/v1/auth/otp/verify", None, Some(json!({ "email": email, "code": code }))).await;
    assert_eq!(b["is_new_user"], false);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn otp_locks_after_5_wrong_attempts(pool: PgPool) {
    let (app, pool) = setup(pool);
    let email = format!("lock-{}@example.com", Uuid::new_v4());
    call(&app, "POST", "/api/v1/auth/otp/request", None, Some(json!({ "email": email }))).await;
    let code = recover_code(&pool, &email).await;
    let wrong = if code == "000000" { "000001" } else { "000000" };
    for _ in 0..5 { call(&app, "POST", "/api/v1/auth/otp/verify", None, Some(json!({ "email": email, "code": wrong }))).await; }
    let (st, _, b) = call(&app, "POST", "/api/v1/auth/otp/verify", None, Some(json!({ "email": email, "code": code }))).await;
    assert_eq!((st, b["code"].as_str()), (StatusCode::BAD_REQUEST, Some("OTP_INVALID")));
    let (st, ..) = call(&app, "POST", "/api/v1/auth/otp/request", None, Some(json!({ "email": "bad" }))).await;
    assert_eq!(st, 422);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn owner_only_and_item_rules(pool: PgPool) {
    let (app, pool) = setup(pool);
    let (_, a) = login(&pool).await;
    let (_, b) = login(&pool).await;
    assert_eq!(call(&app, "GET", "/api/v1/wishlists", None, None).await.0, 401);

    let (st, h, w) = call(&app, "POST", "/api/v1/wishlists", Some(&a), Some(json!({ "type": "registry", "title": "寶寶清單" }))).await;
    assert_eq!(st, 201, "{w}");
    let slug = w["slug"].as_str().unwrap();
    assert!(slug.len() == 10 && slug.chars().all(|c| c.is_ascii_alphanumeric()));
    assert_eq!(w["status"], "draft");
    let wid = w["id"].as_str().unwrap().to_string();
    assert_eq!(h.get(header::LOCATION).unwrap().to_str().unwrap(), format!("/api/v1/wishlists/{wid}"));
    assert_eq!(call(&app, "POST", "/api/v1/wishlists", Some(&a), Some(json!({ "type": "relief", "title": "x" }))).await.0, 422);
    assert_eq!(call(&app, "POST", "/api/v1/wishlists", Some(&a), Some(json!({ "type": "personal", "title": "x", "surprise_mode": true }))).await.0, 422);

    // 空清單不可發佈
    let (st, _, e) = call(&app, "PATCH", &format!("/api/v1/wishlists/{wid}"), Some(&a), Some(json!({ "status": "active" }))).await;
    assert_eq!((st, e["code"].as_str()), (StatusCode::CONFLICT, Some("WISHLIST_NOT_PUBLISHABLE")));

    let (st, _, it) = call(&app, "POST", &format!("/api/v1/wishlists/{wid}/items"), Some(&a), Some(json!({ "title": "奶瓶", "qty_needed": 3 }))).await;
    assert_eq!(st, 201, "{it}");
    let iid = it["id"].as_str().unwrap().to_string();
    assert_eq!(it["sort_order"], 10);
    assert_eq!(call(&app, "POST", &format!("/api/v1/wishlists/{wid}/items"), Some(&a), Some(json!({ "title": "x", "funding_mode": "crowdfund" }))).await.0, 422);

    // 他人一律 404
    let wu = format!("/api/v1/wishlists/{wid}");
    let iu = format!("/api/v1/items/{iid}");
    for (m, u, body) in [("GET", wu.clone(), None), ("PATCH", wu.clone(), Some(json!({ "title": "hack" }))), ("DELETE", wu.clone(), None),
        ("GET", format!("{wu}/dashboard"), None), ("POST", format!("{wu}/items"), Some(json!({ "title": "x" }))),
        ("PATCH", iu.clone(), Some(json!({ "title": "hack" }))), ("DELETE", iu.clone(), None),
        ("POST", format!("{wu}/items/reorder"), Some(json!({ "item_ids": [iid] })))] {
        let (st, _, e) = call(&app, m, &u, Some(&b), body).await;
        assert_eq!((st, e["code"].as_str()), (StatusCode::NOT_FOUND, Some("NOT_FOUND")), "{m} {u}");
    }
    let (_, _, l) = call(&app, "GET", "/api/v1/wishlists", Some(&b), None).await;
    assert!(l["data"].as_array().unwrap().is_empty());
    // 擁有者標題未被改動
    let (_, _, got) = call(&app, "GET", &wu, Some(&a), None).await;
    assert_eq!(got["wishlist"]["title"], "寶寶清單"); assert_eq!(got["items"][0]["qty_claimed"], 0);

    // qty_needed 不得低於已認領
    sqlx::query("UPDATE wishlist_items SET qty_claimed=2 WHERE id=$1").bind(Uuid::parse_str(&iid).unwrap()).execute(&pool).await.unwrap();
    let (st, _, e) = call(&app, "PATCH", &iu, Some(&a), Some(json!({ "qty_needed": 1 }))).await;
    assert_eq!((st, e["code"].as_str()), (StatusCode::CONFLICT, Some("QTY_BELOW_CLAIMED")));
    assert_eq!(call(&app, "PATCH", &iu, Some(&a), Some(json!({ "qty_needed": 2, "priority": "high" }))).await.0, 200);

    // 有認領的品項刪除需 force
    sqlx::query("INSERT INTO claims (item_id, user_id, claimer_name, qty) VALUES ($1, NULL, 'g', 1)").bind(Uuid::parse_str(&iid).unwrap()).execute(&pool).await.unwrap();
    let (st, _, e) = call(&app, "DELETE", &iu, Some(&a), None).await;
    assert_eq!((st, e["code"].as_str()), (StatusCode::CONFLICT, Some("ITEM_HAS_CLAIMS")));
    // 重排：集合不符 → 422；相符 → 204
    let (_, _, it2) = call(&app, "POST", &format!("{wu}/items"), Some(&a), Some(json!({ "title": "第二個" }))).await;
    let i2 = it2["id"].as_str().unwrap();
    assert_eq!(call(&app, "POST", &format!("{wu}/items/reorder"), Some(&a), Some(json!({ "item_ids": [iid] }))).await.0, 422);
    assert_eq!(call(&app, "POST", &format!("{wu}/items/reorder"), Some(&a), Some(json!({ "item_ids": [i2, iid] }))).await.0, 204);
    let (_, _, got) = call(&app, "GET", &wu, Some(&a), None).await;
    assert_eq!(got["items"][0]["id"], i2);
    assert_eq!(call(&app, "DELETE", &format!("{iu}?force=true"), Some(&a), None).await.0, 204);

    // 發佈 → 關閉 → 修改品項被拒；DELETE 清單 = 封存且冪等
    assert_eq!(call(&app, "PATCH", &wu, Some(&a), Some(json!({ "status": "active" }))).await.1.len() > 0, true);
    let (_, _, w) = call(&app, "GET", &wu, Some(&a), None).await;
    assert_eq!(w["wishlist"]["status"], "active");
    assert_eq!(call(&app, "PATCH", &wu, Some(&a), Some(json!({ "status": "closed" }))).await.0, 200);
    let (st, _, e) = call(&app, "PATCH", &format!("/api/v1/items/{i2}"), Some(&a), Some(json!({ "title": "x" }))).await;
    assert_eq!((st, e["code"].as_str()), (StatusCode::CONFLICT, Some("WISHLIST_CLOSED")));
    assert_eq!(call(&app, "DELETE", &wu, Some(&a), None).await.0, 204);
    assert_eq!(call(&app, "DELETE", &wu, Some(&a), None).await.0, 204);
    let (_, _, l) = call(&app, "GET", "/api/v1/wishlists?status=archived&limit=5", Some(&a), None).await;
    assert_eq!(l["data"][0]["status"], "archived");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn line_login_with_mock_provider(pool: PgPool) {
    let (app, pool) = setup(pool);
    // mock LINE：/token 與 /profile
    let mock = Router::new()
        .route("/token", post(|| async { Json(json!({ "access_token": "at-1" })) }))
        .route("/profile", get(|h: axum::http::HeaderMap| async move {
            assert_eq!(h.get("authorization").unwrap(), "Bearer at-1");
            Json(json!({ "userId": "U-mock-123", "displayName": "LINE小明", "pictureUrl": "https://img.example/p.png" }))
        }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, mock).await.unwrap() });
    // 僅此測試使用 LINE_* 環境變數
    std::env::set_var("LINE_CHANNEL_ID", "chan"); std::env::set_var("LINE_CHANNEL_SECRET", "sec");
    std::env::set_var("LINE_TOKEN_URL", format!("{base}/token")); std::env::set_var("LINE_PROFILE_URL", format!("{base}/profile"));

    let (st, h, _) = call(&app, "GET", "/api/v1/auth/oauth/line/start?redirect=/lists", None, None).await;
    assert_eq!(st, 302);
    let loc = h.get(header::LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(loc.starts_with("https://access.line.me/oauth2/v2.1/authorize?") && loc.contains("code_challenge_method=S256"), "{loc}");
    let state = loc.split("state=").nth(1).unwrap().split('&').next().unwrap().to_string();
    let oc = h.get(header::SET_COOKIE).unwrap().to_str().unwrap().split(';').next().unwrap().to_string();
    assert_eq!(call(&app, "GET", "/api/v1/auth/oauth/line/start?redirect=//evil.com", None, None).await.0, 422);

    // state 不符 → 302 login?error=oauth_failed
    let (st, h, _) = call(&app, "GET", "/api/v1/auth/oauth/line/callback?code=c&state=wrong", Some(&oc), None).await;
    assert_eq!(st, 302); assert!(h.get(header::LOCATION).unwrap().to_str().unwrap().ends_with("/login?error=oauth_failed"));

    let (st, h, _) = call(&app, "GET", &format!("/api/v1/auth/oauth/line/callback?code=c&state={state}"), Some(&oc), None).await;
    assert_eq!(st, 302);
    assert!(h.get(header::LOCATION).unwrap().to_str().unwrap().ends_with("/lists"));
    let sess = h.get_all(header::SET_COOKIE).iter().map(|v| v.to_str().unwrap().to_string()).find(|c| c.starts_with("ws_session=")).unwrap();
    let cookie = sess.split(';').next().unwrap().to_string();
    let (st, _, me) = call(&app, "GET", "/api/v1/me", Some(&cookie), None).await;
    assert_eq!(st, 200); assert_eq!(me["display_name"], "LINE小明"); assert_eq!(me["identities"][0]["provider"], "line");
    // 第二次登入是同一 user
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_identities WHERE provider='line' AND provider_uid='U-mock-123'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
}
