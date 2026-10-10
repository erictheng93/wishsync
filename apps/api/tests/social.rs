//! 社群：好友（申請/邀請/解除）、個人頁清單與捐助可見性、handle 與認領 visibility
use axum::{body::Body, http::{Request, StatusCode}};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wishsync_api::{app, session::hash_token, AppState};

async fn call(pool: &PgPool, method: &str, uri: &str, cookie: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut r = Request::builder().method(method).uri(format!("/api/v1{uri}")).header("idempotency-key", Uuid::new_v4().to_string());
    if let Some(c) = cookie { r = r.header("cookie", format!("ws_session={c}")); }
    let req = match body { Some(b) => r.header("content-type", "application/json").body(Body::from(b.to_string())), None => r.body(Body::empty()) }.unwrap();
    let res = app(AppState { pool: pool.clone() }).oneshot(req).await.unwrap();
    let st = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// (id, session token, email)
async fn user(pool: &PgPool, name: &str, handle: Option<&str>) -> (Uuid, String, String) {
    let email = format!("{}@example.com", Uuid::new_v4());
    let id: Uuid = sqlx::query_scalar("INSERT INTO users (display_name, email, handle) VALUES ($1, $2, $3) RETURNING id")
        .bind(name).bind(&email).bind(handle).fetch_one(pool).await.unwrap();
    let tok = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(id).bind(hash_token(&tok)).execute(pool).await.unwrap();
    (id, tok, email)
}

async fn befriend(pool: &PgPool, a: Uuid, b: Uuid) {
    let (x, y) = if a < b { (a, b) } else { (b, a) };
    sqlx::query("INSERT INTO friendships (user_a, user_b) VALUES ($1, $2)").bind(x).bind(y).execute(pool).await.unwrap();
}

/// 清單 + 一個品項；回 (wishlist_id, item_id)
async fn list(pool: &PgPool, owner: Uuid, vis: &str, surprise: bool) -> (Uuid, Uuid) {
    let slug: String = Uuid::new_v4().simple().to_string()[..10].to_string();
    let wid: Uuid = sqlx::query_scalar("INSERT INTO wishlists (owner_id, type, status, slug, title, visibility, surprise_mode, event_date)
        VALUES ($1, 'registry', 'active', $2, $3, $4::text::visibility, $5, current_date + 30) RETURNING id")
        .bind(owner).bind(&slug).bind(format!("清單-{vis}")).bind(vis).bind(surprise).fetch_one(pool).await.unwrap();
    let it: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, '奶瓶', 3) RETURNING id").bind(wid).fetch_one(pool).await.unwrap();
    (wid, it)
}

/// 直接寫入使用者認領
async fn user_claim(pool: &PgPool, item: Uuid, user: Uuid, vis: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO claims (item_id, user_id, claimer_name, qty, visibility) VALUES ($1, $2, 'x', 1, $3::text::share_level) RETURNING id")
        .bind(item).bind(user).bind(vis).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn friend_request_flow_and_no_enumeration(pool: PgPool) {
    let (a, ta, _) = user(&pool, "A", Some("alice")).await;
    let (b, tb, eb) = user(&pool, "B", Some("bobby")).await;
    // 不存在的帳號與存在的帳號回應相同
    assert_eq!(call(&pool, "POST", "/friends/requests", Some(&ta), Some(json!({ "to": "nobody@example.com" }))).await.0, StatusCode::ACCEPTED);
    assert_eq!(call(&pool, "POST", "/friends/requests", Some(&ta), Some(json!({ "to": "ghost" }))).await.0, StatusCode::ACCEPTED);
    assert_eq!(call(&pool, "POST", "/friends/requests", Some(&ta), Some(json!({ "to": "alice" }))).await.0, StatusCode::ACCEPTED); // 對自己
    assert_eq!(call(&pool, "POST", "/friends/requests", Some(&ta), Some(json!({ "to": eb.to_uppercase() }))).await.0, StatusCode::ACCEPTED);
    assert_eq!(call(&pool, "POST", "/friends/requests", Some(&ta), Some(json!({ "to": "@bobby" }))).await.0, StatusCode::ACCEPTED); // 重複
    assert_eq!(call(&pool, "POST", "/friends/requests", None, Some(json!({ "to": "bobby" }))).await.0, StatusCode::UNAUTHORIZED);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM friend_requests").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    let (_, v) = call(&pool, "GET", "/friends", Some(&tb), None).await;
    assert_eq!(v["incoming"].as_array().unwrap().len(), 1);
    assert_eq!(v["incoming"][0]["user"]["handle"], "alice");
    let (_, v2) = call(&pool, "GET", "/friends", Some(&ta), None).await;
    assert_eq!(v2["outgoing"].as_array().unwrap().len(), 1);
    // 寄件者不能自己接受
    let rid = v["incoming"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(call(&pool, "POST", &format!("/friends/requests/{rid}/accept"), Some(&ta), None).await.0, StatusCode::NOT_FOUND);
    let (s, v) = call(&pool, "POST", &format!("/friends/requests/{rid}/accept"), Some(&tb), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["friend"]["id"], json!(a));
    assert!(crate_are_friends(&pool, a, b).await);
    let (_, v) = call(&pool, "GET", "/friends", Some(&ta), None).await;
    assert_eq!(v["friends"][0]["handle"], "bobby");
    assert!(v["outgoing"].as_array().unwrap().is_empty());
    // 已是好友再申請：不動作
    assert_eq!(call(&pool, "POST", "/friends/requests", Some(&ta), Some(json!({ "to": "bobby" }))).await.0, StatusCode::ACCEPTED);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM friend_requests").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0);
}

async fn crate_are_friends(pool: &PgPool, a: Uuid, b: Uuid) -> bool { wishsync_api::access::are_friends(pool, a, b).await.unwrap() }

#[sqlx::test(migrations = "../../db/migrations")]
async fn mutual_request_auto_friends_and_decline_withdraw(pool: PgPool) {
    let (a, ta, _) = user(&pool, "A", Some("alice")).await;
    let (b, tb, _) = user(&pool, "B", Some("bobby")).await;
    let (_, tc, _) = user(&pool, "C", Some("carol")).await;
    call(&pool, "POST", "/friends/requests", Some(&ta), Some(json!({ "to": "bobby" }))).await;
    call(&pool, "POST", "/friends/requests", Some(&tb), Some(json!({ "to": "alice" }))).await;
    assert!(crate_are_friends(&pool, a, b).await);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM friend_requests").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0);
    // 拒絕 / 撤回
    call(&pool, "POST", "/friends/requests", Some(&tc), Some(json!({ "to": "alice" }))).await;
    let (_, v) = call(&pool, "GET", "/friends", Some(&tc), None).await;
    let rid = v["outgoing"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(call(&pool, "DELETE", &format!("/friends/requests/{rid}"), Some(&tb), None).await.0, StatusCode::NOT_FOUND); // 無關第三人
    assert_eq!(call(&pool, "DELETE", &format!("/friends/requests/{rid}"), Some(&tc), None).await.0, StatusCode::NO_CONTENT); // 撤回
    call(&pool, "POST", "/friends/requests", Some(&tc), Some(json!({ "to": "alice" }))).await;
    let (_, v) = call(&pool, "GET", "/friends", Some(&ta), None).await;
    let rid = v["incoming"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(call(&pool, "DELETE", &format!("/friends/requests/{rid}"), Some(&ta), None).await.0, StatusCode::NO_CONTENT); // 拒絕
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn friend_request_rate_limited(pool: PgPool) {
    let (_, ta, _) = user(&pool, "A", Some("alice")).await;
    for _ in 0..30 { assert_eq!(call(&pool, "POST", "/friends/requests", Some(&ta), Some(json!({ "to": "ghost" }))).await.0, StatusCode::ACCEPTED); }
    assert_eq!(call(&pool, "POST", "/friends/requests", Some(&ta), Some(json!({ "to": "ghost" }))).await.0, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn invite_links(pool: PgPool) {
    let (a, ta, _) = user(&pool, "A", Some("alice")).await;
    let (b, tb, _) = user(&pool, "B", Some("bobby")).await;
    let (s, v) = call(&pool, "POST", "/friends/invites", Some(&ta), None).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let tok = v["token"].as_str().unwrap().to_string();
    assert!(v["url"].as_str().unwrap().ends_with(&format!("/invite/{tok}")));
    // 預覽免登入
    let (s, p) = call(&pool, "GET", &format!("/friends/invites/{tok}"), None, None).await;
    assert_eq!((s, p["inviter"]["handle"].as_str()), (StatusCode::OK, Some("alice")));
    assert_eq!(call(&pool, "GET", "/friends/invites/nope", None, None).await.0, StatusCode::NOT_FOUND);
    // 自己的邀請
    let (s, e) = call(&pool, "POST", &format!("/friends/invites/{tok}/accept"), Some(&ta), None).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("SELF_INVITE")));
    assert_eq!(call(&pool, "POST", &format!("/friends/invites/{tok}/accept"), None, None).await.0, StatusCode::UNAUTHORIZED);
    // 接受（含雙向待處理申請被清掉），重複接受仍 200
    call(&pool, "POST", "/friends/requests", Some(&tb), Some(json!({ "to": "alice" }))).await;
    let (s, v) = call(&pool, "POST", &format!("/friends/invites/{tok}/accept"), Some(&tb), None).await;
    assert_eq!((s, v["friend"]["id"].clone()), (StatusCode::OK, json!(a)));
    assert!(crate_are_friends(&pool, a, b).await);
    assert_eq!(call(&pool, "POST", &format!("/friends/invites/{tok}/accept"), Some(&tb), None).await.0, StatusCode::OK);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM friend_requests").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0);
    // 新邀請撤銷舊的
    let (_, v2) = call(&pool, "POST", "/friends/invites", Some(&ta), None).await;
    assert_eq!(call(&pool, "GET", &format!("/friends/invites/{tok}"), None, None).await.0, StatusCode::NOT_FOUND);
    let tok2 = v2["token"].as_str().unwrap();
    assert_eq!(call(&pool, "GET", &format!("/friends/invites/{tok2}"), None, None).await.0, StatusCode::OK);
    // 撤銷全部
    assert_eq!(call(&pool, "DELETE", "/friends/invites", Some(&ta), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(call(&pool, "GET", &format!("/friends/invites/{tok2}"), None, None).await.0, StatusCode::NOT_FOUND);
    // 過期
    let (_, v3) = call(&pool, "POST", "/friends/invites", Some(&ta), None).await;
    let tok3 = v3["token"].as_str().unwrap();
    sqlx::query("UPDATE friend_invites SET expires_at = now() - interval '1 second'").execute(&pool).await.unwrap();
    assert_eq!(call(&pool, "GET", &format!("/friends/invites/{tok3}"), None, None).await.0, StatusCode::NOT_FOUND);
    let (_, tc, _) = user(&pool, "C", None).await;
    assert_eq!(call(&pool, "POST", &format!("/friends/invites/{tok3}/accept"), Some(&tc), None).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unfriend_clears_allowed_users(pool: PgPool) {
    let (a, ta, _) = user(&pool, "A", Some("alice")).await;
    let (b, _, _) = user(&pool, "B", Some("bobby")).await;
    let (c, _, _) = user(&pool, "C", Some("carol")).await;
    befriend(&pool, a, b).await;
    befriend(&pool, a, c).await;
    let (wa, _) = list(&pool, a, "selected", false).await;
    let (wb, _) = list(&pool, b, "selected", false).await;
    for (w, u) in [(wa, b), (wa, c), (wb, a)] {
        sqlx::query("INSERT INTO wishlist_allowed_users (wishlist_id, user_id) VALUES ($1, $2)").bind(w).bind(u).execute(&pool).await.unwrap();
    }
    assert_eq!(call(&pool, "DELETE", &format!("/friends/{b}"), Some(&ta), None).await.0, StatusCode::NO_CONTENT);
    assert!(!crate_are_friends(&pool, a, b).await && crate_are_friends(&pool, a, c).await);
    let left: Vec<(Uuid, Uuid)> = sqlx::query_as("SELECT wishlist_id, user_id FROM wishlist_allowed_users").fetch_all(&pool).await.unwrap();
    assert_eq!(left, vec![(wa, c)]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn profile_lists_visibility(pool: PgPool) {
    let (o, to, _) = user(&pool, "Owner", Some("owner")).await;
    let (f, tf, _) = user(&pool, "Friend", Some("friend")).await;
    let (sel, tsel, _) = user(&pool, "Sel", Some("sel")).await;
    let (_, ts, _) = user(&pool, "Stranger", Some("stranger")).await;
    befriend(&pool, o, f).await;
    befriend(&pool, o, sel).await;
    for v in ["public", "link", "friends", "selected", "private"] { list(&pool, o, v, false).await; }
    let (w_sel, _) = list(&pool, o, "selected", false).await;
    sqlx::query("INSERT INTO wishlist_allowed_users (wishlist_id, user_id) VALUES ($1, $2)").bind(w_sel).bind(sel).execute(&pool).await.unwrap();
    // draft 不列
    sqlx::query("UPDATE wishlists SET status='draft' WHERE id=$1").bind(w_sel).execute(&pool).await.unwrap();
    let (w_sel2, _) = list(&pool, o, "selected", false).await;
    sqlx::query("INSERT INTO wishlist_allowed_users (wishlist_id, user_id) VALUES ($1, $2)").bind(w_sel2).bind(sel).execute(&pool).await.unwrap();
    let vis = |v: &Value| -> Vec<String> { let mut x: Vec<String> = v["wishlists"].as_array().unwrap().iter().map(|w| w["visibility"].as_str().unwrap().into()).collect(); x.sort(); x };
    let (s, v) = call(&pool, "GET", "/users/owner", None, None).await;
    assert_eq!((s, v["relation"].as_str(), vis(&v)), (StatusCode::OK, Some("anonymous"), vec!["public".to_string()]));
    assert_eq!(v["wishlists"][0]["item_count"], 1);
    assert_eq!(v["wishlists"][0]["completion_pct"], 0);
    let (_, v) = call(&pool, "GET", "/users/owner", Some(&ts), None).await;
    assert_eq!((v["relation"].as_str(), vis(&v)), (Some("none"), vec!["public".to_string()]));
    let (_, v) = call(&pool, "GET", "/users/OWNER", Some(&tf), None).await;
    assert_eq!((v["relation"].as_str(), vis(&v)), (Some("friend"), vec!["friends".to_string(), "public".to_string()]));
    let (_, v) = call(&pool, "GET", "/users/owner", Some(&tsel), None).await;
    assert_eq!(vis(&v), vec!["friends", "public", "selected"]);
    let (_, v) = call(&pool, "GET", "/users/owner", Some(&to), None).await;
    assert_eq!((v["relation"].as_str(), vis(&v)), (Some("self"), ["friends", "public", "selected", "selected"].map(String::from).to_vec()));
    assert_eq!(call(&pool, "GET", "/users/nobody", None, None).await.0, StatusCode::NOT_FOUND);
    // 申請中的關係
    call(&pool, "POST", "/friends/requests", Some(&ts), Some(json!({ "to": "owner" }))).await;
    assert_eq!(call(&pool, "GET", "/users/owner", Some(&ts), None).await.1["relation"], "outgoing");
    assert_eq!(call(&pool, "GET", "/users/stranger", Some(&to), None).await.1["relation"], "incoming");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn profile_donations_visibility(pool: PgPool) {
    let (d, td, _) = user(&pool, "Donor", Some("donor")).await;
    let (o, _, _) = user(&pool, "Owner", Some("owner")).await;
    let (f, tf, _) = user(&pool, "Friend", Some("friend")).await;
    let (_, ts, _) = user(&pool, "Stranger", Some("stranger")).await;
    befriend(&pool, d, f).await; // 捐助者的好友（非清單擁有者的好友）
    let (_, pub_item) = list(&pool, o, "public", false).await;
    let (_, link_item) = list(&pool, o, "link", false).await;
    let (_, priv_item) = list(&pool, o, "private", false).await;
    let (_, pw_item) = list(&pool, o, "public", false).await;
    sqlx::query("UPDATE wishlists SET visibility='password', access_password_hash='x' WHERE id=(SELECT wishlist_id FROM wishlist_items WHERE id=$1)").bind(pw_item).execute(&pool).await.unwrap();
    let (_, locked_item) = list(&pool, o, "public", true).await;
    let (_, fr_item) = list(&pool, o, "friends", false).await;
    let (_, hidden_claim_item) = list(&pool, o, "public", false).await;
    for it in [pub_item, link_item, priv_item, pw_item, locked_item, fr_item] { user_claim(&pool, it, d, "public").await; }
    user_claim(&pool, hidden_claim_item, d, "private").await;

    let titles = |v: &Value| -> Vec<(String, bool)> { let mut x: Vec<_> = v["donations"].as_array().unwrap().iter()
        .map(|c| (c["wishlist"]["title"].as_str().unwrap().to_string(), c["wishlist"]["slug"].is_null())).collect(); x.sort(); x };
    // 陌生人/匿名：public 清單與 link 清單（slug 為 null）；排除 private/password/驚喜鎖定/friends（非擁有者好友）/claim 私人
    let expect = vec![("清單-link".to_string(), true), ("清單-public".to_string(), false)];
    let (_, v) = call(&pool, "GET", "/users/donor", None, None).await;
    assert_eq!(titles(&v), expect, "{v}");
    assert_eq!(v["donations"][0]["wishlist"]["owner"]["handle"], "owner");
    let (_, v) = call(&pool, "GET", "/users/donor", Some(&ts), None).await;
    assert_eq!(titles(&v), expect);
    // 本人：多看到 claim visibility=private 的（仍受清單規則限制）
    let (_, v) = call(&pool, "GET", "/users/donor", Some(&td), None).await;
    assert_eq!(v["donations"].as_array().unwrap().len(), 3);
    // 捐助改 friends：陌生人看不到、捐助者好友看得到
    sqlx::query("UPDATE claims SET visibility='friends' WHERE item_id=$1").bind(pub_item).execute(&pool).await.unwrap();
    let (_, v) = call(&pool, "GET", "/users/donor", Some(&ts), None).await;
    assert_eq!(titles(&v), vec![("清單-link".to_string(), true)]);
    let (_, v) = call(&pool, "GET", "/users/donor", Some(&tf), None).await;
    assert_eq!(titles(&v).len(), 2);
    // 擁有者的好友可見 friends 清單的捐助
    let (_, tg, _) = user(&pool, "G", Some("gee")).await;
    let gid: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE handle='gee'").fetch_one(&pool).await.unwrap();
    befriend(&pool, gid, o).await;
    let (_, v) = call(&pool, "GET", "/users/donor", Some(&tg), None).await;
    assert!(titles(&v).contains(&("清單-friends".to_string(), false)));
    // 已取消的認領不列
    sqlx::query("UPDATE claims SET status='cancelled' WHERE item_id=$1").bind(link_item).execute(&pool).await.unwrap();
    let (_, v) = call(&pool, "GET", "/users/donor", Some(&tg), None).await;
    assert!(!titles(&v).iter().any(|t| t.0 == "清單-link"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn me_handle_and_default_visibility(pool: PgPool) {
    let (_, ta, _) = user(&pool, "A", None).await;
    let (_, tb, _) = user(&pool, "B", Some("taken")).await;
    let (_, v) = call(&pool, "GET", "/me", Some(&ta), None).await;
    assert!(v["handle"].is_null());
    assert_eq!(v["default_claim_visibility"], "private");
    let (s, v) = call(&pool, "PATCH", "/me", Some(&ta), Some(json!({ "handle": "Alice_01", "default_claim_visibility": "friends" }))).await;
    assert_eq!((s, v["handle"].as_str(), v["default_claim_visibility"].as_str()), (StatusCode::OK, Some("alice_01"), Some("friends")));
    assert_eq!(call(&pool, "PATCH", "/me", Some(&ta), Some(json!({ "handle": "taken" }))).await.1["code"], "HANDLE_TAKEN");
    assert_eq!(call(&pool, "PATCH", "/me", Some(&ta), Some(json!({ "handle": "taken" }))).await.0, StatusCode::CONFLICT);
    for bad in ["ab", "has space", "中文名字", &"a".repeat(31)] {
        assert_eq!(call(&pool, "PATCH", "/me", Some(&ta), Some(json!({ "handle": bad }))).await.0, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    assert_eq!(call(&pool, "PATCH", "/me", Some(&ta), Some(json!({ "default_claim_visibility": "x" }))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    // 只改 display_name 不動 handle；null 清除
    let (_, v) = call(&pool, "PATCH", "/me", Some(&ta), Some(json!({ "display_name": "新名" }))).await;
    assert_eq!(v["handle"], "alice_01");
    let (_, v) = call(&pool, "PATCH", "/me", Some(&ta), Some(json!({ "handle": null }))).await;
    assert!(v["handle"].is_null());
    let _ = tb;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_visibility_default_override_patch(pool: PgPool) {
    let (o, _, _) = user(&pool, "O", None).await;
    let (_, ta, _) = user(&pool, "A", None).await;
    call(&pool, "PATCH", "/me", Some(&ta), Some(json!({ "default_claim_visibility": "friends" }))).await;
    let mk = |_: ()| async { list(&pool, o, "public", false).await.1 };
    let (i1, i2, i3) = (mk(()).await, mk(()).await, mk(()).await);
    // 預設（trigger）
    let (s, v) = call(&pool, "POST", &format!("/items/{i1}/claims"), Some(&ta), Some(json!({ "qty": 1 }))).await;
    assert_eq!((s, v["claim"]["visibility"].as_str()), (StatusCode::CREATED, Some("friends")), "{v}");
    // 覆寫
    let (_, v) = call(&pool, "POST", &format!("/items/{i2}/claims"), Some(&ta), Some(json!({ "qty": 1, "visibility": "public" }))).await;
    assert_eq!(v["claim"]["visibility"], "public");
    let cid = v["claim"]["id"].as_str().unwrap().to_string();
    assert_eq!(call(&pool, "POST", &format!("/items/{i3}/claims"), Some(&ta), Some(json!({ "qty": 1, "visibility": "bogus" }))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    // PATCH
    let (s, v) = call(&pool, "PATCH", &format!("/claims/{cid}"), Some(&ta), Some(json!({ "visibility": "private" }))).await;
    assert_eq!((s, v["claim"]["visibility"].as_str()), (StatusCode::OK, Some("private")), "{v}");
    assert_eq!(call(&pool, "PATCH", &format!("/claims/{cid}"), Some(&ta), Some(json!({ "visibility": "x" }))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    // 訪客：create 帶 visibility 被忽略（null）；PATCH visibility → 422
    let (s, v) = call(&pool, "POST", &format!("/items/{i3}/claims"), None, Some(json!({ "qty": 1, "display_name": "客", "visibility": "public" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert!(v["claim"]["visibility"].is_null());
    let gtok = v["guest_token"].as_str().unwrap().to_string();
    let gcid = v["claim"]["id"].as_str().unwrap().to_string();
    let req = Request::builder().method("PATCH").uri(format!("/api/v1/claims/{gcid}")).header("x-guest-token", gtok)
        .header("content-type", "application/json").body(Body::from(json!({ "visibility": "public" }).to_string())).unwrap();
    assert_eq!(app(AppState { pool: pool.clone() }).oneshot(req).await.unwrap().status(), StatusCode::UNPROCESSABLE_ENTITY);
    // 別人不能改
    let (_, tb, _) = user(&pool, "B", None).await;
    assert_eq!(call(&pool, "PATCH", &format!("/claims/{cid}"), Some(&tb), Some(json!({ "visibility": "public" }))).await.0, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_page_masks_non_public_user_claims(pool: PgPool) {
    let (o, _, _) = user(&pool, "O", None).await;
    let (d, _, _) = user(&pool, "D", None).await;
    let (e, _, _) = user(&pool, "E", None).await;
    let (wid, it) = list(&pool, o, "link", false).await;
    sqlx::query("UPDATE wishlists SET show_claimer_names = true WHERE id = $1").bind(wid).execute(&pool).await.unwrap();
    sqlx::query("UPDATE claims SET claimer_name = '小明' WHERE id = $1").bind(user_claim(&pool, it, d, "public").await).execute(&pool).await.unwrap();
    user_claim(&pool, it, e, "friends").await;
    let slug: String = sqlx::query_scalar("SELECT slug::text FROM wishlists WHERE id = $1").bind(wid).fetch_one(&pool).await.unwrap();
    let (s, v) = call(&pool, "GET", &format!("/public/wishlists/{slug}"), None, None).await;
    assert_eq!(s, StatusCode::OK);
    let names: Vec<&str> = v["items"][0]["claimers"].as_array().unwrap().iter().map(|c| c["display_name"].as_str().unwrap()).collect();
    assert_eq!(names, ["小明", "匿名"]);
}
