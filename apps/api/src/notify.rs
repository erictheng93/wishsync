//! Email 通知 outbox：認領模組呼叫 `enqueue_claim`（寫 notifications，docs/04 4.10），
//! 背景 worker `spawn_worker` 輪詢到期通知，經 SMTP（預設 Mailpit localhost:1025）寄出。
//! 內容刻意不含品項與認領者，只寫「有 N 件新認領」，所以驚喜模式下天然不洩漏。
use crate::config::Config;
use lettre::{message::header::ContentType, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

/// 認領成功的同一交易內呼叫：第一筆即時 claim.created；之後合併為每日台北 09:00 的 claim.digest（count 累加）。
/// 建立者關閉 email_claims 或已刪除則不寫。
pub async fn enqueue_claim<'e, E: sqlx::PgExecutor<'e>>(ex: E, wishlist_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO notifications (user_id, channel, kind, payload, scheduled_at)
         SELECT u.id, 'email',
                CASE WHEN f.first THEN 'claim.created' ELSE 'claim.digest' END,
                jsonb_build_object('wishlist_id', w.id, 'count', 1),
                CASE WHEN f.first THEN now() ELSE
                  ((date_trunc('day', now() AT TIME ZONE 'Asia/Taipei') + interval '9 hours'
                    + CASE WHEN (now() AT TIME ZONE 'Asia/Taipei')::time >= time '09:00' THEN interval '1 day' ELSE interval '0' END)
                   AT TIME ZONE 'Asia/Taipei') END
           FROM wishlists w JOIN users u ON u.id = w.owner_id,
                LATERAL (SELECT NOT EXISTS (SELECT 1 FROM notifications n WHERE n.user_id = u.id AND n.kind = 'claim.created'
                                              AND n.payload->>'wishlist_id' = w.id::text) AS first) f
          WHERE w.id = $1 AND u.deleted_at IS NULL AND COALESCE((u.notification_prefs->>'email_claims')::boolean, true)
         ON CONFLICT (user_id, kind, (payload->>'wishlist_id')) WHERE kind = 'claim.digest' AND status = 'pending'
         DO UPDATE SET payload = jsonb_set(notifications.payload, '{count}', to_jsonb((notifications.payload->>'count')::int + 1))")
        .bind(wishlist_id).execute(ex).await?;
    Ok(())
}

/// 眾籌（P2-A）：通知某品項的捐贈者（status in `statuses` 的認捐，每人一封）。
/// kind：crowdfund.funded / order.shipped / order.delivered / order.failed / funding.expired。
/// payload 帶 wishlist_id 與 item_id；捐贈者本來就知道自己捐了哪個品項，故可寫品項名稱。
pub async fn enqueue_donors<'e, E: sqlx::PgExecutor<'e>>(ex: E, item_id: Uuid, kind: &str, statuses: &[&str]) -> Result<(), sqlx::Error> {
    let statuses: Vec<String> = statuses.iter().map(|s| s.to_string()).collect();
    sqlx::query(
        "INSERT INTO notifications (user_id, channel, kind, payload)
         SELECT DISTINCT c.user_id, 'email'::notification_channel, $2, jsonb_build_object('wishlist_id', c.wishlist_id, 'item_id', c.item_id)
           FROM contributions c JOIN users u ON u.id = c.user_id
          WHERE c.item_id = $1 AND c.status::text = ANY($3) AND u.deleted_at IS NULL")
        .bind(item_id).bind(kind).bind(&statuses).execute(ex).await?;
    Ok(())
}

/// 唯一寄信點。設了 CF_ACCOUNT_ID + CF_EMAIL_API_TOKEN 就走 Cloudflare Email Service REST API
/// （production 啟動時強制要求）；否則 dev 走 SMTP → Mailpit。錯誤字串不含 token。
pub async fn send_mail(cfg: &Config, to: &str, subject: &str, body: &str) -> Result<(), String> {
    if let (Some(acct), Some(tok)) = (&cfg.cf_account_id, &cfg.cf_email_api_token) {
        let url = format!("{}/accounts/{acct}/email/sending/send", cfg.cf_api_base.trim_end_matches('/'));
        let r = reqwest::Client::builder().timeout(Duration::from_secs(15)).build().map_err(|e| e.without_url().to_string())?
            .post(url).bearer_auth(tok)
            .json(&serde_json::json!({ "from": cfg.mail_from, "to": to, "subject": subject, "text": body }))
            .send().await.map_err(|e| e.without_url().to_string())?;
        let status = r.status();
        let v: Value = r.json().await.unwrap_or(Value::Null);
        // 2xx 且 success != false 才算成功；永久退信也視為失敗
        if status.is_success() && v["success"] != false && v["result"]["permanent_bounces"].as_array().is_none_or(|a| a.is_empty()) { return Ok(()); }
        return Err(format!("cloudflare email {status}: {}", v["errors"]));
    }
    let msg = Message::builder().from(cfg.mail_from.parse().map_err(|e| format!("{e}"))?)
        .to(to.parse().map_err(|e| format!("{e}"))?)
        .subject(subject).header(ContentType::TEXT_PLAIN).body(body.to_string()).map_err(|e| e.to_string())?;
    AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(cfg.smtp_host.clone()).port(cfg.smtp_port).build().send(msg).await.map(|_| ()).map_err(|e| e.to_string())
}

pub fn spawn_worker(pool: PgPool) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = tick(&pool).await { tracing::warn!(error = %e, "notify worker"); }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

const MAX_ATTEMPTS: i16 = 3;

/// 處理一批到期通知；回傳處理筆數。領取時 attempts+1 並把 scheduled_at 推後 1 分鐘當租約（失敗即自動重試）。
pub async fn tick(pool: &PgPool) -> Result<usize, sqlx::Error> {
    let rows: Vec<(Uuid, Option<Uuid>, Option<Uuid>, String, String, Value, i16)> = sqlx::query_as(
        "UPDATE notifications SET attempts = attempts + 1, scheduled_at = now() + interval '1 minute'
          WHERE id IN (SELECT id FROM notifications WHERE status = 'pending' AND scheduled_at <= now()
                        ORDER BY scheduled_at LIMIT 20 FOR UPDATE SKIP LOCKED)
         RETURNING id, user_id, guest_id, channel::text, kind, payload, attempts")
        .fetch_all(pool).await?;
    if rows.is_empty() { return Ok(0); }
    let cfg = crate::config::get();
    let n = rows.len();
    for (id, uid, gid, channel, kind, payload, attempts) in rows {
        let to: Option<String> = if channel != "email" { None } else if let Some(u) = uid {
            sqlx::query_scalar("SELECT email FROM users WHERE id = $1 AND deleted_at IS NULL").bind(u).fetch_optional(pool).await?.flatten()
        } else if let Some(g) = gid {
            sqlx::query_scalar("SELECT email FROM guests WHERE id = $1 AND deleted_at IS NULL").bind(g).fetch_optional(pool).await?.flatten()
        } else { None };
        let Some(to) = to else {
            sqlx::query("UPDATE notifications SET status = 'cancelled', last_error = 'no email recipient' WHERE id = $1").bind(id).execute(pool).await?;
            continue;
        };
        let title: String = match payload.get("wishlist_id").and_then(Value::as_str).and_then(|s| s.parse::<Uuid>().ok()) {
            Some(w) => sqlx::query_scalar("SELECT title FROM wishlists WHERE id = $1").bind(w).fetch_optional(pool).await?.unwrap_or_default(),
            None => String::new(),
        };
        // 寄信時才產生：退訂連結；訪客確認信的一次性恢復權杖（每次寄信換發，30 天）
        let mut payload = payload;
        // 眾籌通知：寄信時才查品項名稱（品項可能已被刪除，仍保留列）
        if let Some(it) = payload.get("item_id").and_then(Value::as_str).and_then(|s| s.parse::<Uuid>().ok()) {
            let t: Option<String> = sqlx::query_scalar("SELECT title FROM wishlist_items WHERE id = $1").bind(it).fetch_optional(pool).await?;
            if let Some(t) = t { payload["item_title"] = t.into(); }
        }
        let api = cfg.api_base_url.clone();
        let sub = match (uid, gid) { (Some(u), _) => Some(('u', u)), (_, Some(g)) => Some(('g', g)), _ => None };
        if let Some((k, i)) = sub {
            payload["unsubscribe_url"] = format!("{}/api/v1/unsubscribe?token={}", api.trim_end_matches('/'), crate::account::unsub_token(k, i)).into();
        }
        if let (Some(g), "claim.confirmation") = (gid, kind.as_str()) {
            let (tok, h) = crate::guest::new_token();
            sqlx::query("INSERT INTO guest_recovery_tokens (guest_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '30 days')")
                .bind(g).bind(h).execute(pool).await?;
            payload["recovery_token"] = tok.into();
        }
        let (subject, body) = render(&kind, &title, &payload);
        if to.parse::<lettre::Address>().is_err() {
            sqlx::query("UPDATE notifications SET status = 'failed', last_error = 'bad address' WHERE id = $1").bind(id).execute(pool).await?;
            continue;
        }
        let sent = send_mail(&cfg, &to, &subject, &body).await;
        match sent {
            Ok(()) => { sqlx::query("UPDATE notifications SET status = 'sent', sent_at = now(), last_error = NULL WHERE id = $1").bind(id).execute(pool).await?; }
            Err(e) => {
                let st = if attempts >= MAX_ATTEMPTS { "failed" } else { "pending" };
                sqlx::query("UPDATE notifications SET status = $2::notification_status, last_error = $3 WHERE id = $1").bind(id).bind(st).bind(e).execute(pool).await?;
            }
        }
    }
    Ok(n)
}

/// 純函式：(主旨, 內文)。刻意只用數量，不帶品項 / 認領者。
pub fn render(kind: &str, title: &str, payload: &Value) -> (String, String) {
    let n = payload.get("count").and_then(Value::as_i64).unwrap_or(1);
    let base = crate::config::get().app_url;
    let manage = match payload.get("recovery_token").and_then(Value::as_str) { Some(t) => format!("{base}/me/claims#r={t}"), None => format!("{base}/me/claims") };
    let item = payload.get("item_title").and_then(Value::as_str).unwrap_or("品項");
    let wallet = format!("{base}/me/wallet");
    let (subject, body) = match kind {
        "crowdfund.funded" => (format!("【WishSync】《{item}》集資達標了"), format!("你支持的「{item}」（清單《{title}》）已集資達標，營運團隊會開始代購。\n查看我的點數與捐贈：{wallet}\n")),
        "order.shipped" => (format!("【WishSync】《{item}》已出貨"), format!("你支持的「{item}」（清單《{title}》）已出貨。\n查看我的點數與捐贈：{wallet}\n")),
        "order.delivered" => (format!("【WishSync】《{item}》已送達"), format!("你支持的「{item}」（清單《{title}》）已送達受贈者，謝謝你的心意。\n查看我的點數與捐贈：{wallet}\n")),
        "order.failed" => (format!("【WishSync】《{item}》代購未成功，點數已退回"), format!("你支持的「{item}」（清單《{title}》）代購未成功，你捐出的點數已全數退回錢包。\n查看我的點數：{wallet}\n")),
        "funding.expired" => (format!("【WishSync】《{item}》募集期限已到"), format!("「{item}」（清單《{title}》）在期限內未達標。你的點數會再保留 7 天：期間可到錢包撤回，或轉捐給同一份清單的其他品項；逾期未處理會自動退回錢包。\n處理我的捐贈：{wallet}\n")),
        "item.removed" => (format!("【WishSync】《{item}》已被移除，點數已退回"), format!("清單《{title}》的建立者移除了「{item}」（或封存了清單），你捐出的點數已退回錢包。\n查看我的點數：{wallet}\n")),
        "claim.digest" => (format!("【WishSync】《{title}》今日有 {n} 件新認領"), format!("你的清單《{title}》今日彙整：有 {n} 件新認領。\n登入查看：{base}/dashboard\n")),
        "claim.created" => (format!("【WishSync】《{title}》有 {n} 件新認領"), format!("你的清單《{title}》有 {n} 件新認領。\n登入查看：{base}/dashboard\n")),
        "claim.confirmation" => ("【WishSync】認領已確認".into(), format!("你的認領已記錄，謝謝你的心意。\n管理我的認領：{manage}\n")),
        "claim.item_removed" => ("【WishSync】你認領的品項已被移除".into(), format!("清單《{title}》的建立者移除了你認領的品項，這筆認領已取消。\n查看其他品項：{manage}\n")),
        "event.reminder" => (format!("【WishSync】《{title}》活動日快到了"), format!("你的清單《{title}》的活動日即將到來。\n{base}/dashboard\n")),
        _ => ("【WishSync】通知".into(), format!("{base}\n")),
    };
    match payload.get("unsubscribe_url").and_then(Value::as_str) {
        Some(u) => (subject, format!("{body}\n退訂此類通知：{u}\n")),
        None => (subject, body),
    }
}
