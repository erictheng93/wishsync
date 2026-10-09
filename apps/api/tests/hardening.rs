//! F-01 / F-03 / F-09 / F-11 / F-16 / F-17：冪等重播重簽 token、集中式文字驗證、Email、日期範圍、強制刪除善後
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
fn gt(t: &str) -> Vec<(&'static str, String)> { vec![("x-guest-token", t.to_string())] }
fn idem(k: Uuid) -> Vec<(&'static str, String)> { vec![("idempotency-key", k.to_string())] }

async fn user(pool: &PgPool) -> (Uuid, String) {
    let id: Uuid = sqlx::query_scalar("INSERT INTO users (display_name, email) VALUES ('u', $1) RETURNING id")
        .bind(format!("{}@example.com", Uuid::new_v4())).fetch_one(pool).await.unwrap();
    let tok = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(id).bind(hash_token(&tok)).execute(pool).await.unwrap();
    (id, tok)
}

/// active 清單 + qty_needed=5 的品項
async fn fixture(pool: &PgPool, owner: Uuid) -> (Uuid, String, Uuid) {
    let slug: String = Uuid::new_v4().simple().to_string()[..10].to_string();
    let wid: Uuid = sqlx::query_scalar("INSERT INTO wishlists (owner_id, type, status, slug, title) VALUES ($1, 'registry', 'active', $2, '清單') RETURNING id")
        .bind(owner).bind(&slug).fetch_one(pool).await.unwrap();
    let it: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, '奶瓶', 5) RETURNING id").bind(wid).fetch_one(pool).await.unwrap();
    (wid, slug, it)
}

async fn claim(pool: &PgPool, it: Uuid, key: Uuid, hdr: Vec<(&'static str, String)>, body: Value) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut h = idem(key);
    h.extend(hdr);
    call(pool, "POST", &format!("/items/{it}/claims"), &h, Some(body)).await
}

fn field(b: &Value) -> &str { b["errors"][0]["pointer"].as_str().unwrap_or("") }

// ---------- F-01 ----------
#[sqlx::test(migrations = "../../db/migrations")]
async fn lost_first_response_replay_reissues_usable_token(pool: PgPool) {
    let (owner, _) = user(&pool).await;
    let (_, _, it) = fixture(&pool, owner).await;
    let key = Uuid::new_v4();
    let body = json!({"qty": 2, "display_name": "小明"});
    // 首次回應「遺失」：伺服器已處理，客戶端沒拿到 token（這裡直接丟掉 b1 的 token 以外的資訊，只留來驗證舊 token 失效）
    let (s1, _, b1) = claim(&pool, it, key, vec![], body.clone()).await;
    assert_eq!(s1, StatusCode::CREATED);
    let old = b1["guest_token"].as_str().unwrap().to_string();
    // 以同 key 重送 → 同一認領 + 新 token + Set-Cookie
    let (s2, h2, b2) = claim(&pool, it, key, vec![], body.clone()).await;
    assert_eq!(s2, StatusCode::CREATED);
    assert_eq!(h2["idempotency-replayed"], "true");
    let new = b2["guest_token"].as_str().unwrap().to_string();
    assert_ne!(new, old);
    assert!(h2["set-cookie"].to_str().unwrap().starts_with(&format!("ws_guest={new};")));
    assert_eq!(b1["claim"]["id"], b2["claim"]["id"]);
    // 新 token 看得到該認領；舊 token 失效
    let (s, _, me) = call(&pool, "GET", "/guest/me", &gt(&new), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["claims"][0]["claim"]["id"], b1["claim"]["id"]);
    assert_eq!(call(&pool, "GET", "/guest/me", &gt(&old), None).await.0, StatusCode::UNAUTHORIZED);
    // 仍沒有重複 guest / claim，且 idempotency_keys 不存明文 token
    let (g, c): (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM guests), (SELECT count(*) FROM claims)").fetch_one(&pool).await.unwrap();
    assert_eq!((g, c), (1, 1));
    let leaked: i64 = sqlx::query_scalar("SELECT count(*) FROM idempotency_keys WHERE response_body::text LIKE $1 OR response_body::text LIKE $2")
        .bind(format!("%{old}%")).bind(format!("%{new}%")).fetch_one(&pool).await.unwrap();
    assert_eq!(leaked, 0);
}

// ---------- F-03 / F-16 ----------
#[sqlx::test(migrations = "../../db/migrations")]
async fn text_validation_wishlists_and_items(pool: PgPool) {
    let (_, tok) = user(&pool).await;
    let h = ck(&tok);
    for (t, why) in [("a\u{0}b", "nul"), ("a\u{7}b", "bel"), ("a\u{1b}b", "esc"), ("a\u{202e}b", "rlo"), ("a\u{2066}b", "isolate"),
                     ("\u{200b}\u{200d}\u{feff}", "zero-width only"), ("a\nb", "newline in single line")] {
        let (s, _, b) = call(&pool, "POST", "/wishlists", &h, Some(json!({"type": "registry", "title": t}))).await;
        assert_eq!((s, field(&b)), (StatusCode::UNPROCESSABLE_ENTITY, "/title"), "{why}");
        assert_eq!(b["code"], "VALIDATION_FAILED");
    }
    // description 保留 \n \t，但拒絕 NUL / 控制字元
    let (s, _, w) = call(&pool, "POST", "/wishlists", &h, Some(json!({"type": "registry", "title": " 好清單 ", "description": "第一行\n\t第二行"}))).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!((w["title"].as_str().unwrap(), w["description"].as_str().unwrap()), ("好清單", "第一行\n\t第二行"));
    let (s, _, b) = call(&pool, "POST", "/wishlists", &h, Some(json!({"type": "registry", "title": "ok", "description": "x\u{0}y"}))).await;
    assert_eq!((s, field(&b)), (StatusCode::UNPROCESSABLE_ENTITY, "/description"));
    let (s, _, b) = call(&pool, "PATCH", &format!("/wishlists/{}", w["id"].as_str().unwrap()), &h, Some(json!({"title": "a\u{0}"}))).await;
    assert_eq!((s, field(&b)), (StatusCode::UNPROCESSABLE_ENTITY, "/title"));

    let wid = w["id"].as_str().unwrap();
    for k in ["title", "description", "brand", "spec", "product_url"] {
        let (s, _, b) = call(&pool, "POST", &format!("/wishlists/{wid}/items"), &h, Some(json!({"title": "t", k: "a\u{0}b"}))).await;
        assert_eq!((s, field(&b)), (StatusCode::UNPROCESSABLE_ENTITY, format!("/{k}").as_str()), "{k}");
    }
    let (s, _, b) = call(&pool, "POST", &format!("/wishlists/{wid}/items"), &h, Some(json!({"title": "\u{200b}"}))).await;
    assert_eq!((s, field(&b)), (StatusCode::UNPROCESSABLE_ENTITY, "/title"));
    for u in ["javascript:alert(1)", "data:text/html,x", "ftp://x.com/a"] {
        let (s, _, b) = call(&pool, "POST", &format!("/wishlists/{wid}/items"), &h, Some(json!({"title": "t", "product_url": u}))).await;
        assert_eq!((s, field(&b)), (StatusCode::UNPROCESSABLE_ENTITY, "/product_url"), "{u}");
    }
    let (s, _, _) = call(&pool, "POST", &format!("/wishlists/{wid}/items"), &h, Some(json!({"title": "t", "product_url": "https://example.com/a"}))).await;
    assert_eq!(s, StatusCode::CREATED);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn text_validation_claims_reports_slug(pool: PgPool) {
    let (owner, _) = user(&pool).await;
    let (_, slug, it) = fixture(&pool, owner).await;
    for (k, v) in [("display_name", "a\u{0}b"), ("display_name", "\u{200b}"), ("display_name", "a\u{202e}b"), ("note", "a\u{0}b"), ("contact", "a\u{7}b"), ("contact", "a\nb")] {
        let mut body = json!({"qty": 1, "display_name": "小明"});
        body[k] = json!(v);
        let (s, _, b) = claim(&pool, it, Uuid::new_v4(), vec![], body).await;
        assert_eq!((s, field(&b)), (StatusCode::UNPROCESSABLE_ENTITY, format!("/{k}").as_str()), "{k}");
    }
    // 備註可含換行
    let (s, _, _) = claim(&pool, it, Uuid::new_v4(), vec![], json!({"qty": 1, "display_name": "小明", "note": "a\nb"})).await;
    assert_eq!(s, StatusCode::CREATED);
    // 失敗請求不留下 guest
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM guests").fetch_one(&pool).await.unwrap(), 1);
    // 檢舉 detail
    let (s, _, b) = call(&pool, "POST", &format!("/public/wishlists/{slug}/reports"), &[], Some(json!({"reason": "scam", "detail": "x\u{0}y"}))).await;
    assert_eq!((s, field(&b)), (StatusCode::UNPROCESSABLE_ENTITY, "/detail"));
    // slug 含 NUL → 404 而非 500
    assert_eq!(call(&pool, "GET", "/public/wishlists/abc%00defghi", &[], None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "POST", "/public/wishlists/abc%00defghi/reports", &[], Some(json!({"reason": "scam"}))).await.0, StatusCode::NOT_FOUND);
}

// ---------- F-11 ----------
#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_email_must_be_deliverable(pool: PgPool) {
    let (owner, _) = user(&pool).await;
    let (_, _, it) = fixture(&pool, owner).await;
    for e in ["a b@c.com", "<script>@x.com", "a@b.com\r\nBcc: x@y.com", "no-at", "a@", "@b.com", "a@b@c.com"] {
        let (s, _, b) = claim(&pool, it, Uuid::new_v4(), vec![], json!({"qty": 1, "display_name": "小明", "email": e})).await;
        assert_eq!((s, field(&b)), (StatusCode::UNPROCESSABLE_ENTITY, "/email"), "{e:?}");
    }
    let (s, _, _) = claim(&pool, it, Uuid::new_v4(), vec![], json!({"qty": 1, "display_name": "小明", "email": " Ok@Example.com "})).await;
    assert_eq!(s, StatusCode::CREATED);
    let e: String = sqlx::query_scalar("SELECT email FROM guests").fetch_one(&pool).await.unwrap();
    assert_eq!(e, "ok@example.com");
}

// ---------- F-17 ----------
#[sqlx::test(migrations = "../../db/migrations")]
async fn event_date_range(pool: PgPool) {
    let (_, tok) = user(&pool).await;
    let h = ck(&tok);
    for d in ["0000-01-01", "-0001-01-01", "1999-12-31", "2101-01-01", "9999-12-31"] {
        for surprise in [false, true] {
            let (s, _, b) = call(&pool, "POST", "/wishlists", &h, Some(json!({"type": "registry", "title": "t", "event_date": d, "surprise_mode": surprise}))).await;
            assert_eq!((s, field(&b)), (StatusCode::UNPROCESSABLE_ENTITY, "/event_date"), "{d} {surprise}");
        }
    }
    for d in ["2000-01-01", "2100-12-31"] {
        let (s, _, _) = call(&pool, "POST", "/wishlists", &h, Some(json!({"type": "registry", "title": "t", "event_date": d}))).await;
        assert_eq!(s, StatusCode::CREATED, "{d}");
    }
}

// ---------- F-09 ----------
#[sqlx::test(migrations = "../../db/migrations")]
async fn force_delete_cleans_up_claims(pool: PgPool) {
    let (owner, tok) = user(&pool).await;
    let (wid, _, it) = fixture(&pool, owner).await;
    let (_, _, a) = claim(&pool, it, Uuid::new_v4(), vec![], json!({"qty": 2, "display_name": "甲", "email": "a@example.com"})).await;
    let (_, _, _b) = claim(&pool, it, Uuid::new_v4(), vec![], json!({"qty": 1, "display_name": "乙"})).await;
    let (cu, ctok) = user(&pool).await; // 登入使用者認領（有 email）
    let (s, _, _) = claim(&pool, it, Uuid::new_v4(), ck(&ctok), json!({"qty": 1})).await;
    assert_eq!(s, StatusCode::CREATED);
    assert!(a["guest_token"].is_string());
    let h = ck(&tok);
    // 無 force 仍 409
    assert_eq!(call(&pool, "DELETE", &format!("/items/{it}"), &h, None).await.0, StatusCode::CONFLICT);
    assert_eq!(call(&pool, "DELETE", &format!("/items/{it}?force=true"), &h, None).await.0, StatusCode::NO_CONTENT);

    let (q, del): (i32, bool) = sqlx::query_as("SELECT qty_claimed, deleted_at IS NOT NULL FROM wishlist_items WHERE id=$1").bind(it).fetch_one(&pool).await.unwrap();
    assert_eq!((q, del), (0, true));
    let cancelled: i64 = sqlx::query_scalar("SELECT count(*) FROM claims WHERE item_id=$1 AND status='cancelled'").bind(it).fetch_one(&pool).await.unwrap();
    assert_eq!(cancelled, 3);
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='claim.cancel_by_item_delete' AND actor_id=$1").bind(owner).fetch_one(&pool).await.unwrap();
    assert_eq!(audits, 3);
    // 通知：有 email 的訪客甲 + 使用者各一則；無 email 的乙沒有
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='claim.item_removed' AND payload->>'wishlist_id' = $1").bind(wid.to_string()).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 2);
    let nu: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='claim.item_removed' AND user_id=$1").bind(cu).fetch_one(&pool).await.unwrap();
    assert_eq!(nu, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn force_delete_still_forbidden_while_surprise_locked(pool: PgPool) {
    let (owner, tok) = user(&pool).await;
    let (wid, _, it) = fixture(&pool, owner).await;
    sqlx::query("UPDATE wishlists SET surprise_mode = true, event_date = current_date + 30 WHERE id = $1").bind(wid).execute(&pool).await.unwrap();
    claim(&pool, it, Uuid::new_v4(), vec![], json!({"qty": 1, "display_name": "甲"})).await;
    assert_eq!(call(&pool, "DELETE", &format!("/items/{it}?force=true"), &ck(&tok), None).await.0, StatusCode::FORBIDDEN);
    let q: i32 = sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id=$1").bind(it).fetch_one(&pool).await.unwrap();
    assert_eq!(q, 1);
}
