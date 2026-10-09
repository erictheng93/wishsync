//! Config fail-closed、Cloudflare Email mock、client_ip。（不需資料庫）
use axum::{extract::ConnectInfo, http::{HeaderMap, Request}, routing::post, Json, Router};
use serde_json::{json, Value};
use std::{net::SocketAddr, sync::{Arc, Mutex}};
use wishsync_api::config::{client_ip, Config};

#[test]
fn production_fails_closed() {
    let all = ["APP_ENV", "OTP_PEPPER", "OAUTH_SECRET", "UNSUB_SECRET", "CF_ACCOUNT_ID", "CF_EMAIL_API_TOKEN", "MAIL_FROM", "APP_URL", "API_BASE_URL"];
    for k in all { std::env::remove_var(k); }
    // 未設 APP_ENV => production => 缺 OTP_PEPPER
    assert!(Config::try_from_env().unwrap_err().contains("OTP_PEPPER"));
    let long = "x".repeat(32);
    std::env::set_var("OTP_PEPPER", "short");
    assert!(Config::try_from_env().unwrap_err().contains("OTP_PEPPER"));
    std::env::set_var("OTP_PEPPER", &long);
    assert!(Config::try_from_env().unwrap_err().contains("OAUTH_SECRET")); // 不 fallback 到 pepper
    std::env::set_var("OAUTH_SECRET", &long);
    assert!(Config::try_from_env().unwrap_err().contains("UNSUB_SECRET")); // 不 fallback 到 oauth
    std::env::set_var("UNSUB_SECRET", &long);
    assert!(Config::try_from_env().unwrap_err().contains("CF_ACCOUNT_ID"));
    for (k, v) in [("CF_ACCOUNT_ID", "a"), ("CF_EMAIL_API_TOKEN", "t"), ("MAIL_FROM", "a@b.tw"), ("APP_URL", "https://x.tw"), ("API_BASE_URL", "https://api.x.tw")] { std::env::set_var(k, v); }
    let c = Config::try_from_env().unwrap();
    assert!(c.is_prod && c.secure_cookie() == "; Secure" && !c.trusted_proxy_cloudflare);
    std::env::set_var("APP_ENV", "dev");
    let c = Config::try_from_env().unwrap();
    assert!(!c.is_prod && c.secure_cookie().is_empty());
    for k in all { std::env::remove_var(k); }
}

#[test]
fn client_ip_trust() {
    let mk = |cf: bool, with_ci: bool| {
        let mut r = Request::builder().header("cf-connecting-ip", "1.1.1.1").header("x-forwarded-for", "2.2.2.2").body(()).unwrap();
        if with_ci { r.extensions_mut().insert(ConnectInfo("9.9.9.9:1234".parse::<SocketAddr>().unwrap())); }
        let mut c = Config::dev();
        c.trusted_proxy_cloudflare = cf;
        (r.into_parts().0, c)
    };
    let (p, c) = mk(false, true); assert_eq!(client_ip(&p, &c), "9.9.9.9"); // 偽造標頭無效
    let (p, c) = mk(false, false); assert_eq!(client_ip(&p, &c), "unknown");
    let (p, c) = mk(true, true); assert_eq!(client_ip(&p, &c), "1.1.1.1");
    let _: HeaderMap = p.headers;
}

#[tokio::test]
async fn cloudflare_email_request_format() {
    let seen: Arc<Mutex<Vec<(String, String, Value)>>> = Default::default();
    let s2 = seen.clone();
    let mock = Router::new().route("/accounts/acct1/email/sending/send", post(move |h: HeaderMap, Json(b): Json<Value>| {
        let s = s2.clone();
        async move {
            let auth = h["authorization"].to_str().unwrap().to_string();
            let fail = b["to"] == "bad@example.com";
            s.lock().unwrap().push((auth, h["content-type"].to_str().unwrap().to_string(), b));
            if fail { Json(json!({"success": false, "errors": [{"code": 1, "message": "nope"}], "result": null})) }
            else { Json(json!({"success": true, "errors": [], "result": {"delivered": ["x"], "permanent_bounces": [], "queued": []}})) }
        }
    }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, mock).await.unwrap() });
    let mut c = Config::dev();
    c.cf_account_id = Some("acct1".into());
    c.cf_email_api_token = Some("secret-token".into());
    c.cf_api_base = format!("http://{addr}");
    c.mail_from = "WishSync <no-reply@wishsync.tw>".into();
    wishsync_api::notify::send_mail(&c, "u@example.com", "主旨", "內文").await.unwrap();
    let e = wishsync_api::notify::send_mail(&c, "bad@example.com", "s", "b").await.unwrap_err();
    assert!(!e.contains("secret-token"));
    let v = seen.lock().unwrap();
    assert_eq!(v[0].0, "Bearer secret-token");
    assert!(v[0].1.starts_with("application/json"));
    assert_eq!(v[0].2, json!({"from": "WishSync <no-reply@wishsync.tw>", "to": "u@example.com", "subject": "主旨", "text": "內文"}));
}
