//! P2-A 捐贈者端：認捐 / 撤回 / 轉投 / 錢包 / tick_funding。fixture 全以 SQL 建立，不依賴品項 API。
use axum::{body::Body, http::{Request, StatusCode}, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wishsync_api::{contributions, session::hash_token, wallet, AppState};

type Resp = (StatusCode, axum::http::HeaderMap, Value);

fn test_app(pool: &PgPool) -> Router {
    Router::new().nest("/api/v1", contributions::routes().merge(wallet::routes())).with_state(AppState { pool: pool.clone() })
}

async fn call(app: &Router, method: &str, uri: &str, cookie: Option<&str>, key: Option<Uuid>, body: Option<Value>) -> Resp {
    let mut r = Request::builder().method(method).uri(format!("/api/v1{uri}"));
    if let Some(c) = cookie { r = r.header("cookie", format!("ws_session={c}")); }
    if let Some(k) = key { r = r.header("idempotency-key", k.to_string()); }
    let req = match body { Some(b) => r.header("content-type", "application/json").body(Body::from(b.to_string())), None => r.body(Body::empty()) }.unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let (st, h) = (res.status(), res.headers().clone());
    let b = res.into_body().collect().await.unwrap().to_bytes();
    (st, h, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

async fn pledge(app: &Router, item: Uuid, tok: &str, points: i64) -> Resp {
    call(app, "POST", &format!("/items/{item}/contributions"), Some(tok), Some(Uuid::new_v4()), Some(json!({ "points": points }))).await
}

/// 使用者 + session；回傳 (id, token)
async fn user(pool: &PgPool, name: &str) -> (Uuid, String) {
    let id: Uuid = sqlx::query_scalar("INSERT INTO users (display_name, email) VALUES ($1, $2) RETURNING id")
        .bind(name).bind(format!("{}@example.com", Uuid::new_v4())).fetch_one(pool).await.unwrap();
    let tok = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(id).bind(hash_token(&tok)).execute(pool).await.unwrap();
    (id, tok)
}

/// 發點（ledger grant），保持 balance = Σ delta
async fn grant(pool: &PgPool, uid: Uuid, n: i64) {
    let wid: Uuid = sqlx::query_scalar("INSERT INTO point_wallets (user_id) VALUES ($1) ON CONFLICT (user_id) DO UPDATE SET user_id = EXCLUDED.user_id RETURNING id")
        .bind(uid).fetch_one(pool).await.unwrap();
    let after: i64 = sqlx::query_scalar("UPDATE point_wallets SET balance = balance + $2 WHERE id = $1 RETURNING balance").bind(wid).bind(n).fetch_one(pool).await.unwrap();
    sqlx::query("INSERT INTO point_ledger (wallet_id, delta, balance_after, entry_type, ref_type, note) VALUES ($1, $2, $3, 'grant', 'manual', 'test')")
        .bind(wid).bind(n).bind(after).execute(pool).await.unwrap();
}

async fn balance(pool: &PgPool, uid: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COALESCE((SELECT balance FROM point_wallets WHERE user_id = $1), 0)").bind(uid).fetch_one(pool).await.unwrap()
}

async fn wishlist(pool: &PgPool, owner: Uuid) -> Uuid {
    let slug: String = Uuid::new_v4().simple().to_string()[..10].to_string();
    let w: Uuid = sqlx::query_scalar("INSERT INTO wishlists (owner_id, type, status, slug, title) VALUES ($1, 'registry', 'active', $2, '寶寶清單') RETURNING id")
        .bind(owner).bind(slug).fetch_one(pool).await.unwrap();
    let sealed = |s: &str| wishsync_api::sealed::seal(s.as_bytes());
    sqlx::query("INSERT INTO shipping_addresses (wishlist_id, recipient_name_enc, phone_enc, address_enc) VALUES ($1, $2, $3, $4)")
        .bind(w).bind(sealed("王小米")).bind(sealed("0912345678")).bind(sealed("台北市大安區某路 1 號")).execute(pool).await.unwrap();
    w
}

async fn cf_item(pool: &PgPool, wl: Uuid, target: i64) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO wishlist_items (wishlist_id, title, qty_needed, funding_mode, target_points, funding_status, funding_deadline, fulfillment_type)
         VALUES ($1, '嬰兒推車', 1, 'crowdfund', $2, 'open', now() + interval '7 days', 'concierge') RETURNING id")
        .bind(wl).bind(target).fetch_one(pool).await.unwrap()
}

async fn item_row(pool: &PgPool, id: Uuid) -> (i64, String) {
    sqlx::query_as("SELECT pledged_points, funding_status::text FROM wishlist_items WHERE id = $1").bind(id).fetch_one(pool).await.unwrap()
}

async fn assert_reconciled(pool: &PgPool) {
    let bad: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM point_wallets w WHERE w.balance <> COALESCE((SELECT sum(delta) FROM point_ledger l WHERE l.wallet_id = w.id), 0)")
        .fetch_one(pool).await.unwrap();
    assert_eq!(bad, 0, "錢包 balance 與 ledger 加總不一致");
    // 品項快取：pledged_points = Σ pledged/captured
    let drift: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM wishlist_items i WHERE i.funding_mode = 'crowdfund'
           AND i.pledged_points <> COALESCE((SELECT sum(points) FROM contributions c WHERE c.item_id = i.id AND c.status IN ('pledged','captured')), 0)")
        .fetch_one(pool).await.unwrap();
    assert_eq!(drift, 0, "item.pledged_points 漂移");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn pledge_happy_path(pool: PgPool) {
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let (donor, tok) = user(&pool, "阿明").await;
    grant(&pool, donor, 1000).await;
    let item = cf_item(&pool, wishlist(&pool, owner).await, 500).await;

    let (st, h, b) = call(&app, "POST", &format!("/items/{item}/contributions"), Some(&tok), Some(Uuid::new_v4()),
        Some(json!({ "points": 200, "message": "恭喜", "is_anonymous": true }))).await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    assert!(h["cache-control"].to_str().unwrap().contains("no-store"));
    assert_eq!(b["funded"], false);
    assert_eq!(b["wallet"]["balance"], 800);
    assert_eq!(b["item"]["pledged_points"], 200);
    assert_eq!(b["item"]["remaining_points"], 300);
    assert_eq!(b["item"]["progress_percent"], 40);
    assert_eq!(b["contribution"]["status"], "pledged");
    assert_eq!(b["contribution"]["is_anonymous"], true);
    assert_eq!(balance(&pool, donor).await, 800);
    let (delta, kind, after): (i64, String, i64) = sqlx::query_as(
        "SELECT delta, entry_type::text, balance_after FROM point_ledger WHERE ref_type = 'contribution' ORDER BY seq DESC LIMIT 1").fetch_one(&pool).await.unwrap();
    assert_eq!((delta, kind.as_str(), after), (-200, "pledge", 800));
    let name: String = sqlx::query_scalar("SELECT donor_name FROM contributions").fetch_one(&pool).await.unwrap();
    assert_eq!(name, "阿明");
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn exact_target_funds_and_captures(pool: PgPool) {
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let (a, ta) = user(&pool, "甲").await;
    let (b, tb) = user(&pool, "乙").await;
    grant(&pool, a, 1000).await;
    grant(&pool, b, 1000).await;
    let item = cf_item(&pool, wishlist(&pool, owner).await, 500).await;

    assert_eq!(pledge(&app, item, &ta, 300).await.0, StatusCode::CREATED);
    let (st, _, r) = pledge(&app, item, &tb, 200).await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    assert_eq!(r["funded"], true);
    assert_eq!(r["item"]["funding_status"], "funded");
    assert_eq!(r["item"]["progress_percent"], 100);
    assert_eq!(r["contribution"]["status"], "captured");
    assert_eq!(item_row(&pool, item).await, (500, "funded".into()));
    let captured: i64 = sqlx::query_scalar("SELECT count(*) FROM contributions WHERE status = 'captured' AND captured_at IS NOT NULL").fetch_one(&pool).await.unwrap();
    assert_eq!(captured, 2);
    let (amount, ost): (i64, String) = sqlx::query_as("SELECT amount, status::text FROM purchase_orders WHERE item_id = $1").bind(item).fetch_one(&pool).await.unwrap();
    assert_eq!((amount, ost.as_str()), (500, "pending"));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind = 'crowdfund.funded'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 2);
    // 達標後再認捐 → ITEM_FUNDED
    let (st, _, e) = pledge(&app, item, &ta, 1).await;
    assert_eq!((st, e["code"].as_str()), (StatusCode::CONFLICT, Some("ITEM_FUNDED")));
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn hard_cap_reports_remaining(pool: PgPool) {
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let (a, ta) = user(&pool, "甲").await;
    grant(&pool, a, 1000).await;
    let item = cf_item(&pool, wishlist(&pool, owner).await, 500).await;
    assert_eq!(pledge(&app, item, &ta, 450).await.0, StatusCode::CREATED);
    let (st, _, e) = pledge(&app, item, &ta, 100).await;
    assert_eq!((st, e["code"].as_str(), e["remaining_points"].as_i64()), (StatusCode::CONFLICT, Some("CROWDFUND_TARGET_EXCEEDED"), Some(50)));
    assert_eq!(balance(&pool, a).await, 550);
    assert_eq!(item_row(&pool, item).await.0, 450);
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_last_slot_only_one_wins(pool: PgPool) {
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let item = cf_item(&pool, wishlist(&pool, owner).await, 100).await;
    let mut users = vec![];
    for n in 0..6 {
        let (id, tok) = user(&pool, &format!("u{n}")).await;
        grant(&pool, id, 100).await;
        users.push((id, tok));
    }
    let tasks: Vec<_> = users.iter().map(|(_, tok)| {
        let (app, tok) = (app.clone(), tok.clone());
        tokio::spawn(async move { pledge(&app, item, &tok, 60).await })
    }).collect();
    let mut ok = 0;
    for t in tasks {
        let (st, _, b) = t.await.unwrap();
        match st {
            StatusCode::CREATED => ok += 1,
            StatusCode::CONFLICT => assert!(["CROWDFUND_TARGET_EXCEEDED", "ITEM_FUNDED"].contains(&b["code"].as_str().unwrap()), "{b}"),
            other => panic!("unexpected {other} {b}"),
        }
    }
    assert_eq!(ok, 1);
    assert_eq!(item_row(&pool, item).await, (60, "open".into()));
    // 失敗者點數未被扣
    let mut spent = 0;
    for (id, _) in &users { let b = balance(&pool, *id).await; assert!(b == 100 || b == 40); if b == 40 { spent += 1; } }
    assert_eq!(spent, 1);
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_exact_fill_never_oversells(pool: PgPool) {
    // 5 人各 30 點搶 100 點目標 → 最多 3 人成功（90），不會超額
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let item = cf_item(&pool, wishlist(&pool, owner).await, 100).await;
    let mut toks = vec![];
    for n in 0..5 { let (id, t) = user(&pool, &format!("u{n}")).await; grant(&pool, id, 100).await; toks.push(t); }
    let tasks: Vec<_> = toks.iter().map(|t| { let (app, t) = (app.clone(), t.clone()); tokio::spawn(async move { pledge(&app, item, &t, 30).await.0 }) }).collect();
    let mut ok = 0;
    for t in tasks { if t.await.unwrap() == StatusCode::CREATED { ok += 1; } }
    assert_eq!(ok, 3);
    assert_eq!(item_row(&pool, item).await.0, 90);
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn pledge_rejections(pool: PgPool) {
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let (a, ta) = user(&pool, "甲").await;
    grant(&pool, a, 100).await;
    let wl = wishlist(&pool, owner).await;
    let item = cf_item(&pool, wl, 500).await;
    let code = |r: &Resp| (r.0, r.2["code"].as_str().unwrap_or("").to_string());

    // 餘額不足：回 balance，且不留下認捐
    let r = pledge(&app, item, &ta, 101).await;
    assert_eq!(code(&r), (StatusCode::CONFLICT, "INSUFFICIENT_POINTS".into()));
    assert_eq!(r.2["balance"], 100);
    assert_eq!(item_row(&pool, item).await.0, 0);
    // 欄位驗證
    for p in [0, -5, 10_000_001] { assert_eq!(pledge(&app, item, &ta, p).await.0, StatusCode::UNPROCESSABLE_ENTITY); }
    let r = call(&app, "POST", &format!("/items/{item}/contributions"), Some(&ta), Some(Uuid::new_v4()), Some(json!({ "points": 1, "message": "x".repeat(201) }))).await;
    assert_eq!(r.0, StatusCode::UNPROCESSABLE_ENTITY);
    // 未登入 / 缺 key
    let r = call(&app, "POST", &format!("/items/{item}/contributions"), None, Some(Uuid::new_v4()), Some(json!({ "points": 1 }))).await;
    assert_eq!(code(&r), (StatusCode::UNAUTHORIZED, "UNAUTHORIZED".into()));
    let r = call(&app, "POST", &format!("/items/{item}/contributions"), Some(&ta), None, Some(json!({ "points": 1 }))).await;
    assert_eq!(code(&r), (StatusCode::BAD_REQUEST, "IDEMPOTENCY_KEY_REQUIRED".into()));
    // 不存在的品項
    assert_eq!(pledge(&app, Uuid::new_v4(), &ta, 1).await.0, StatusCode::NOT_FOUND);
    // 數量型品項
    let q: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, '奶瓶', 3) RETURNING id").bind(wl).fetch_one(&pool).await.unwrap();
    assert_eq!(code(&pledge(&app, q, &ta, 1).await), (StatusCode::CONFLICT, "FUNDING_MODE_MISMATCH".into()));
    // 已過截止但 tick 尚未跑
    sqlx::query("UPDATE wishlist_items SET funding_deadline = now() - interval '1 minute' WHERE id = $1").bind(item).execute(&pool).await.unwrap();
    assert_eq!(code(&pledge(&app, item, &ta, 1).await), (StatusCode::CONFLICT, "FUNDING_EXPIRED".into()));
    // 已 expired
    sqlx::query("UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() WHERE id = $1").bind(item).execute(&pool).await.unwrap();
    assert_eq!(code(&pledge(&app, item, &ta, 1).await), (StatusCode::CONFLICT, "FUNDING_EXPIRED".into()));
    // 凍結
    let open = cf_item(&pool, wl, 500).await;
    sqlx::query("UPDATE point_wallets SET status = 'frozen' WHERE user_id = $1").bind(a).execute(&pool).await.unwrap();
    assert_eq!(code(&pledge(&app, open, &ta, 1).await), (StatusCode::FORBIDDEN, "WALLET_FROZEN".into()));
    assert_eq!(balance(&pool, a).await, 100);
    // 清單已關閉
    sqlx::query("UPDATE point_wallets SET status = 'active'").execute(&pool).await.unwrap();
    sqlx::query("UPDATE wishlists SET status = 'closed' WHERE id = $1").bind(wl).execute(&pool).await.unwrap();
    assert_eq!(code(&pledge(&app, open, &ta, 1).await), (StatusCode::CONFLICT, "WISHLIST_CLOSED".into()));
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn idempotent_replay_does_not_double_charge(pool: PgPool) {
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let (a, ta) = user(&pool, "甲").await;
    grant(&pool, a, 1000).await;
    let item = cf_item(&pool, wishlist(&pool, owner).await, 500).await;
    let key = Uuid::new_v4();
    let uri = format!("/items/{item}/contributions");
    let (s1, h1, b1) = call(&app, "POST", &uri, Some(&ta), Some(key), Some(json!({ "points": 100 }))).await;
    let (s2, h2, b2) = call(&app, "POST", &uri, Some(&ta), Some(key), Some(json!({ "points": 100 }))).await;
    assert_eq!((s1, s2), (StatusCode::CREATED, StatusCode::CREATED));
    assert!(h1.get("idempotency-replayed").is_none());
    assert_eq!(h2["idempotency-replayed"], "true");
    assert_eq!(b1["contribution"]["id"], b2["contribution"]["id"]);
    assert_eq!(balance(&pool, a).await, 900);
    // 同 key 不同內容 → 衝突
    let (s3, _, b3) = call(&app, "POST", &uri, Some(&ta), Some(key), Some(json!({ "points": 50 }))).await;
    assert_eq!((s3, b3["code"].as_str()), (StatusCode::CONFLICT, Some("IDEMPOTENCY_CONFLICT")));
    // 失敗不占 key：同 key 於餘額補足後可重試
    let k2 = Uuid::new_v4();
    let (s, ..) = call(&app, "POST", &uri, Some(&ta), Some(k2), Some(json!({ "points": 5000 }))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM idempotency_keys WHERE key = $1").bind(k2).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0);
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn withdraw_refunds_and_is_idempotent(pool: PgPool) {
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let (a, ta) = user(&pool, "甲").await;
    let (_, tb) = user(&pool, "乙").await;
    grant(&pool, a, 1000).await;
    let item = cf_item(&pool, wishlist(&pool, owner).await, 500).await;
    let (_, _, r) = pledge(&app, item, &ta, 300).await;
    let cid = r["contribution"]["id"].as_str().unwrap().to_string();

    // 他人 403；不存在 404；未登入 401
    assert_eq!(call(&app, "DELETE", &format!("/contributions/{cid}"), Some(&tb), None, None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(call(&app, "DELETE", &format!("/contributions/{}", Uuid::new_v4()), Some(&ta), None, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&app, "DELETE", &format!("/contributions/{cid}"), None, None, None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(balance(&pool, a).await, 700);

    let (st, _, b) = call(&app, "DELETE", &format!("/contributions/{cid}"), Some(&ta), None, None).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["contribution"]["status"], "released");
    assert_eq!(b["contribution"]["refunded_points"], 300);
    assert_eq!(b["wallet"]["balance"], 1000);
    assert_eq!(b["item"]["pledged_points"], 0);
    // 重複撤回：冪等，不重複退點
    let (st, _, b) = call(&app, "DELETE", &format!("/contributions/{cid}"), Some(&ta), None, None).await;
    assert_eq!((st, b["contribution"]["status"].as_str()), (StatusCode::OK, Some("released")));
    assert_eq!(balance(&pool, a).await, 1000);
    let rel: i64 = sqlx::query_scalar("SELECT count(*) FROM point_ledger WHERE entry_type = 'release'").fetch_one(&pool).await.unwrap();
    assert_eq!(rel, 1);
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn withdraw_locked_when_captured_or_window_over(pool: PgPool) {
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let (a, ta) = user(&pool, "甲").await;
    grant(&pool, a, 1000).await;
    let wl = wishlist(&pool, owner).await;
    let item = cf_item(&pool, wl, 100).await;
    let (_, _, r) = pledge(&app, item, &ta, 100).await;
    assert_eq!(r["funded"], true);
    let cid = r["contribution"]["id"].as_str().unwrap();
    let (st, _, e) = call(&app, "DELETE", &format!("/contributions/{cid}"), Some(&ta), None, None).await;
    assert_eq!((st, e["code"].as_str()), (StatusCode::CONFLICT, Some("CONTRIBUTION_LOCKED")));
    assert_eq!(balance(&pool, a).await, 900);

    // expired 且超過 7 天選擇期
    let item2 = cf_item(&pool, wl, 100).await;
    let (_, _, r) = pledge(&app, item2, &ta, 10).await;
    let cid2 = r["contribution"]["id"].as_str().unwrap();
    sqlx::query("UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() - interval '8 days' WHERE id = $1").bind(item2).execute(&pool).await.unwrap();
    let (st, _, e) = call(&app, "DELETE", &format!("/contributions/{cid2}"), Some(&ta), None, None).await;
    assert_eq!((st, e["code"].as_str()), (StatusCode::CONFLICT, Some("CONTRIBUTION_LOCKED")));
    // expired 但仍在選擇期 → 可撤回
    sqlx::query("UPDATE wishlist_items SET expired_at = now() - interval '1 day' WHERE id = $1").bind(item2).execute(&pool).await.unwrap();
    assert_eq!(call(&app, "DELETE", &format!("/contributions/{cid2}"), Some(&ta), None, None).await.0, StatusCode::OK);
    assert_eq!(balance(&pool, a).await, 900);
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reallocate_within_choice_window(pool: PgPool) {
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let (a, ta) = user(&pool, "甲").await;
    let (_, tb) = user(&pool, "乙").await;
    grant(&pool, a, 1000).await;
    let wl = wishlist(&pool, owner).await;
    let src = cf_item(&pool, wl, 500).await;
    let dst = cf_item(&pool, wl, 200).await;
    let other_wl = wishlist(&pool, owner).await;
    let foreign = cf_item(&pool, other_wl, 200).await;
    let (_, _, r) = pledge(&app, src, &ta, 200).await;
    let cid = r["contribution"]["id"].as_str().unwrap().to_string();
    let uri = format!("/contributions/{cid}/reallocate");
    let go = |tok: &str, target: Uuid| {
        let (app, uri, tok) = (app.clone(), uri.clone(), tok.to_string());
        async move { call(&app, "POST", &uri, Some(&tok), Some(Uuid::new_v4()), Some(json!({ "target_item_id": target }))).await }
    };
    let not_allowed = |r: &Resp| assert_eq!((r.0, r.2["code"].as_str()), (StatusCode::CONFLICT, Some("REALLOCATION_NOT_ALLOWED")), "{}", r.2);

    // 來源仍 open → 不可轉投
    not_allowed(&go(&ta, dst).await);
    sqlx::query("UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() - interval '1 day', funding_deadline = now() - interval '1 day' WHERE id = $1").bind(src).execute(&pool).await.unwrap();
    // 他人 403；跨清單 / 自己 / 不存在的目標 → 不可
    assert_eq!(go(&tb, dst).await.0, StatusCode::FORBIDDEN);
    not_allowed(&go(&ta, foreign).await);
    not_allowed(&go(&ta, src).await);
    not_allowed(&go(&ta, Uuid::new_v4()).await);
    // 無 Idempotency-Key
    assert_eq!(call(&app, "POST", &uri, Some(&ta), None, Some(json!({ "target_item_id": dst }))).await.0, StatusCode::BAD_REQUEST);

    let before = balance(&pool, a).await;
    let ledger_before: i64 = sqlx::query_scalar("SELECT count(*) FROM point_ledger").fetch_one(&pool).await.unwrap();
    let key = Uuid::new_v4();
    let body = json!({ "target_item_id": dst });
    let (st, _, b) = call(&app, "POST", &uri, Some(&ta), Some(key), Some(body.clone())).await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    assert_eq!(b["funded"], true);   // 200 點剛好填滿 dst
    assert_eq!(b["original"]["status"], "reallocated");
    assert_eq!(b["contribution"]["status"], "captured");
    assert_eq!(b["item"]["funding_status"], "funded");
    assert_eq!(item_row(&pool, src).await.0, 0);
    assert_eq!(item_row(&pool, dst).await, (200, "funded".into()));
    // 錢包不動、不寫 ledger
    assert_eq!(balance(&pool, a).await, before);
    let ledger_after: i64 = sqlx::query_scalar("SELECT count(*) FROM point_ledger").fetch_one(&pool).await.unwrap();
    assert_eq!(ledger_before, ledger_after);
    let po: i64 = sqlx::query_scalar("SELECT count(*) FROM purchase_orders WHERE item_id = $1").bind(dst).fetch_one(&pool).await.unwrap();
    assert_eq!(po, 1);
    // 重放同 key → 不會再轉一次
    let (st, h, _) = call(&app, "POST", &uri, Some(&ta), Some(key), Some(body)).await;
    assert_eq!((st, h["idempotency-replayed"].to_str().unwrap()), (StatusCode::CREATED, "true"));
    // 已轉投的不能再轉
    not_allowed(&go(&ta, dst).await);
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reallocate_rejects_overflow_and_expired_window(pool: PgPool) {
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let (a, ta) = user(&pool, "甲").await;
    grant(&pool, a, 1000).await;
    let wl = wishlist(&pool, owner).await;
    let src = cf_item(&pool, wl, 500).await;
    let small = cf_item(&pool, wl, 100).await;
    let (_, _, r) = pledge(&app, src, &ta, 200).await;
    let cid = r["contribution"]["id"].as_str().unwrap().to_string();
    sqlx::query("UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() - interval '1 day' WHERE id = $1").bind(src).execute(&pool).await.unwrap();
    let uri = format!("/contributions/{cid}/reallocate");
    let r = call(&app, "POST", &uri, Some(&ta), Some(Uuid::new_v4()), Some(json!({ "target_item_id": small }))).await;
    assert_eq!((r.0, r.2["code"].as_str()), (StatusCode::CONFLICT, Some("REALLOCATION_NOT_ALLOWED"))); // 200 > 目標 100
    sqlx::query("UPDATE wishlist_items SET expired_at = now() - interval '8 days' WHERE id = $1").bind(src).execute(&pool).await.unwrap();
    let big = cf_item(&pool, wl, 900).await;
    let r = call(&app, "POST", &uri, Some(&ta), Some(Uuid::new_v4()), Some(json!({ "target_item_id": big }))).await;
    assert_eq!((r.0, r.2["code"].as_str()), (StatusCode::CONFLICT, Some("REALLOCATION_NOT_ALLOWED"))); // 選擇期已過
    assert_eq!(item_row(&pool, src).await.0, 200);
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn tick_expires_then_auto_refunds_once(pool: PgPool) {
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let (a, ta) = user(&pool, "甲").await;
    let (b, tb) = user(&pool, "乙").await;
    grant(&pool, a, 1000).await;
    grant(&pool, b, 1000).await;
    let wl = wishlist(&pool, owner).await;
    let item = cf_item(&pool, wl, 500).await;
    let fresh = cf_item(&pool, wl, 500).await;
    pledge(&app, item, &ta, 100).await;
    pledge(&app, item, &tb, 150).await;
    pledge(&app, fresh, &ta, 50).await;

    // 未到期：不動
    assert_eq!(wishsync_api::contributions::tick_funding(&pool).await.unwrap(), (0, 0));
    sqlx::query("UPDATE wishlist_items SET funding_deadline = now() - interval '1 minute' WHERE id = $1").bind(item).execute(&pool).await.unwrap();
    assert_eq!(wishsync_api::contributions::tick_funding(&pool).await.unwrap(), (1, 0));
    let (pledged, fs) = item_row(&pool, item).await;
    assert_eq!((pledged, fs.as_str()), (250, "expired"));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind = 'funding.expired'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 2);
    // 重複執行：不再重複轉換 / 通知
    assert_eq!(wishsync_api::contributions::tick_funding(&pool).await.unwrap(), (0, 0));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind = 'funding.expired'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 2);
    assert_eq!(balance(&pool, a).await, 850); // 選擇期內不退

    // 選擇期結束 → 自動退點
    sqlx::query("UPDATE wishlist_items SET expired_at = now() - interval '8 days' WHERE id = $1").bind(item).execute(&pool).await.unwrap();
    assert_eq!(wishsync_api::contributions::tick_funding(&pool).await.unwrap(), (0, 2));
    assert_eq!(balance(&pool, a).await, 950); // 1000 - 50（fresh 仍 pledged）
    assert_eq!(balance(&pool, b).await, 1000);
    assert_eq!(item_row(&pool, item).await.0, 0);
    assert_eq!(item_row(&pool, fresh).await, (50, "open".into()));
    // 再跑不重複退點
    assert_eq!(wishsync_api::contributions::tick_funding(&pool).await.unwrap(), (0, 0));
    assert_eq!(balance(&pool, a).await, 950);
    let rel: i64 = sqlx::query_scalar("SELECT count(*) FROM point_ledger WHERE entry_type = 'release'").fetch_one(&pool).await.unwrap();
    assert_eq!(rel, 2);
    // 並行執行安全
    sqlx::query("UPDATE wishlist_items SET funding_deadline = now() - interval '1 minute' WHERE id = $1").bind(fresh).execute(&pool).await.unwrap();
    let (r1, r2) = tokio::join!(wishsync_api::contributions::tick_funding(&pool), wishsync_api::contributions::tick_funding(&pool));
    assert_eq!(r1.unwrap().0 + r2.unwrap().0, 1);
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn wallet_endpoint_held_points_and_pagination(pool: PgPool) {
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let (a, ta) = user(&pool, "甲").await;

    // 沒有錢包
    let (st, h, b) = call(&app, "GET", "/wallet", Some(&ta), None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(h["cache-control"], "private, no-store");
    assert_eq!((b["wallet"]["balance"].as_i64(), b["wallet"]["status"].as_str(), b["wallet"]["held_points"].as_i64()), (Some(0), Some("active"), Some(0)));
    assert!(b["wallet"]["id"].is_null());
    assert_eq!(call(&app, "GET", "/wallet", None, None, None).await.0, StatusCode::UNAUTHORIZED);

    grant(&pool, a, 1000).await;
    let wl = wishlist(&pool, owner).await;
    let open = cf_item(&pool, wl, 500).await;
    let done = cf_item(&pool, wl, 100).await;
    pledge(&app, open, &ta, 120).await;
    pledge(&app, done, &ta, 100).await; // captured
    let (_, _, b) = call(&app, "GET", "/wallet", Some(&ta), None, None).await;
    assert_eq!(b["wallet"]["balance"], 780);
    assert_eq!(b["wallet"]["held_points"], 220);
    assert_eq!(b["data"].as_array().unwrap().len(), 3);
    assert_eq!(b["data"][0]["entry_type"], "pledge");   // seq DESC
    assert_eq!(b["data"][2]["entry_type"], "grant");
    assert!(b["next_cursor"].is_null());
    // 部分退款後 held 只算實際花費
    sqlx::query("UPDATE contributions SET refunded_points = 10 WHERE item_id = $1").bind(done).execute(&pool).await.unwrap();
    let (_, _, b) = call(&app, "GET", "/wallet", Some(&ta), None, None).await;
    assert_eq!(b["wallet"]["held_points"], 210);

    // 分頁
    let (_, _, p1) = call(&app, "GET", "/wallet?limit=2", Some(&ta), None, None).await;
    assert_eq!(p1["data"].as_array().unwrap().len(), 2);
    let cur = p1["next_cursor"].as_str().unwrap();
    let (_, _, p2) = call(&app, "GET", &format!("/wallet?limit=2&cursor={cur}"), Some(&ta), None, None).await;
    assert_eq!(p2["data"].as_array().unwrap().len(), 1);
    assert_eq!(p2["data"][0]["entry_type"], "grant");
    assert!(p2["next_cursor"].is_null());
    assert_eq!(call(&app, "GET", "/wallet?cursor=abc", Some(&ta), None, None).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let _ = a;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn wallet_contributions_flags_and_filter(pool: PgPool) {
    let app = test_app(&pool);
    let (owner, _) = user(&pool, "小米").await;
    let (a, ta) = user(&pool, "甲").await;
    let (_, tb) = user(&pool, "乙").await;
    grant(&pool, a, 1000).await;
    let wl = wishlist(&pool, owner).await;
    let open = cf_item(&pool, wl, 500).await;
    let expired = cf_item(&pool, wl, 500).await;
    let late = cf_item(&pool, wl, 500).await;
    let done = cf_item(&pool, wl, 100).await;
    for it in [open, expired, late, done] { assert_eq!(pledge(&app, it, &ta, if it == done { 100 } else { 10 }).await.0, StatusCode::CREATED); }
    sqlx::query("UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() - interval '1 day' WHERE id = $1").bind(expired).execute(&pool).await.unwrap();
    sqlx::query("UPDATE wishlist_items SET funding_status = 'expired', expired_at = now() - interval '9 days' WHERE id = $1").bind(late).execute(&pool).await.unwrap();

    let (st, h, b) = call(&app, "GET", "/wallet/contributions", Some(&ta), None, None).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(h["cache-control"], "private, no-store");
    let rows = b["data"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    let by = |it: Uuid| rows.iter().find(|r| r["contribution"]["item_id"] == it.to_string()).unwrap();
    let flags = |it: Uuid| (by(it)["can_withdraw"].as_bool().unwrap(), by(it)["can_reallocate"].as_bool().unwrap());
    assert_eq!(flags(open), (true, false));
    assert_eq!(flags(expired), (true, true));
    assert_eq!(flags(late), (false, false));
    assert_eq!(flags(done), (false, false));
    assert_eq!(by(open)["item"]["display_status"], "open");
    assert_eq!(by(done)["item"]["display_status"], "funded");
    assert_eq!(by(expired)["item"]["display_status"], "expired");
    assert!(by(expired)["item"]["reallocation_deadline"].is_string());
    assert!(by(open)["item"]["reallocation_deadline"].is_null());
    assert!(by(open)["wishlist"]["slug"].is_string());
    assert!(by(open)["item"].get("image_url").is_some());
    // 篩選 + 分頁
    let (_, _, f) = call(&app, "GET", "/wallet/contributions?status=captured", Some(&ta), None, None).await;
    assert_eq!(f["data"].as_array().unwrap().len(), 1);
    assert_eq!(call(&app, "GET", "/wallet/contributions?status=bogus", Some(&ta), None, None).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, _, p1) = call(&app, "GET", "/wallet/contributions?limit=3", Some(&ta), None, None).await;
    assert_eq!(p1["data"].as_array().unwrap().len(), 3);
    let cur = p1["next_cursor"].as_str().unwrap();
    let (_, _, p2) = call(&app, "GET", &format!("/wallet/contributions?limit=3&cursor={cur}"), Some(&ta), None, None).await;
    assert_eq!(p2["data"].as_array().unwrap().len(), 1);
    assert!(p2["next_cursor"].is_null());
    // 只看得到自己的
    let (_, _, other) = call(&app, "GET", "/wallet/contributions", Some(&tb), None, None).await;
    assert_eq!(other["data"].as_array().unwrap().len(), 0);
}
