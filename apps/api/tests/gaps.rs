//! 後端缺口：dashboard 彙總 / guest 刪除與找回 / export / 退訂 / owner 操作認領 / 唯讀模式 / 確認信
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
async fn dashboard_locked_totals_and_moderation(pool: PgPool) {
    let (o, t) = user(&pool, false).await;
    let (wid, it) = list(&pool, o, true, 30).await;
    claim(&pool, it, None).await;
    let (_, _, v) = call(&pool, "GET", &format!("/wishlists/{wid}/dashboard"), &ck(&t), None).await;
    assert_eq!(v["totals"]["qty_claimed"], 1);
    assert!(v["items"][0]["qty_claimed"].is_null() && v["claims"].is_null());
    assert_eq!(v["moderation_status"], "ok");
    sqlx::query("UPDATE wishlists SET moderation_status='hidden', moderation_reason='違規' WHERE id=$1").bind(wid).execute(&pool).await.unwrap();
    let (_, _, v) = call(&pool, "GET", &format!("/wishlists/{wid}/dashboard"), &ck(&t), None).await;
    assert_eq!((v["moderation_status"].as_str(), v["moderation_reason"].as_str()), (Some("hidden"), Some("違規")));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn guest_delete_and_recover(pool: PgPool) {
    let (o, _) = user(&pool, false).await;
    let (_, it) = list(&pool, o, false, 0).await;
    let (tok, cid) = claim(&pool, it, Some("a@example.com")).await;
    // 確認信已入佇列
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='claim.confirmation' AND guest_id IS NOT NULL").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    // 找回：一次性、換發新 token、舊 token 失效
    let gid: Uuid = sqlx::query_scalar("SELECT guest_id FROM claims WHERE id=$1").bind(cid).fetch_one(&pool).await.unwrap();
    let r = "recover-token-xyz";
    sqlx::query("INSERT INTO guest_recovery_tokens (guest_id, token_hash, expires_at) VALUES ($1,$2, now() + interval '30 days')").bind(gid).bind(hash_token(r)).execute(&pool).await.unwrap();
    let (s, h, v) = call(&pool, "POST", "/guest/recover", &[], Some(json!({ "token": r }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(h["set-cookie"].to_str().unwrap().starts_with("ws_guest="));
    let new = v["guest_token"].as_str().unwrap().to_string();
    assert_eq!(call(&pool, "GET", "/guest/me", &gt(&tok), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&pool, "GET", "/guest/me", &gt(&new), None).await.0, StatusCode::OK);
    assert_eq!(call(&pool, "POST", "/guest/recover", &[], Some(json!({ "token": r }))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "POST", "/guest/recover", &[], Some(json!({}))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    // 刪除：claims 保留、PII 清除、待發通知取消、token 401
    assert_eq!(call(&pool, "DELETE", "/guest/me", &gt(&new), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(call(&pool, "GET", "/guest/me", &gt(&new), None).await.0, StatusCode::UNAUTHORIZED);
    let (name, em, dn): (String, Option<String>, bool) = sqlx::query_as("SELECT display_name, email, deleted_at IS NOT NULL FROM guests WHERE id=$1").bind(gid).fetch_one(&pool).await.unwrap();
    assert_eq!((name.as_str(), em, dn), ("已刪除的訪客", None, true));
    let cn: String = sqlx::query_scalar("SELECT claimer_name FROM claims WHERE id=$1").bind(cid).fetch_one(&pool).await.unwrap();
    assert_eq!(cn, "已刪除的訪客");
    let pend: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE guest_id=$1 AND status='pending'").bind(gid).fetch_one(&pool).await.unwrap();
    assert_eq!(pend, 0);
    // 已刪除訪客的權杖不可再用
    sqlx::query("INSERT INTO guest_recovery_tokens (guest_id, token_hash, expires_at) VALUES ($1,$2, now() + interval '30 days')").bind(gid).bind(hash_token("again")).execute(&pool).await.unwrap();
    assert_eq!(call(&pool, "POST", "/guest/recover", &[], Some(json!({ "token": "again" }))).await.0, StatusCode::NOT_FOUND);
}

#[test]
fn confirmation_mail_has_recovery_and_unsubscribe_links() {
    let p = json!({ "recovery_token": "TOK", "unsubscribe_url": "http://x/u?token=1" });
    let (_, body) = wishsync_api::notify::render("claim.confirmation", "", &p);
    assert!(body.contains("/me/claims#r=TOK") && body.contains("http://x/u?token=1"), "{body}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn no_confirmation_without_email(pool: PgPool) {
    let (o, _) = user(&pool, false).await;
    let (_, it) = list(&pool, o, false, 0).await;
    claim(&pool, it, None).await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='claim.confirmation'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn export_and_unsubscribe(pool: PgPool) {
    let (u, t) = user(&pool, false).await;
    let (_, it) = list(&pool, u, false, 0).await;
    claim(&pool, it, None).await;
    let (s, h, v) = call(&pool, "GET", "/me/export", &ck(&t), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h["content-disposition"].to_str().unwrap().contains("wishsync-export-"));
    assert_eq!((v["wishlists"][0]["items"][0]["title"].as_str(), v["claims"].as_array().map(Vec::len)), (Some("奶瓶"), Some(0)));
    assert_eq!(call(&pool, "GET", "/me/export", &[], None).await.0, StatusCode::UNAUTHORIZED);
    call(&pool, "GET", "/me/export", &ck(&t), None).await;
    call(&pool, "GET", "/me/export", &ck(&t), None).await;
    assert_eq!(call(&pool, "GET", "/me/export", &ck(&t), None).await.0, StatusCode::TOO_MANY_REQUESTS);

    // 退訂：建立者。GET 只轉址、不變更狀態；POST 才退訂
    sqlx::query("INSERT INTO notifications (user_id, channel, kind) VALUES ($1,'email','claim.digest')").bind(u).execute(&pool).await.unwrap();
    let tok = wishsync_api::account::unsub_token('u', u);
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id=$1 AND status='pending'").bind(u).fetch_one(&pool).await.unwrap();
    let (s, h, _) = call(&pool, "GET", &format!("/unsubscribe?token={tok}"), &[], None).await;
    assert_eq!(s, StatusCode::FOUND);
    assert!(h["location"].to_str().unwrap().ends_with(&format!("/unsubscribe?token={tok}")));
    let pend: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id=$1 AND status='pending'").bind(u).fetch_one(&pool).await.unwrap();
    assert_eq!(pend, before, "GET 不得變更狀態");
    for _ in 0..2 {
        assert_eq!(call(&pool, "POST", "/unsubscribe", &[], Some(json!({ "token": tok }))).await.0, StatusCode::NO_CONTENT);
    }
    let off: bool = sqlx::query_scalar("SELECT (notification_prefs->>'email_claims')::boolean = false FROM users WHERE id=$1").bind(u).fetch_one(&pool).await.unwrap();
    let pend: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id=$1 AND status='pending'").bind(u).fetch_one(&pool).await.unwrap();
    assert!(off && pend == 0);
    // 訪客：清 email
    let g: Uuid = sqlx::query_scalar("INSERT INTO guests (guest_token_hash, display_name, email) VALUES ($1,'g','g@example.com') RETURNING id").bind(hash_token("g")).fetch_one(&pool).await.unwrap();
    let gtok = wishsync_api::account::unsub_token('g', g);
    call(&pool, "GET", &format!("/unsubscribe?token={gtok}"), &[], None).await;
    assert!(sqlx::query_scalar::<_, Option<String>>("SELECT email FROM guests WHERE id=$1").bind(g).fetch_one(&pool).await.unwrap().is_some());
    assert_eq!(call(&pool, "POST", "/unsubscribe", &[], Some(json!({ "token": gtok }))).await.0, StatusCode::NO_CONTENT);
    let em: Option<String> = sqlx::query_scalar("SELECT email FROM guests WHERE id=$1").bind(g).fetch_one(&pool).await.unwrap();
    assert!(em.is_none());
    // 無效 / 竄改 → POST 422 INVALID_TOKEN；GET 仍只轉址
    for bad in [format!("{tok}x"), "garbage".into(), String::new()] {
        let (s, _, v) = call(&pool, "POST", "/unsubscribe", &[], Some(json!({ "token": bad }))).await;
        assert!(s == StatusCode::UNPROCESSABLE_ENTITY && v["code"] == "INVALID_TOKEN");
        assert_eq!(call(&pool, "GET", &format!("/unsubscribe?token={bad}"), &[], None).await.0, StatusCode::FOUND);
    }
    assert_eq!(call(&pool, "POST", "/unsubscribe", &[], Some(json!({}))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn owner_delivers_and_cancels_claims(pool: PgPool) {
    let (o, ot) = user(&pool, false).await;
    let (_, st) = user(&pool, false).await;
    let (wid, it) = list(&pool, o, false, 0).await;
    let (_, c1) = claim(&pool, it, None).await;
    let (_, c2) = claim(&pool, it, None).await;
    let url = |c: Uuid| format!("/claims/{c}");
    // 路人 / 擁有者改數量或 purchased → 403
    assert_eq!(call(&pool, "PATCH", &url(c1), &ck(&st), Some(json!({"status":"delivered"}))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(call(&pool, "PATCH", &url(c1), &ck(&ot), Some(json!({"status":"purchased"}))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(call(&pool, "PATCH", &url(c1), &ck(&ot), Some(json!({"qty":2}))).await.0, StatusCode::FORBIDDEN);
    // 推進 delivered（qty_claimed 不變）
    let (s, _, v) = call(&pool, "PATCH", &url(c1), &ck(&ot), Some(json!({"status":"delivered"}))).await;
    assert_eq!((s, v["claim"]["status"].as_str(), v["item"]["qty_claimed"].as_i64()), (StatusCode::OK, Some("delivered"), Some(2)), "{v}");
    // delivered 不可取消
    assert_eq!(call(&pool, "DELETE", &url(c1), &ck(&ot), None).await.0, StatusCode::CONFLICT);
    // 取消別人的認領 → 回補、audit
    assert_eq!(call(&pool, "DELETE", &url(c2), &ck(&ot), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(call(&pool, "DELETE", &url(c2), &ck(&ot), None).await.0, StatusCode::NO_CONTENT);
    let q: i32 = sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id=$1").bind(it).fetch_one(&pool).await.unwrap();
    assert_eq!(q, 1);
    let a: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE actor_type='user' AND actor_id=$1 AND action='claim.owner_update'").bind(o).fetch_one(&pool).await.unwrap();
    assert_eq!(a, 2);
    // 驚喜鎖定期間禁止
    let (_, it2) = list(&pool, o, true, 30).await;
    let (_, c3) = claim(&pool, it2, None).await;
    assert_eq!(call(&pool, "DELETE", &url(c3), &ck(&ot), None).await.0, StatusCode::FORBIDDEN);
    let _ = wid;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn read_only_mode_blocks_writes(pool: PgPool) {
    let (_, staff) = user(&pool, true).await;
    let (o, ot) = user(&pool, false).await;
    let (_, it) = list(&pool, o, false, 0).await;
    let (_, _, v) = call(&pool, "GET", "/admin/system-flags", &ck(&staff), None).await;
    assert_eq!(v["data"][0]["key"], "read_only");
    assert_eq!(v["data"][0]["value"], false);
    assert_eq!(call(&pool, "GET", "/admin/system-flags", &ck(&ot), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(call(&pool, "PUT", "/admin/system-flags/read_only", &ck(&staff), Some(json!({"value": true}))).await.0, StatusCode::OK);
    let (s, h, v) = call(&pool, "POST", &format!("/items/{it}/claims"), &[("idempotency-key", Uuid::new_v4().to_string())], Some(json!({"qty":1,"display_name":"x"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("READ_ONLY_MODE")));
    assert!(h.contains_key("retry-after"));
    assert_eq!(call(&pool, "DELETE", "/me", &ck(&ot), Some(json!({"confirm":"DELETE"}))).await.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(call(&pool, "GET", "/me/export", &ck(&ot), None).await.0, StatusCode::OK);
    assert_eq!(call(&pool, "POST", "/auth/logout", &ck(&ot), None).await.0, StatusCode::NO_CONTENT);
    // 可關閉
    assert_eq!(call(&pool, "PUT", "/admin/system-flags/read_only", &ck(&staff), Some(json!({"value": false}))).await.0, StatusCode::OK);
    assert_eq!(claim_status(&pool, it).await, StatusCode::CREATED);
}

async fn claim_status(pool: &PgPool, it: Uuid) -> StatusCode {
    call(pool, "POST", &format!("/items/{it}/claims"), &[("idempotency-key", Uuid::new_v4().to_string())], Some(json!({"qty":1,"display_name":"x"}))).await.0
}
