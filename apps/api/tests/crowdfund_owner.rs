//! P2-A 眾籌：建立者 / 公開頁 / 營運採購（認捐 API 由另一個模組負責，這裡用 SQL + points 核心組出「已有認捐 / 已達標」的狀態）
use axum::{body::Body, http::{Request, StatusCode}};
use chrono::{Duration, Utc};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wishsync_api::{app, points, session::hash_token, AppState};

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
fn idem(t: &str) -> [(&'static str, String); 2] { [("cookie", format!("ws_session={t}")), ("idempotency-key", Uuid::new_v4().to_string())] }

async fn user(pool: &PgPool, name: &str, staff: bool) -> (Uuid, String) {
    let id: Uuid = sqlx::query_scalar("INSERT INTO users (display_name, email, is_staff) VALUES ($1, $2, $3) RETURNING id")
        .bind(name).bind(format!("{}@example.com", Uuid::new_v4())).bind(staff).fetch_one(pool).await.unwrap();
    let tok = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(id).bind(hash_token(&tok)).execute(pool).await.unwrap();
    (id, tok)
}

/// 透過 API 建清單（registry）；extra 會合併進 body。回傳 (wishlist_id, slug)
async fn mk_list(pool: &PgPool, tok: &str, extra: Value) -> (Uuid, String) {
    let mut b = json!({ "type": "registry", "title": "寶寶清單", "show_claimer_names": true });
    b.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let (s, _, b) = call(pool, "POST", "/wishlists", &ck(tok), Some(b)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    (b["id"].as_str().unwrap().parse().unwrap(), b["slug"].as_str().unwrap().into())
}

async fn put_addr(pool: &PgPool, tok: &str, wid: Uuid) {
    let (s, _, b) = call(pool, "PUT", &format!("/wishlists/{wid}/shipping-address"), &ck(tok),
        Some(json!({ "recipient_name": "王小米", "phone": "0912345678", "address": "台北市大安區和平東路二段 1 號 3 樓" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

fn deadline() -> String { (Utc::now() + Duration::days(7)).to_rfc3339() }

async fn cf_item(pool: &PgPool, tok: &str, wid: Uuid, target: i64) -> Uuid {
    let (s, _, b) = call(pool, "POST", &format!("/wishlists/{wid}/items"), &ck(tok),
        Some(json!({ "title": "嬰兒推車", "funding_mode": "crowdfund", "target_points": target, "funding_deadline": deadline() }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b["id"].as_str().unwrap().parse().unwrap()
}

async fn publish(pool: &PgPool, tok: &str, wid: Uuid) {
    let (s, _, b) = call(pool, "PATCH", &format!("/wishlists/{wid}"), &ck(tok), Some(json!({ "status": "active" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

/// 營運發點到某使用者錢包（直接走 points::post）
async fn grant(pool: &PgPool, user: Uuid, n: i64) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    let w = points::ensure_wallet(&mut tx, user).await.unwrap();
    points::lock_wallets(&mut tx, &[w]).await.unwrap();
    points::post(&mut tx, w, n, points::Entry { kind: "grant", ref_type: "manual", ref_id: None, note: Some("測試"), actor: None }).await.unwrap();
    tx.commit().await.unwrap();
    w
}

/// 模擬一筆認捐（認捐 API 的效果）：發點 → 扣點 → INSERT contribution → 累加 pledged_points；剛好達標則 funded + capture_and_order
async fn pledge(pool: &PgPool, item: Uuid, user: Uuid, n: i64, anon: bool) -> Uuid {
    let w = grant(pool, user, n).await;
    let mut tx = pool.begin().await.unwrap();
    let (wid,): (Uuid,) = sqlx::query_as("SELECT wishlist_id FROM wishlist_items WHERE id = $1").bind(item).fetch_one(&mut *tx).await.unwrap();
    points::lock_wallets(&mut tx, &[w]).await.unwrap();
    let cid = Uuid::new_v4();
    points::post(&mut tx, w, -n, points::Entry::contribution("pledge", cid)).await.unwrap();
    sqlx::query("INSERT INTO contributions (id, item_id, wishlist_id, user_id, wallet_id, donor_name, points, is_anonymous, message)
                 SELECT $1, $2, $3, $4, $5, u.display_name, $6, $7, '加油' FROM users u WHERE u.id = $4")
        .bind(cid).bind(item).bind(wid).bind(user).bind(w).bind(n).bind(anon).execute(&mut *tx).await.unwrap();
    let (pledged, target): (i64, i64) = sqlx::query_as("UPDATE wishlist_items SET pledged_points = pledged_points + $2 WHERE id = $1 RETURNING pledged_points, target_points")
        .bind(item).bind(n).fetch_one(&mut *tx).await.unwrap();
    if pledged == target {
        sqlx::query("UPDATE wishlist_items SET funding_status = 'funded' WHERE id = $1").bind(item).execute(&mut *tx).await.unwrap();
        points::capture_and_order(&mut tx, item).await.unwrap();
    }
    tx.commit().await.unwrap();
    cid
}

async fn balance(pool: &PgPool, user: Uuid) -> i64 {
    sqlx::query_scalar("SELECT coalesce(sum(balance), 0)::bigint FROM point_wallets WHERE user_id = $1").bind(user).fetch_one(pool).await.unwrap()
}

/// 對帳不變式：每個錢包 balance = Σ ledger.delta
async fn assert_reconciled(pool: &PgPool) {
    let bad: Vec<(Uuid, i64, i64)> = sqlx::query_as(
        "SELECT w.id, w.balance, coalesce(sum(l.delta), 0)::bigint FROM point_wallets w LEFT JOIN point_ledger l ON l.wallet_id = w.id GROUP BY w.id HAVING w.balance <> coalesce(sum(l.delta), 0)")
        .fetch_all(pool).await.unwrap();
    assert!(bad.is_empty(), "balance 與 ledger 加總不一致: {bad:?}");
}

async fn order_of(pool: &PgPool, item: Uuid) -> Uuid { sqlx::query_scalar("SELECT id FROM purchase_orders WHERE item_id = $1").bind(item).fetch_one(pool).await.unwrap() }

// ================= 建立眾籌品項 =================
#[sqlx::test(migrations = "../../db/migrations")]
async fn create_crowdfund_item_rules(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (wid, _) = mk_list(&pool, &t, json!({})).await;
    let path = format!("/wishlists/{wid}/items");
    let body = json!({ "title": "推車", "funding_mode": "crowdfund", "target_points": 5000, "funding_deadline": deadline() });

    // 沒有收件資訊
    let (s, _, b) = call(&pool, "POST", &path, &ck(&t), Some(body.clone())).await;
    assert_eq!((s.as_u16(), b["code"].as_str()), (409, Some("SHIPPING_ADDRESS_REQUIRED")), "{b}");
    put_addr(&pool, &t, wid).await;

    // target 規則
    for bad in [json!({ "target_points": 0 }), json!({ "target_points": -5 }), json!({ "target_points": "5000" }), json!({ "target_points": null })] {
        let mut x = body.clone(); x.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
        let (s, _, b) = call(&pool, "POST", &path, &ck(&t), Some(x)).await;
        assert_eq!((s.as_u16(), b["code"].as_str()), (422, Some("VALIDATION_FAILED")), "{b}");
    }
    let mut x = body.clone(); x.as_object_mut().unwrap().remove("target_points");
    assert_eq!(call(&pool, "POST", &path, &ck(&t), Some(x)).await.0.as_u16(), 422);

    // deadline：過去 / 格式錯 / 省略且清單沒有 event_date
    for d in [json!((Utc::now() - Duration::hours(1)).to_rfc3339()), json!("明天"), json!(123)] {
        let mut x = body.clone(); x["funding_deadline"] = d;
        let (s, _, b) = call(&pool, "POST", &path, &ck(&t), Some(x)).await;
        assert_eq!((s.as_u16(), b["code"].as_str()), (422, Some("VALIDATION_FAILED")), "{b}");
    }
    let mut x = body.clone(); x.as_object_mut().unwrap().remove("funding_deadline");
    let (s, _, b) = call(&pool, "POST", &path, &ck(&t), Some(x.clone())).await;
    assert_eq!((s.as_u16(), b["errors"][0]["pointer"].as_str()), (422, Some("/funding_deadline")), "{b}");

    // 成功：伺服器強制 qty_needed=1 / concierge / open
    let mut ok = body.clone(); ok["qty_needed"] = json!(5);
    let (s, _, b) = call(&pool, "POST", &path, &ck(&t), Some(ok)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!((b["funding_mode"].as_str(), b["qty_needed"].as_i64(), b["fulfillment_type"].as_str(), b["funding_status"].as_str(), b["target_points"].as_i64(), b["pledged_points"].as_i64()),
               (Some("crowdfund"), Some(1), Some("concierge"), Some("open"), Some(5000), Some(0)), "{b}");
    assert_eq!(b["display_status"], "open");

    // 省略 deadline 且清單有 event_date → 當天 23:59（台北）= 15:59Z
    let day = (Utc::now() + Duration::days(30)).format("%Y-%m-%d").to_string();
    let (s, _, _) = call(&pool, "PATCH", &format!("/wishlists/{wid}"), &ck(&t), Some(json!({ "event_date": day }))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, b) = call(&pool, "POST", &path, &ck(&t), Some(x)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["funding_deadline"].as_str(), Some(format!("{day}T15:59:00Z").as_str()), "{b}");

    // GET 清單：has_shipping_address
    let (_, _, b) = call(&pool, "GET", &format!("/wishlists/{wid}"), &ck(&t), None).await;
    assert_eq!(b["wishlist"]["has_shipping_address"], true);
    assert_eq!(b["items"][0]["target_points"], 5000);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn edit_rules_after_pledges(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (a, _) = user(&pool, "阿明", false).await;
    let (wid, _) = mk_list(&pool, &t, json!({})).await;
    put_addr(&pool, &t, wid).await;
    let item = cf_item(&pool, &t, wid, 1000).await;
    let path = format!("/items/{item}");

    // 沒人捐時可改 target / 改回 quantity
    let (s, _, b) = call(&pool, "PATCH", &path, &ck(&t), Some(json!({ "target_points": 2000 }))).await;
    assert_eq!((s, b["target_points"].as_i64()), (StatusCode::OK, Some(2000)), "{b}");
    let (s, _, b) = call(&pool, "PATCH", &path, &ck(&t), Some(json!({ "funding_mode": "quantity", "qty_needed": 2 }))).await;
    assert_eq!((s, b["funding_mode"].as_str(), b["target_points"].is_null(), b["funding_status"].is_null()), (StatusCode::OK, Some("quantity"), true, true), "{b}");
    let (s, _, b) = call(&pool, "PATCH", &path, &ck(&t), Some(json!({ "funding_mode": "crowdfund", "target_points": 1000, "funding_deadline": deadline() }))).await;
    assert_eq!((s, b["qty_needed"].as_i64(), b["funding_status"].as_str()), (StatusCode::OK, Some(1), Some("open")), "{b}");

    pledge(&pool, item, a, 400, false).await;
    // 有認捐後不可改 funding_mode
    let (s, _, b) = call(&pool, "PATCH", &path, &ck(&t), Some(json!({ "funding_mode": "quantity" }))).await;
    assert_eq!((s.as_u16(), b["code"].as_str()), (409, Some("FUNDING_MODE_LOCKED")), "{b}");
    // target 必須 > pledged（等於也拒絕）
    for t_ in [100, 400] {
        let (s, _, b) = call(&pool, "PATCH", &path, &ck(&t), Some(json!({ "target_points": t_ }))).await;
        assert_eq!((s.as_u16(), b["code"].as_str()), (422, Some("VALIDATION_FAILED")), "{b}");
    }
    let (s, _, b) = call(&pool, "PATCH", &path, &ck(&t), Some(json!({ "target_points": 401 }))).await;
    assert_eq!((s, b["target_points"].as_i64(), b["pledged_points"].as_i64()), (StatusCode::OK, Some(401), Some(400)), "{b}");
    // 一般欄位仍可改，qty_needed 被強制為 1
    let (s, _, b) = call(&pool, "PATCH", &path, &ck(&t), Some(json!({ "title": "新名稱", "qty_needed": 9 }))).await;
    assert_eq!((s, b["title"].as_str(), b["qty_needed"].as_i64()), (StatusCode::OK, Some("新名稱"), Some(1)), "{b}");

    // 曾有認捐（即使已撤回）仍不可改模式
    let item2 = cf_item(&pool, &t, wid, 1000).await;
    let cid = pledge(&pool, item2, a, 100, false).await;
    let mut tx = pool.begin().await.unwrap();
    points::release(&mut tx, &[cid], &["pledged"]).await.unwrap();
    sqlx::query("UPDATE wishlist_items SET pledged_points = 0 WHERE id = $1").bind(item2).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let (s, _, b) = call(&pool, "PATCH", &format!("/items/{item2}"), &ck(&t), Some(json!({ "funding_mode": "quantity" }))).await;
    assert_eq!((s.as_u16(), b["code"].as_str()), (409, Some("FUNDING_MODE_LOCKED")), "{b}");

    // 達標後不可改 target / deadline
    let (b_id, _) = user(&pool, "阿華", false).await;
    pledge(&pool, item, b_id, 1, false).await; // 401 = target
    let (s, _, b) = call(&pool, "PATCH", &path, &ck(&t), Some(json!({ "target_points": 9999 }))).await;
    assert_eq!((s.as_u16(), b["code"].as_str()), (409, Some("FUNDING_LOCKED")), "{b}");
    let (s, _, b) = call(&pool, "PATCH", &path, &ck(&t), Some(json!({ "funding_deadline": deadline() }))).await;
    assert_eq!((s.as_u16(), b["code"].as_str()), (409, Some("FUNDING_LOCKED")), "{b}");
    // 同值重送（整份表單 PATCH）不算修改
    let (s, _, b) = call(&pool, "PATCH", &path, &ck(&t), Some(json!({ "target_points": 401, "title": "再改名" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["funding_status"], "funded");
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn publish_requires_shipping_address(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (wid, _) = mk_list(&pool, &t, json!({})).await;
    put_addr(&pool, &t, wid).await;
    cf_item(&pool, &t, wid, 1000).await;
    sqlx::query("DELETE FROM shipping_addresses WHERE wishlist_id = $1").bind(wid).execute(&pool).await.unwrap();
    let (s, _, b) = call(&pool, "PATCH", &format!("/wishlists/{wid}"), &ck(&t), Some(json!({ "status": "active" }))).await;
    assert_eq!((s.as_u16(), b["code"].as_str()), (409, Some("WISHLIST_NOT_PUBLISHABLE")), "{b}");
    assert!(b["errors"].as_array().unwrap().iter().any(|e| e["code"] == "SHIPPING_ADDRESS_REQUIRED"), "{b}");
    put_addr(&pool, &t, wid).await;
    publish(&pool, &t, wid).await;
}

// ================= 收件資訊 =================
#[sqlx::test(migrations = "../../db/migrations")]
async fn shipping_address_masked_and_encrypted(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (_, t2) = user(&pool, "路人", false).await;
    let (wid, _) = mk_list(&pool, &t, json!({})).await;
    let path = format!("/wishlists/{wid}/shipping-address");

    let (s, _, b) = call(&pool, "GET", &path, &ck(&t), None).await;
    assert_eq!((s, b), (StatusCode::OK, json!({ "has_shipping_address": false })));

    let body = json!({ "recipient_name": "王小米", "phone": "0912345678", "address": "台北市大安區和平東路二段 1 號 3 樓" });
    let (s, _, b) = call(&pool, "PUT", &path, &ck(&t), Some(body.clone())).await;
    let want = json!({ "has_shipping_address": true, "recipient_name": "王*米", "phone": "0912***678", "address": "台北市大安區***" });
    assert_eq!((s, &b), (StatusCode::OK, &want));
    assert_eq!(call(&pool, "GET", &path, &ck(&t), None).await.2, want);

    // DB 內是密文：任何明文片段都不得出現
    let (n, p, a): (Vec<u8>, Vec<u8>, Vec<u8>) = sqlx::query_as("SELECT recipient_name_enc, phone_enc, address_enc FROM shipping_addresses WHERE wishlist_id = $1")
        .bind(wid).fetch_one(&pool).await.unwrap();
    let has = |hay: &[u8], needle: &str| hay.windows(needle.len()).any(|w| w == needle.as_bytes());
    assert!(!has(&n, "王小米") && !has(&p, "0912345678") && !has(&a, "和平東路"), "收件資訊不得以明文存放");
    assert_eq!(wishsync_api::sealed::open_str(&a).as_deref(), Some("台北市大安區和平東路二段 1 號 3 樓"));

    // 覆蓋更新
    let (s, _, b) = call(&pool, "PUT", &path, &ck(&t), Some(json!({ "recipient_name": "李大華", "phone": "+886 912-345-678", "address": "新北市板橋區" }))).await;
    assert_eq!((s, b["recipient_name"].as_str()), (StatusCode::OK, Some("李*華")), "{b}");
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM shipping_addresses WHERE wishlist_id = $1").bind(wid).fetch_one(&pool).await.unwrap(), 1);

    // 驗證
    for bad in [json!({ "recipient_name": "", "phone": "0912345678", "address": "台北市大安區" }),
                json!({ "recipient_name": "王", "phone": "12345", "address": "台北市大安區" }),
                json!({ "recipient_name": "王", "phone": "09123abc78", "address": "台北市大安區" }),
                json!({ "recipient_name": "王", "phone": "0912345678", "address": "短" }),
                json!({ "recipient_name": "王", "phone": "0912345678" })] {
        let (s, _, b) = call(&pool, "PUT", &path, &ck(&t), Some(bad)).await;
        assert_eq!((s.as_u16(), b["code"].as_str()), (422, Some("VALIDATION_FAILED")), "{b}");
    }
    // 非擁有者 / 未登入
    assert_eq!(call(&pool, "GET", &path, &ck(&t2), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "PUT", &path, &ck(&t2), Some(body)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "GET", &path, &[], None).await.0, StatusCode::UNAUTHORIZED);
}

// ================= dashboard =================
#[sqlx::test(migrations = "../../db/migrations")]
async fn dashboard_lists_contributors(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (a, _) = user(&pool, "阿明", false).await;
    let (b_, _) = user(&pool, "小華", false).await;
    let (c, _) = user(&pool, "路人C", false).await;
    let (wid, _) = mk_list(&pool, &t, json!({})).await;
    put_addr(&pool, &t, wid).await;
    let open_item = cf_item(&pool, &t, wid, 1000).await;
    let funded_item = cf_item(&pool, &t, wid, 500).await;
    pledge(&pool, open_item, a, 300, false).await;
    pledge(&pool, open_item, b_, 200, true).await; // 匿名
    let gone = pledge(&pool, open_item, c, 50, false).await; // 之後撤回
    let mut tx = pool.begin().await.unwrap();
    points::release(&mut tx, &[gone], &["pledged"]).await.unwrap();
    sqlx::query("UPDATE wishlist_items SET pledged_points = pledged_points - 50 WHERE id = $1").bind(open_item).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    pledge(&pool, funded_item, a, 500, false).await; // 一筆就達標 → captured

    let (s, _, d) = call(&pool, "GET", &format!("/wishlists/{wid}/dashboard"), &ck(&t), None).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    let cs = d["contributions"].as_array().unwrap();
    assert_eq!(cs.len(), 3, "只列 pledged / captured: {d}"); // released 的不列
    let by = |name: &str| cs.iter().filter(|c| c["display_name"] == name).collect::<Vec<_>>();
    assert_eq!(by("阿明").len(), 2);
    let anon = by("匿名朋友");
    assert_eq!((anon.len(), anon[0]["points"].as_i64(), anon[0]["status"].as_str()), (1, Some(200), Some("pledged")));
    assert!(!d.to_string().contains("example.com"), "不得含 email");
    assert!(!d.to_string().contains("小華"), "匿名者的暱稱不得外洩");
    let pledged = by("阿明").into_iter().find(|c| c["item_id"] == json!(open_item)).unwrap();
    assert_eq!((pledged["points"].as_i64(), pledged["spent_points"].as_i64(), pledged["message"].as_str(), pledged["status"].as_str()), (Some(300), Some(300), Some("加油"), Some("pledged")));
    assert!(pledged["created_at"].is_string() && pledged["captured_at"].is_null());
    let captured = by("阿明").into_iter().find(|c| c["item_id"] == json!(funded_item)).unwrap();
    assert_eq!(captured["status"], "captured");
    assert!(captured["captured_at"].is_string(), "達標時間: {captured}");
    // 依 created_at 排序
    let times: Vec<&str> = cs.iter().map(|c| c["created_at"].as_str().unwrap()).collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]));

    assert_eq!((d["totals"]["target_points"].as_i64(), d["totals"]["pledged_points"].as_i64(), d["totals"]["funded_item_count"].as_i64()), (Some(1500), Some(1000), Some(1)));
    assert_eq!(d["orders_summary"]["funded_count"], 1);
    let it = |id: Uuid| d["items"].as_array().unwrap().iter().find(|i| i["item_id"] == json!(id)).unwrap().clone();
    assert_eq!((it(open_item)["pledged_points"].as_i64(), it(open_item)["funding_status"].as_str()), (Some(500), Some("open")));
    assert_eq!((it(funded_item)["display_status"].as_str(), it(funded_item)["funded_at"].is_string()), (Some("funded"), true));
    // 完成度：1 個眾籌品項達標 / 共 2
    assert_eq!(d["totals"]["completion_pct"], 50);

    // 非擁有者看不到
    let (_, t2) = user(&pool, "別人", false).await;
    assert_eq!(call(&pool, "GET", &format!("/wishlists/{wid}/dashboard"), &ck(&t2), None).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn dashboard_and_orders_hide_everything_while_surprise_locked(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (a, _) = user(&pool, "阿明", false).await;
    let day = (Utc::now() + Duration::days(30)).format("%Y-%m-%d").to_string();
    let (wid, _) = mk_list(&pool, &t, json!({ "surprise_mode": true, "event_date": day })).await;
    put_addr(&pool, &t, wid).await;
    let item = cf_item(&pool, &t, wid, 500).await;
    pledge(&pool, item, a, 500, false).await; // 達標並建立採購單

    let (_, _, d) = call(&pool, "GET", &format!("/wishlists/{wid}/dashboard"), &ck(&t), None).await;
    assert_eq!(d["surprise_locked"], true);
    assert!(d["contributions"].is_null() && d["claims"].is_null(), "{d}");
    assert!(d["totals"]["pledged_points"].is_null() && d["totals"]["funded_item_count"].is_null(), "{d}");
    let i = &d["items"][0];
    assert!(i["pledged_points"].is_null() && i["funding_status"].is_null() && i["display_status"].is_null() && i["funded_at"].is_null(), "{i}");
    assert!(!d.to_string().contains("阿明"));

    let (s, _, o) = call(&pool, "GET", &format!("/wishlists/{wid}/orders"), &ck(&t), None).await;
    assert_eq!(s, StatusCode::OK, "{o}");
    assert!(o["data"].is_null() && o["surprise_locked"] == true, "{o}");
    assert_eq!(o["summary"]["funded_count"], 1);

    // 擁有者 GET 品項：眾籌狀態遮蔽；改 target 一律通用 403（不可洩漏「已達標」）
    let (_, _, w) = call(&pool, "GET", &format!("/wishlists/{wid}"), &ck(&t), None).await;
    assert!(w["items"][0]["pledged_points"].is_null() && w["items"][0]["funding_status"].is_null(), "{w}");
    let (s, _, b) = call(&pool, "PATCH", &format!("/items/{item}"), &ck(&t), Some(json!({ "target_points": 9000 }))).await;
    assert_eq!((s.as_u16(), b["code"].as_str()), (403, Some("FORBIDDEN")), "{b}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn owner_orders_progress(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (_, s) = user(&pool, "營運", true).await;
    let (a, _) = user(&pool, "阿明", false).await;
    let (wid, _) = mk_list(&pool, &t, json!({})).await;
    put_addr(&pool, &t, wid).await;
    let item = cf_item(&pool, &t, wid, 500).await;
    pledge(&pool, item, a, 500, false).await;
    let oid = order_of(&pool, item).await;
    for (st, extra) in [("placed", json!({ "merchant_order_id": "SHOP-1", "amount": 500 })), ("shipped", json!({ "tracking_no": "TW123" }))] {
        let mut b = json!({ "status": st }); b.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert_eq!(call(&pool, "PATCH", &format!("/admin/orders/{oid}"), &ck(&s), Some(b)).await.0, StatusCode::OK);
    }
    let (st, _, o) = call(&pool, "GET", &format!("/wishlists/{wid}/orders"), &ck(&t), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!((o["summary"]["shipped_count"].as_i64(), o["data"][0]["status"].as_str(), o["data"][0]["tracking_no"].as_str(), o["data"][0]["item_title"].as_str()),
               (Some(1), Some("shipped"), Some("TW123"), Some("嬰兒推車")), "{o}");
    assert!(o["data"][0]["shipped_at"].is_string() && o["data"][0]["placed_at"].is_string());
    assert!(!o.to_string().contains("阿明") && !o.to_string().contains("和平東路"), "擁有者看不到捐贈者與收件人以外的資料");
}

// ================= 公開頁 =================
#[sqlx::test(migrations = "../../db/migrations")]
async fn public_page_funding_fields_and_contributors(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (a, _) = user(&pool, "阿明", false).await;
    let (b_, _) = user(&pool, "小華", false).await;
    let (c, _) = user(&pool, "路人C", false).await;
    let (wid, slug) = mk_list(&pool, &t, json!({})).await;
    put_addr(&pool, &t, wid).await;
    let item = cf_item(&pool, &t, wid, 1000).await;
    let done = cf_item(&pool, &t, wid, 100).await;
    pledge(&pool, item, a, 100, false).await;
    pledge(&pool, item, a, 200, false).await; // 同一位非匿名 → 合併
    pledge(&pool, item, b_, 50, true).await;
    pledge(&pool, item, b_, 70, true).await; // 匿名各自一筆
    let gone = pledge(&pool, item, c, 30, false).await;
    let mut tx = pool.begin().await.unwrap();
    points::release(&mut tx, &[gone], &["pledged"]).await.unwrap();
    sqlx::query("UPDATE wishlist_items SET pledged_points = pledged_points - 30 WHERE id = $1").bind(item).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    pledge(&pool, done, c, 100, false).await;
    publish(&pool, &t, wid).await;

    let (s, _, p) = call(&pool, "GET", &format!("/public/wishlists/{slug}"), &[], None).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let find = |id: Uuid| p["items"].as_array().unwrap().iter().find(|i| i["id"] == json!(id)).unwrap().clone();
    let i = find(item);
    assert_eq!((i["target_points"].as_i64(), i["pledged_points"].as_i64(), i["remaining_points"].as_i64(), i["progress_percent"].as_i64()), (Some(1000), Some(420), Some(580), Some(42)), "{i}");
    assert_eq!((i["funding_status"].as_str(), i["display_status"].as_str(), i["is_fully_claimed"].as_bool()), (Some("open"), Some("open"), Some(false)));
    assert!(i["funding_deadline"].is_string());
    let cs = i["contributors"].as_array().unwrap();
    assert_eq!(cs.len(), 3, "{cs:?}"); // 阿明(合併) + 匿名 x2；撤回的不列
    assert_eq!(cs[0], json!({ "display_name": "阿明", "points": 300 }));
    assert_eq!(cs.iter().filter(|c| c["display_name"] == "匿名朋友").map(|c| c["points"].as_i64().unwrap()).collect::<Vec<_>>(), vec![50, 70]);
    assert!(!p.to_string().contains("小華") && !p.to_string().contains("example.com"));
    let d = find(done);
    assert_eq!((d["display_status"].as_str(), d["is_fully_claimed"].as_bool(), d["progress_percent"].as_i64()), (Some("funded"), Some(true), Some(100)));
    assert_eq!(d["contributors"], json!([{ "display_name": "路人C", "points": 100 }]));
    assert_eq!((p["completion"]["fulfilled_count"].as_i64(), p["completion"]["completion_pct"].as_i64()), (Some(1), Some(50)), "{}", p["completion"]);

    // show_claimer_names 關閉 → 不輸出 contributors（金額彙總照常）
    call(&pool, "PATCH", &format!("/wishlists/{wid}"), &ck(&t), Some(json!({ "show_claimer_names": false }))).await;
    let (_, _, p) = call(&pool, "GET", &format!("/public/wishlists/{slug}"), &[], None).await;
    assert!(p["items"][0].get("contributors").is_none(), "{p}");
    assert_eq!(p["items"][0]["pledged_points"], 420);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_page_hides_contributors_while_surprise_locked(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (a, _) = user(&pool, "阿明", false).await;
    let day = (Utc::now() + Duration::days(30)).format("%Y-%m-%d").to_string();
    let (wid, slug) = mk_list(&pool, &t, json!({ "surprise_mode": true, "event_date": day })).await;
    put_addr(&pool, &t, wid).await;
    let item = cf_item(&pool, &t, wid, 1000).await;
    pledge(&pool, item, a, 100, false).await;
    publish(&pool, &t, wid).await;
    let (_, _, p) = call(&pool, "GET", &format!("/public/wishlists/{slug}"), &[], None).await;
    assert_eq!(p["claimers_visible"], false);
    assert!(p["items"][0].get("contributors").is_none() && !p.to_string().contains("阿明"), "{p}");
}

// ================= 營運：採購單 =================
#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_order_placed_partial_refund_and_lifecycle(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (sid, s) = user(&pool, "營運", true).await;
    let (a, _) = user(&pool, "阿明", false).await;
    let (b_, _) = user(&pool, "小華", false).await;
    let (wid, _) = mk_list(&pool, &t, json!({})).await;
    put_addr(&pool, &t, wid).await;
    let item = cf_item(&pool, &t, wid, 7600).await;
    pledge(&pool, item, a, 1600, false).await;
    pledge(&pool, item, b_, 6000, false).await;
    let oid = order_of(&pool, item).await;
    let url = format!("/admin/orders/{oid}");

    // 佇列：含解密後的收件資訊，且逐筆寫 audit
    let (st, h, l) = call(&pool, "GET", "/admin/orders?status=pending", &ck(&s), None).await;
    assert_eq!(st, StatusCode::OK, "{l}");
    assert_eq!(h["cache-control"], "private, no-store");
    assert_eq!(l["data"][0]["shipping_address"], json!({ "recipient_name": "王小米", "phone": "0912345678", "address": "台北市大安區和平東路二段 1 號 3 樓" }));
    assert_eq!((l["data"][0]["target_points"].as_i64(), l["data"][0]["item"]["title"].as_str(), l["data"][0]["wishlist"]["title"].as_str()), (Some(7600), Some("嬰兒推車"), Some("寶寶清單")));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = 'order.view_address' AND entity_id = $1 AND actor_id = $2").bind(oid).bind(sid).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1, "讀取收件地址必須寫 audit");
    assert_eq!(call(&pool, "GET", "/admin/orders?status=bogus", &ck(&s), None).await.0.as_u16(), 422);

    // 實際花費超過 target → 422，狀態不變、不動點數
    let (st, _, b) = call(&pool, "PATCH", &url, &ck(&s), Some(json!({ "status": "placed", "merchant_order_id": "SHOP-1", "amount": 7601 }))).await;
    assert_eq!((st.as_u16(), b["code"].as_str()), (422, Some("VALIDATION_FAILED")), "{b}");
    // 缺欄位
    assert_eq!(call(&pool, "PATCH", &url, &ck(&s), Some(json!({ "status": "placed", "amount": 7500 }))).await.0.as_u16(), 422);
    assert_eq!(call(&pool, "PATCH", &url, &ck(&s), Some(json!({ "status": "placed", "merchant_order_id": "X" }))).await.0.as_u16(), 422);
    assert_eq!(sqlx::query_scalar::<_, String>("SELECT status::text FROM purchase_orders WHERE id = $1").bind(oid).fetch_one(&pool).await.unwrap(), "pending");
    assert_eq!(balance(&pool, a).await + balance(&pool, b_).await, 0);

    // 不合法轉移
    for to in ["shipped", "delivered", "pending"] {
        let (st, _, b) = call(&pool, "PATCH", &url, &ck(&s), Some(json!({ "status": to }))).await;
        assert_eq!((st.as_u16(), b["code"].as_str()), (409, Some("INVALID_STATE_TRANSITION")), "{to}: {b}");
    }

    // placed：7500 → 差額 100，依 1600:6000 = 21 / 79
    let (st, _, b) = call(&pool, "PATCH", &url, &ck(&s), Some(json!({ "status": "placed", "merchant_order_id": "SHOP-1", "amount": 7500 }))).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!((b["refunded_points"].as_i64(), b["amount"].as_i64(), b["status"].as_str(), b["operator"]["user_id"].as_str()), (Some(100), Some(7500), Some("placed"), Some(sid.to_string().as_str())), "{b}");
    assert_eq!((balance(&pool, a).await, balance(&pool, b_).await), (21, 79));
    let refunds: Vec<(i64,)> = sqlx::query_as("SELECT delta FROM point_ledger WHERE entry_type = 'refund' ORDER BY delta").fetch_all(&pool).await.unwrap();
    assert_eq!(refunds, vec![(21,), (79,)]);
    let spent: Vec<(i64,)> = sqlx::query_as("SELECT points - refunded_points FROM contributions ORDER BY points").fetch_all(&pool).await.unwrap();
    assert_eq!(spent, vec![(1579,), (5921,)]);
    // 重複 placed → 409（不會再退一次）
    let (st, _, _) = call(&pool, "PATCH", &url, &ck(&s), Some(json!({ "status": "placed", "merchant_order_id": "SHOP-1", "amount": 7000 }))).await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(balance(&pool, a).await + balance(&pool, b_).await, 100);

    // shipped → delivered；通知捐贈者（每人一封）
    let (st, _, b) = call(&pool, "PATCH", &url, &ck(&s), Some(json!({ "status": "shipped", "tracking_no": "TW999" }))).await;
    assert_eq!((st, b["tracking_no"].as_str()), (StatusCode::OK, Some("TW999")), "{b}");
    let (st, _, b) = call(&pool, "PATCH", &url, &ck(&s), Some(json!({ "status": "delivered" }))).await;
    assert_eq!((st, b["tracking_no"].as_str()), (StatusCode::OK, Some("TW999")), "{b}");
    let (fs, ds): (String, Option<chrono::DateTime<Utc>>) = sqlx::query_as("SELECT i.funding_status::text, po.delivered_at FROM wishlist_items i JOIN purchase_orders po ON po.item_id = i.id WHERE i.id = $1")
        .bind(item).fetch_one(&pool).await.unwrap();
    assert_eq!(fs, "fulfilled");
    assert!(ds.is_some());
    for k in ["order.shipped", "order.delivered"] {
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM notifications WHERE kind = $1").bind(k).fetch_one(&pool).await.unwrap(), 2, "{k}");
    }
    // 終態不可再轉
    assert_eq!(call(&pool, "PATCH", &url, &ck(&s), Some(json!({ "status": "failed", "failure_reason": "x" }))).await.0, StatusCode::CONFLICT);
    // 擁有者 dashboard 反映
    let (_, _, d) = call(&pool, "GET", &format!("/wishlists/{wid}/dashboard"), &ck(&t), None).await;
    assert_eq!((d["orders_summary"]["delivered_count"].as_i64(), d["items"][0]["display_status"].as_str()), (Some(1), Some("delivered")));
    // audit
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_logs WHERE action = 'order.update' AND entity_id = $1").bind(oid).fetch_one(&pool).await.unwrap(), 3);
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_order_failed_refunds_everything(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (_, s) = user(&pool, "營運", true).await;
    let (a, _) = user(&pool, "阿明", false).await;
    let (b_, _) = user(&pool, "小華", false).await;
    let (wid, _) = mk_list(&pool, &t, json!({})).await;
    put_addr(&pool, &t, wid).await;
    let item = cf_item(&pool, &t, wid, 7600).await;
    pledge(&pool, item, a, 1600, false).await;
    pledge(&pool, item, b_, 6000, false).await;
    let oid = order_of(&pool, item).await;
    let url = format!("/admin/orders/{oid}");
    call(&pool, "PATCH", &url, &ck(&s), Some(json!({ "status": "placed", "merchant_order_id": "SHOP-1", "amount": 7500 }))).await; // 先部分退 21 / 79

    // failure_reason 必填
    let (st, _, b) = call(&pool, "PATCH", &url, &ck(&s), Some(json!({ "status": "failed" }))).await;
    assert_eq!((st.as_u16(), b["code"].as_str()), (422, Some("VALIDATION_FAILED")), "{b}");

    let (st, _, b) = call(&pool, "PATCH", &url, &ck(&s), Some(json!({ "status": "failed", "failure_reason": "商家缺貨" }))).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!((b["status"].as_str(), b["refunded_points"].as_i64(), b["failure_reason"].as_str()), (Some("failed"), Some(7500), Some("商家缺貨")), "{b}");
    // 全額退回 = 已退差額 + 這次退回的剩餘（扣除已部分退款的點數）
    assert_eq!((balance(&pool, a).await, balance(&pool, b_).await), (1600, 6000));
    let (ps, fs, exp): (i64, String, Option<chrono::DateTime<Utc>>) = sqlx::query_as("SELECT pledged_points, funding_status::text, expired_at FROM wishlist_items WHERE id = $1").bind(item).fetch_one(&pool).await.unwrap();
    assert_eq!((ps, fs.as_str(), exp.is_none()), (0, "open", true));
    let cs: Vec<(String, i64)> = sqlx::query_as("SELECT status::text, refunded_points - points FROM contributions").fetch_all(&pool).await.unwrap();
    assert!(cs.iter().all(|c| c.0 == "released" && c.1 == 0));
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM notifications WHERE kind = 'order.failed'").fetch_one(&pool).await.unwrap(), 2);
    assert_reconciled(&pool).await;

    // 品項可再被認捐達標：重用同一張採購單（capture_and_order 的 upsert），pending 重新開始
    pledge(&pool, item, a, 7600, false).await;
    assert_eq!(order_of(&pool, item).await, oid);
    // 取消（cancelled）同樣處理；已過期限 → 品項轉 expired
    sqlx::query("UPDATE wishlist_items SET funding_deadline = now() - interval '1 hour' WHERE id = $1").bind(item).execute(&pool).await.unwrap();
    let (st, _, b) = call(&pool, "PATCH", &url, &ck(&s), Some(json!({ "status": "cancelled", "failure_reason": "受贈者取消" }))).await;
    assert_eq!((st, b["status"].as_str(), b["refunded_points"].as_i64()), (StatusCode::OK, Some("cancelled"), Some(7600)), "{b}");
    let (ps, fs, exp): (i64, String, Option<chrono::DateTime<Utc>>) = sqlx::query_as("SELECT pledged_points, funding_status::text, expired_at FROM wishlist_items WHERE id = $1").bind(item).fetch_one(&pool).await.unwrap();
    assert_eq!((ps, fs.as_str(), exp.is_some()), (0, "expired", true));
    assert_eq!(balance(&pool, a).await, 1600 + 7600);
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_endpoints_staff_only(pool: PgPool) {
    let (_, t) = user(&pool, "一般人", false).await;
    let (u, _) = user(&pool, "某人", false).await;
    let id = Uuid::new_v4();
    let key = Uuid::new_v4().to_string();
    let cases: Vec<(&str, String, Option<Value>)> = vec![
        ("GET", "/admin/orders".into(), None),
        ("PATCH", format!("/admin/orders/{id}"), Some(json!({ "status": "placed" }))),
        ("GET", "/admin/wallets".into(), None),
        ("POST", "/admin/wallets/grants".into(), Some(json!({ "user_id": u, "points": 10, "reason": "x" }))),
        ("PATCH", format!("/admin/wallets/{id}"), Some(json!({ "status": "frozen", "reason": "x" }))),
    ];
    for (m, uri, body) in cases {
        let (s, _, b) = call(&pool, m, &uri, &[("cookie", format!("ws_session={t}")), ("idempotency-key", key.clone())], body.clone()).await;
        assert_eq!((s.as_u16(), b["code"].as_str()), (403, Some("STAFF_ONLY")), "{m} {uri}: {b}");
        assert_eq!(call(&pool, m, &uri, &[], body).await.0, StatusCode::UNAUTHORIZED, "{m} {uri}");
    }
    assert_eq!(balance(&pool, u).await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_order_unknown_id_is_404(pool: PgPool) {
    let (_, s) = user(&pool, "營運", true).await;
    let (st, _, _) = call(&pool, "PATCH", &format!("/admin/orders/{}", Uuid::new_v4()), &ck(&s), Some(json!({ "status": "placed", "merchant_order_id": "x", "amount": 1 }))).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

// ================= 營運：錢包 =================
#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_wallet_grant_adjust_idempotent_freeze(pool: PgPool) {
    let (sid, s) = user(&pool, "營運", true).await;
    let (u, _) = user(&pool, "阿明", false).await;
    let g = |pts: i64| json!({ "user_id": u, "points": pts, "reason": "活動贈點" });

    // 缺 Idempotency-Key
    let (st, _, b) = call(&pool, "POST", "/admin/wallets/grants", &ck(&s), Some(g(500))).await;
    assert_eq!((st.as_u16(), b["code"].as_str()), (400, Some("IDEMPOTENCY_KEY_REQUIRED")), "{b}");

    let k = Uuid::new_v4().to_string();
    let hdr = [("cookie", format!("ws_session={s}")), ("idempotency-key", k.clone())];
    let (st, _, b) = call(&pool, "POST", "/admin/wallets/grants", &hdr, Some(g(500))).await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    assert_eq!((b["wallet"]["balance"].as_i64(), b["wallet"]["status"].as_str(), b["entry"]["entry_type"].as_str(), b["entry"]["delta"].as_i64(), b["entry"]["note"].as_str()),
               (Some(500), Some("active"), Some("grant"), Some(500), Some("活動贈點")), "{b}");
    assert!(b["wallet"]["id"].is_string() && b["entry"]["balance_after"] == 500);
    let wallet_id = b["wallet"]["id"].as_str().unwrap().to_string();
    // 重放：同 key 同內容 → 同回應 + Idempotency-Replayed，不重複入帳
    let (st, h, b2) = call(&pool, "POST", "/admin/wallets/grants", &hdr, Some(g(500))).await;
    assert_eq!((st, h.get("idempotency-replayed").map(|v| v.to_str().unwrap())), (StatusCode::CREATED, Some("true")));
    assert_eq!(b2, b);
    assert_eq!(balance(&pool, u).await, 500);
    // 同 key 不同內容 → 409
    let (st, _, b3) = call(&pool, "POST", "/admin/wallets/grants", &hdr, Some(g(900))).await;
    assert_eq!((st.as_u16(), b3["code"].as_str()), (409, Some("IDEMPOTENCY_CONFLICT")), "{b3}");

    // 負數 = adjustment；扣到負數 → 409 INSUFFICIENT_POINTS（附 balance），餘額不動
    let (st, _, b) = call(&pool, "POST", "/admin/wallets/grants", &idem(&s), Some(g(-200))).await;
    assert_eq!((st, b["entry"]["entry_type"].as_str(), b["wallet"]["balance"].as_i64()), (StatusCode::CREATED, Some("adjustment"), Some(300)), "{b}");
    let (st, _, b) = call(&pool, "POST", "/admin/wallets/grants", &idem(&s), Some(g(-301))).await;
    assert_eq!((st.as_u16(), b["code"].as_str(), b["balance"].as_i64()), (409, Some("INSUFFICIENT_POINTS"), Some(300)), "{b}");
    assert_eq!(balance(&pool, u).await, 300);

    // 驗證
    for bad in [json!({ "user_id": u, "points": 0, "reason": "x" }), json!({ "user_id": u, "points": 1_000_001, "reason": "x" }),
                json!({ "user_id": u, "points": 1.5, "reason": "x" }), json!({ "user_id": u, "points": 5 }), json!({ "user_id": u, "points": 5, "reason": "" }),
                json!({ "points": 5, "reason": "x" })] {
        let (st, _, b) = call(&pool, "POST", "/admin/wallets/grants", &idem(&s), Some(bad)).await;
        assert_eq!((st.as_u16(), b["code"].as_str()), (422, Some("VALIDATION_FAILED")), "{b}");
    }
    let (st, _, _) = call(&pool, "POST", "/admin/wallets/grants", &idem(&s), Some(json!({ "user_id": Uuid::new_v4(), "points": 5, "reason": "x" }))).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // ledger 帶 actor / note；audit
    let (actor, note): (Option<Uuid>, Option<String>) = sqlx::query_as("SELECT actor_id, note FROM point_ledger WHERE entry_type = 'grant'").fetch_one(&pool).await.unwrap();
    assert_eq!((actor, note.as_deref()), (Some(sid), Some("活動贈點")));
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_logs WHERE action = 'wallet.grant' AND actor_id = $1").bind(sid).fetch_one(&pool).await.unwrap(), 2);

    // 搜尋
    let (st, _, l) = call(&pool, "GET", "/admin/wallets?q=%E9%98%BF%E6%98%8E", &ck(&s), None).await; // 阿明
    assert_eq!(st, StatusCode::OK, "{l}");
    assert_eq!((l["data"].as_array().unwrap().len(), l["data"][0]["wallet"]["balance"].as_i64(), l["data"][0]["user"]["id"].as_str()), (1, Some(300), Some(u.to_string().as_str())), "{l}");

    // 凍結 / 解凍
    let (st, _, b) = call(&pool, "PATCH", &format!("/admin/wallets/{wallet_id}"), &ck(&s), Some(json!({ "status": "frozen", "reason": "疑似盜用" }))).await;
    assert_eq!((st, b["status"].as_str()), (StatusCode::OK, Some("frozen")), "{b}");
    assert_eq!(call(&pool, "PATCH", &format!("/admin/wallets/{wallet_id}"), &ck(&s), Some(json!({ "status": "frozen" }))).await.0.as_u16(), 422);
    assert_eq!(call(&pool, "PATCH", &format!("/admin/wallets/{wallet_id}"), &ck(&s), Some(json!({ "status": "banned", "reason": "x" }))).await.0.as_u16(), 422);
    assert_eq!(call(&pool, "PATCH", &format!("/admin/wallets/{}", Uuid::new_v4()), &ck(&s), Some(json!({ "status": "active", "reason": "x" }))).await.0, StatusCode::NOT_FOUND);
    let (st, _, b) = call(&pool, "PATCH", &format!("/admin/wallets/{wallet_id}"), &ck(&s), Some(json!({ "status": "active", "reason": "已查證" }))).await;
    assert_eq!((st, b["status"].as_str()), (StatusCode::OK, Some("active")), "{b}");
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_logs WHERE action = 'wallet.set_status'").fetch_one(&pool).await.unwrap(), 2);
    assert_reconciled(&pool).await;
}

// ================= 刪除品項 / 封存 / 帳號 =================
#[sqlx::test(migrations = "../../db/migrations")]
async fn item_delete_force_releases_pledges(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (a, _) = user(&pool, "阿明", false).await;
    let (b_, _) = user(&pool, "小華", false).await;
    let (wid, _) = mk_list(&pool, &t, json!({})).await;
    put_addr(&pool, &t, wid).await;
    let item = cf_item(&pool, &t, wid, 1000).await;
    pledge(&pool, item, a, 300, false).await;
    pledge(&pool, item, b_, 200, true).await;
    assert_eq!((balance(&pool, a).await, balance(&pool, b_).await), (0, 0));

    let (st, _, b) = call(&pool, "DELETE", &format!("/items/{item}"), &ck(&t), None).await;
    assert_eq!((st.as_u16(), b["code"].as_str()), (409, Some("ITEM_HAS_CLAIMS")), "{b}");
    let (st, _, _) = call(&pool, "DELETE", &format!("/items/{item}?force=true"), &ck(&t), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!((balance(&pool, a).await, balance(&pool, b_).await), (300, 200));
    let (n, ps): (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM contributions WHERE item_id = $1 AND status = 'released'), pledged_points FROM wishlist_items WHERE id = $1").bind(item).fetch_one(&pool).await.unwrap();
    assert_eq!((n, ps), (2, 0));
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM notifications WHERE kind = 'item.removed'").fetch_one(&pool).await.unwrap(), 2);
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM wishlist_items WHERE id = $1 AND deleted_at IS NOT NULL").bind(item).fetch_one(&pool).await.unwrap(), 1);

    // 已達標（captured）→ 即使 force 也不能刪
    let done = cf_item(&pool, &t, wid, 100).await;
    pledge(&pool, done, a, 100, false).await;
    for q in ["", "?force=true"] {
        let (st, _, b) = call(&pool, "DELETE", &format!("/items/{done}{q}"), &ck(&t), None).await;
        assert_eq!((st.as_u16(), b["code"].as_str()), (409, Some("ITEM_HAS_CLAIMS")), "{b}");
    }
    assert_eq!(balance(&pool, a).await, 300);
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn archiving_wishlist_releases_pledges(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (a, _) = user(&pool, "阿明", false).await;
    let (wid, _) = mk_list(&pool, &t, json!({})).await;
    put_addr(&pool, &t, wid).await;
    let item = cf_item(&pool, &t, wid, 1000).await;
    pledge(&pool, item, a, 300, false).await;
    let (st, _, _) = call(&pool, "DELETE", &format!("/wishlists/{wid}"), &ck(&t), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(balance(&pool, a).await, 300);
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT pledged_points FROM wishlist_items WHERE id = $1").bind(item).fetch_one(&pool).await.unwrap(), 0);
    assert_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_delete_blocked_by_points(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (a, ta) = user(&pool, "阿明", false).await;
    let (b_, tb) = user(&pool, "小華", false).await;
    let (c, tc) = user(&pool, "路人C", false).await;
    let (wid, _) = mk_list(&pool, &t, json!({})).await;
    put_addr(&pool, &t, wid).await;
    let item = cf_item(&pool, &t, wid, 1000).await;
    let small = cf_item(&pool, &t, wid, 100).await;
    pledge(&pool, item, a, 300, false).await; // a：有進行中的認捐
    grant(&pool, b_, 50).await; // b：只有餘額
    pledge(&pool, small, c, 100, false).await; // c：點數已全部花出（captured），餘額 0

    for tok in [&ta, &tb] {
        let (st, _, b) = call(&pool, "DELETE", "/me", &ck(tok), Some(json!({ "confirm": "DELETE" }))).await;
        assert_eq!((st.as_u16(), b["code"].as_str()), (409, Some("ACCOUNT_HAS_POINTS")), "{b}");
    }
    // 被擋下時什麼都沒變
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users WHERE id = ANY($1) AND deleted_at IS NULL").bind(vec![a, b_]).fetch_one(&pool).await.unwrap(), 2);

    // 匯出含錢包 / 流水 / 認捐
    let (st, _, e) = call(&pool, "GET", "/me/export", &ck(&tc), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!((e["wallet"]["balance"].as_i64(), e["ledger"].as_array().unwrap().len(), e["contributions"].as_array().unwrap().len()), (Some(0), 2, 1), "{e}");

    // c 只剩 captured 認捐、餘額 0 → 可刪除，donor_name 去識別
    let (st, _, b) = call(&pool, "DELETE", "/me", &ck(&tc), Some(json!({ "confirm": "DELETE" }))).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(sqlx::query_scalar::<_, String>("SELECT donor_name FROM contributions WHERE user_id = $1").bind(c).fetch_one(&pool).await.unwrap(), "已刪除的使用者");
    // 擁有者刪帳號：別人 pledged 在其清單上的點數退回
    let (st, _, b) = call(&pool, "DELETE", "/me", &ck(&t), Some(json!({ "confirm": "DELETE" }))).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(balance(&pool, a).await, 300);
    assert_reconciled(&pool).await;
}

// 分頁多查的那一筆只用來判斷 next_cursor，不可被解密或寫入 order.view_address
#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_orders_audit_only_returned_rows(pool: PgPool) {
    let (_, t) = user(&pool, "媽媽", false).await;
    let (sid, s) = user(&pool, "營運", true).await;
    let (a, _) = user(&pool, "阿明", false).await;
    let (wid, _) = mk_list(&pool, &t, json!({})).await;
    put_addr(&pool, &t, wid).await;
    for _ in 0..3 { let it = cf_item(&pool, &t, wid, 100).await; pledge(&pool, it, a, 100, false).await; }
    let audits = || async { sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_logs WHERE action = 'order.view_address' AND actor_id = $1").bind(sid).fetch_one(&pool).await.unwrap() };

    let (st, _, p1) = call(&pool, "GET", "/admin/orders?limit=2", &ck(&s), None).await;
    assert_eq!(st, StatusCode::OK, "{p1}");
    assert_eq!(p1["data"].as_array().unwrap().len(), 2);
    assert_eq!(p1["next_cursor"], p1["data"][1]["id"]);
    assert_eq!(audits().await, 2, "只記實際回傳的筆數");

    let (_, _, p2) = call(&pool, "GET", &format!("/admin/orders?limit=2&cursor={}", p1["next_cursor"].as_str().unwrap()), &ck(&s), None).await;
    assert_eq!(p2["data"].as_array().unwrap().len(), 1);
    assert!(p2["next_cursor"].is_null());
    assert_eq!(audits().await, 3);
}
