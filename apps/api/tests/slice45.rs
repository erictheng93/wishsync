use axum::{body::Body, http::{Request, StatusCode}, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wishsync_api::{app, session::hash_token, AppState};

fn router(pool: &PgPool) -> Router { app(AppState { pool: pool.clone() }) }

async fn call(pool: &PgPool, method: &str, uri: &str, cookie: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut r = Request::builder().method(method).uri(format!("/api/v1{uri}"));
    if let Some(c) = cookie { r = r.header("cookie", format!("ws_session={c}")); }
    let req = if let Some(b) = body { r.header("content-type", "application/json").body(Body::from(b.to_string())) } else { r.body(Body::empty()) }.unwrap();
    let res = router(pool).oneshot(req).await.unwrap();
    let st = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// 回傳 (user_id, session token)
async fn user(pool: &PgPool, name: &str, staff: bool) -> (Uuid, String) {
    let id: Uuid = sqlx::query_scalar("INSERT INTO users (display_name, email, is_staff) VALUES ($1, $2, $3) RETURNING id")
        .bind(name).bind(format!("{}@example.com", Uuid::new_v4())).bind(staff).fetch_one(pool).await.unwrap();
    let tok = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(id).bind(hash_token(&tok)).execute(pool).await.unwrap();
    (id, tok)
}

async fn wishlist(pool: &PgPool, owner: Uuid, surprise: bool, event_offset_days: i32) -> (Uuid, String, Uuid) {
    let slug: String = Uuid::new_v4().simple().to_string()[..10].to_string();
    let wid: Uuid = sqlx::query_scalar(
        "INSERT INTO wishlists (owner_id, type, status, slug, title, surprise_mode, event_date)
         VALUES ($1, 'registry', 'active', $2, '寶寶清單', $3, current_date + $4::int) RETURNING id")
        .bind(owner).bind(&slug).bind(surprise).bind(event_offset_days).fetch_one(pool).await.unwrap();
    let item: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed, qty_claimed) VALUES ($1, '玻璃奶瓶', 2, 1) RETURNING id")
        .bind(wid).fetch_one(pool).await.unwrap();
    let g: Uuid = sqlx::query_scalar("INSERT INTO guests (guest_token_hash, display_name, contact) VALUES ($1, '秘密客', 'line:secret') RETURNING id")
        .bind(hash_token(&Uuid::new_v4().to_string())).fetch_one(pool).await.unwrap();
    sqlx::query("INSERT INTO claims (item_id, guest_id, claimer_name, qty) VALUES ($1, $2, '秘密客', 1)").bind(item).bind(g).execute(pool).await.unwrap();
    (wid, slug, item)
}

const M: &str = "../../db/migrations";

#[sqlx::test(migrations = "../../db/migrations")]
async fn surprise_dashboard_hides_claimers(pool: PgPool) {
    let _ = M;
    let (owner, tok) = user(&pool, "owner", false).await;
    let (stranger, stok) = user(&pool, "x", false).await;
    let _ = stranger;
    let (wid, _, _) = wishlist(&pool, owner, true, 30).await;
    let (s, v) = call(&pool, "GET", &format!("/wishlists/{wid}/dashboard"), Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK);
    let raw = v.to_string();
    assert!(!raw.contains("秘密客") && !raw.contains("line:secret"), "leak: {raw}");
    assert_eq!(v["surprise_locked"], true);
    assert!(v["claims"].is_null() && v["items"][0]["qty_claimed"].is_null());
    assert_eq!(v["totals"]["completion_pct"], 50); // 數量計 1/2（totals.qty_claimed 本就輸出）
    // 非擁有者 → 404
    assert_eq!(call(&pool, "GET", &format!("/wishlists/{wid}/dashboard"), Some(&stok), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "GET", &format!("/wishlists/{wid}/dashboard"), None, None).await.0, StatusCode::UNAUTHORIZED);
    // 活動日已過 → 解鎖，看得到認領者
    sqlx::query("UPDATE wishlists SET event_date = current_date - 1 WHERE id = $1").bind(wid).execute(&pool).await.unwrap();
    let (_, v) = call(&pool, "GET", &format!("/wishlists/{wid}/dashboard"), Some(&tok), None).await;
    assert_eq!(v["surprise_locked"], false);
    assert_eq!(v["claims"].as_array().map(|a| a.len()), Some(1));
    assert_eq!(v["items"][0]["qty_claimed"], 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn staff_guard_report_and_moderation(pool: PgPool) {
    let (owner, _) = user(&pool, "owner", false).await;
    let (_, normal) = user(&pool, "n", false).await;
    let (staff_id, staff) = user(&pool, "s", true).await;
    let (wid, slug, _) = wishlist(&pool, owner, false, 30).await;

    for uri in ["/admin/reports", "/admin/wishlists", "/admin/users", "/admin/stats"] {
        let (s, v) = call(&pool, "GET", uri, Some(&normal), None).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("STAFF_ONLY")), "{uri}");
        assert_eq!(call(&pool, "GET", uri, None, None).await.0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(call(&pool, "PUT", "/admin/system-flags/read_only", Some(&normal), Some(json!({"value": true}))).await.0, StatusCode::FORBIDDEN);

    // 公開檢舉
    let (s, r) = call(&pool, "POST", &format!("/public/wishlists/{slug}/reports"), None, Some(json!({"reason": "scam", "detail": "私下轉帳"}))).await;
    assert_eq!(s, StatusCode::CREATED);
    let rid = r["id"].as_str().unwrap().to_string();
    assert_eq!(call(&pool, "POST", &format!("/public/wishlists/{slug}/reports"), None, Some(json!({"reason": "bogus"}))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(call(&pool, "POST", "/public/wishlists/nonexistent/reports", None, Some(json!({"reason": "scam"}))).await.0, StatusCode::NOT_FOUND);

    let (s, q) = call(&pool, "GET", "/admin/reports", Some(&staff), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(q["data"][0]["wishlist"]["slug"], slug.as_str());
    assert_eq!(q["data"][0]["reporter"], "anonymous");

    // 下架：reason 必填
    let uri = format!("/admin/wishlists/{wid}/moderation");
    assert_eq!(call(&pool, "PATCH", &uri, Some(&staff), Some(json!({"moderation_status": "hidden"}))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, v) = call(&pool, "PATCH", &uri, Some(&staff), Some(json!({"moderation_status": "hidden", "reason": "疑似詐騙"}))).await;
    assert_eq!((s, v["moderation_status"].as_str()), (StatusCode::OK, Some("hidden")));
    let (ms, rs): (String, String) = sqlx::query_as("SELECT w.moderation_status::text, r.status::text FROM wishlists w, content_reports r WHERE w.id = $1 AND r.id = $2")
        .bind(wid).bind(rid.parse::<Uuid>().unwrap()).fetch_one(&pool).await.unwrap();
    assert_eq!((ms.as_str(), rs.as_str()), ("hidden", "actioned"));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE actor_type = 'staff' AND actor_id = $1 AND action = 'wishlist.moderate' AND entity_id = $2")
        .bind(staff_id).bind(wid).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    // 下架後再檢舉 → 410；已處理檢舉再改 → 409
    assert_eq!(call(&pool, "POST", &format!("/public/wishlists/{slug}/reports"), None, Some(json!({"reason": "scam"}))).await.0, StatusCode::GONE);
    let (s, v) = call(&pool, "PATCH", &format!("/admin/reports/{rid}"), Some(&staff), Some(json!({"status": "dismissed"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION")));
    // 復原
    let (_, v) = call(&pool, "PATCH", &uri, Some(&staff), Some(json!({"moderation_status": "ok"}))).await;
    assert_eq!(v["moderation_status"], "ok");
    assert!(v["moderation_reason"].is_null());

    // 搜尋 + system flag + stats
    let (_, v) = call(&pool, "GET", &format!("/admin/wishlists?q={slug}"), Some(&staff), None).await;
    assert_eq!(v["data"][0]["id"], wid.to_string());
    let (s, v) = call(&pool, "PUT", "/admin/system-flags/read_only", Some(&staff), Some(json!({"value": true}))).await;
    assert_eq!((s, &v["value"]), (StatusCode::OK, &json!(true)));
    assert_eq!(call(&pool, "PUT", "/admin/system-flags/nope", Some(&staff), Some(json!({"value": true}))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "PUT", "/admin/system-flags/read_only", Some(&staff), Some(json!({"value": "x"}))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, v) = call(&pool, "GET", "/admin/stats", Some(&staff), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["flags"]["read_only"], true);
    assert_eq!(v["claims_by_status"]["reserved"], 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_delete_anonymizes(pool: PgPool) {
    let (uid, tok) = user(&pool, "小米", false).await;
    let (wid, slug, item) = wishlist(&pool, uid, false, 30).await;
    // 此使用者在別人清單上的認領
    let (other, _) = user(&pool, "o", false).await;
    let (_, _, item2) = wishlist(&pool, other, false, 30).await;
    sqlx::query("INSERT INTO claims (item_id, user_id, claimer_name, qty) VALUES ($1, $2, '小米', 1)").bind(item2).bind(uid).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id, provider, provider_uid) VALUES ($1, 'email', 'a@b.c')").bind(uid).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO notifications (user_id, channel, kind) VALUES ($1, 'email', 'claim.digest')").bind(uid).execute(&pool).await.unwrap();
    let _ = (slug, item);

    assert_eq!(call(&pool, "DELETE", "/me", Some(&tok), Some(json!({"confirm": "yes"}))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, v) = call(&pool, "DELETE", "/me", Some(&tok), Some(json!({"confirm": "DELETE"}))).await;
    assert_eq!((s, &v["deleted"]), (StatusCode::OK, &json!(true)));

    let (name, email, del, anon): (String, Option<String>, bool, bool) = sqlx::query_as(
        "SELECT display_name, email, deleted_at IS NOT NULL, anonymized_at IS NOT NULL FROM users WHERE id = $1").bind(uid).fetch_one(&pool).await.unwrap();
    assert_eq!((name.as_str(), email, del, anon), ("已刪除的使用者", None, true, true));
    let (st, vis): (String, String) = sqlx::query_as("SELECT status::text, visibility::text FROM wishlists WHERE id = $1").bind(wid).fetch_one(&pool).await.unwrap();
    assert_eq!((st.as_str(), vis.as_str()), ("archived", "private"));
    let cn: String = sqlx::query_scalar("SELECT claimer_name FROM claims WHERE user_id = $1").bind(uid).fetch_one(&pool).await.unwrap();
    assert_eq!(cn, "已刪除的使用者");
    let idn: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_identities WHERE user_id = $1").bind(uid).fetch_one(&pool).await.unwrap();
    assert_eq!(idn, 0);
    let pend: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id = $1 AND status = 'pending'").bind(uid).fetch_one(&pool).await.unwrap();
    assert_eq!(pend, 0);
    // session 失效
    assert_eq!(call(&pool, "GET", "/admin/stats", Some(&tok), None).await.0, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn enqueue_claim_first_immediate_then_digest(pool: PgPool) {
    let (owner, _) = user(&pool, "o", false).await;
    let (wid, _, _) = wishlist(&pool, owner, true, 30).await;
    for _ in 0..3 { wishsync_api::notify::enqueue_claim(&pool, wid).await.unwrap(); }
    let rows: Vec<(String, i32, bool)> = sqlx::query_as(
        "SELECT kind, (payload->>'count')::int, scheduled_at > now() + interval '1 second' FROM notifications WHERE user_id = $1 ORDER BY kind").bind(owner).fetch_all(&pool).await.unwrap();
    assert_eq!(rows, vec![("claim.created".to_string(), 1, false), ("claim.digest".to_string(), 2, true)]);
    // 關閉通知則不寫
    sqlx::query("UPDATE users SET notification_prefs = '{\"email_claims\": false}' WHERE id = $1").bind(owner).execute(&pool).await.unwrap();
    wishsync_api::notify::enqueue_claim(&pool, wid).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 2);
    let (_, body) = wishsync_api::notify::render("claim.digest", "清單", &json!({"count": 2}));
    assert!(body.contains("2 件新認領"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn sse_pushes_item_update_and_410_when_hidden(pool: PgPool) {
    let (owner, _) = user(&pool, "o", false).await;
    let (wid, slug, item) = wishlist(&pool, owner, false, 30).await;
    let req = Request::builder().uri(format!("/api/v1/public/wishlists/{slug}/events")).body(Body::empty()).unwrap();
    let res = router(&pool).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "text/event-stream");
    let mut body = res.into_body();
    tokio::time::sleep(std::time::Duration::from_millis(800)).await; // 等 listener LISTEN
    sqlx::query("UPDATE wishlist_items SET qty_claimed = 2 WHERE id = $1").bind(item).execute(&pool).await.unwrap();
    wishsync_api::dashboard::notify(&pool, wid).await.unwrap();
    let mut got = String::new();
    while !got.contains("wishlist.updated") {
        let f = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame()).await.expect("sse timeout").unwrap().unwrap();
        got.push_str(&String::from_utf8_lossy(f.data_ref().unwrap()));
    }
    assert!(got.contains("event: item.updated") && got.contains("\"qty_claimed\":2") && got.contains("\"is_fully_claimed\":true"), "{got}");
    assert!(!got.contains("秘密客"));
    // 下架 → 新連線 410
    sqlx::query("UPDATE wishlists SET moderation_status='hidden', moderation_reason='x' WHERE id=$1").bind(wid).execute(&pool).await.unwrap();
    let req = Request::builder().uri(format!("/api/v1/public/wishlists/{slug}/events")).body(Body::empty()).unwrap();
    assert_eq!(router(&pool).oneshot(req).await.unwrap().status(), StatusCode::GONE);
}

/// 需 Mailpit：docker compose up -d mailpit；scripts/cargo.sh test -- --include-ignored
#[sqlx::test(migrations = "../../db/migrations")]
#[ignore]
async fn worker_sends_via_smtp(pool: PgPool) {
    if std::env::var("SMTP_HOST").is_err() { std::env::set_var("SMTP_HOST", "host.docker.internal"); }
    let (owner, _) = user(&pool, "o", false).await;
    let (wid, _, _) = wishlist(&pool, owner, true, 30).await;
    wishsync_api::notify::enqueue_claim(&pool, wid).await.unwrap();
    assert_eq!(wishsync_api::notify::tick(&pool).await.unwrap(), 1);
    let (st, err): (String, Option<String>) = sqlx::query_as("SELECT status::text, last_error FROM notifications").fetch_one(&pool).await.unwrap();
    assert_eq!((st.as_str(), err), ("sent", None));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unified_pct_owner_names_list_fields_and_image_base(pool: PgPool) {
    let (owner, tok) = user(&pool, "owner", false).await;
    let (wid, slug, _) = wishlist(&pool, owner, false, -1).await; // 1/2 claimed, show_claimer_names 預設 false
    // 第二品項 1/3（數量計 2/5=40；以品項計會是 0 或 50）；soft-deleted 品項不計
    sqlx::query("INSERT INTO wishlist_items (wishlist_id, title, qty_needed, qty_claimed) VALUES ($1, 'b', 3, 1)").bind(wid).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO wishlist_items (wishlist_id, title, qty_needed, qty_claimed, deleted_at) VALUES ($1, 'gone', 100, 0, now())").bind(wid).execute(&pool).await.unwrap();
    sqlx::query("UPDATE wishlists SET cover_image_key = 'covers/x.jpg', cover_image_status = 'ready', moderation_status = 'ok' WHERE id = $1").bind(wid).execute(&pool).await.unwrap();

    let (_, d) = call(&pool, "GET", &format!("/wishlists/{wid}/dashboard"), Some(&tok), None).await;
    assert_eq!(d["claims"][0]["claimer_name"], "秘密客");
    assert_eq!(d["totals"]["completion_pct"], 40);

    let (_, l) = call(&pool, "GET", "/wishlists", Some(&tok), None).await;
    assert_eq!(l["data"][0]["completion"]["completion_pct"], 40);
    assert_eq!(l["data"][0]["moderation_status"], "ok");
    assert!(l["data"][0].get("moderation_reason").is_some());

    let (_, p) = call(&pool, "GET", &format!("/public/wishlists/{slug}"), None, None).await;
    assert_eq!(p["completion"]["completion_pct"], 40);
    assert_eq!(p["cover_image_url"], wishsync_api::uploads::S3::get().public_url("covers/x.jpg"));
    assert_eq!(wishsync_api::wishlists::completion_pct(0, 0), 0);
}
