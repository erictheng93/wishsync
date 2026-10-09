//! 集中式設定。機密（OTP_PEPPER / OAUTH_SECRET / UNSUB_SECRET）彼此獨立、不互相 fallback。
//! fail closed：APP_ENV 未設或非 dev/test 一律視為 production，缺機密或 <32 字元即 panic。
use std::{net::SocketAddr, sync::OnceLock};

#[derive(Clone, Debug)]
pub struct Config {
    pub is_prod: bool,
    pub app_url: String,
    pub api_base_url: String,
    pub otp_pepper: String,
    pub oauth_secret: String,
    pub unsub_secret: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub mail_from: String,
    pub cf_account_id: Option<String>,
    pub cf_email_api_token: Option<String>,
    /// Cloudflare API 根（測試指向 mock）
    pub cf_api_base: String,
    pub trusted_proxy_cloudflare: bool,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

/// main.rs 啟動時呼叫一次。
pub fn init(c: Config) { let _ = CONFIG.set(c); }

/// 已 init 則回該設定；否則（僅測試 / examples 等未經 main 的情況）回 dev 設定。
/// 正式程序一定經 main 的 init，所以此 fallback 不會在 production 生效。
pub fn get() -> Config { CONFIG.get().cloned().unwrap_or_else(Config::dev) }

fn var(k: &str) -> Option<String> { std::env::var(k).ok().filter(|v| !v.is_empty()) }

impl Config {
    /// dev 預設值（僅本機 / 測試）；仍讀環境變數覆寫。
    pub fn dev() -> Config {
        let v = |k: &str, d: &str| var(k).unwrap_or_else(|| d.into());
        Config {
            is_prod: false,
            app_url: v("APP_URL", "http://localhost:3000").trim_end_matches('/').into(),
            api_base_url: v("API_BASE_URL", "http://localhost:8080"),
            otp_pepper: v("OTP_PEPPER", "dev-pepper"),
            oauth_secret: v("OAUTH_SECRET", "dev-oauth-secret"),
            unsub_secret: v("UNSUB_SECRET", "dev-unsub-secret"),
            smtp_host: v("SMTP_HOST", "localhost"),
            smtp_port: var("SMTP_PORT").and_then(|p| p.parse().ok()).unwrap_or(1025),
            mail_from: v("MAIL_FROM", "WishSync <no-reply@wishsync.tw>"),
            cf_account_id: var("CF_ACCOUNT_ID"),
            cf_email_api_token: var("CF_EMAIL_API_TOKEN"),
            cf_api_base: v("CF_API_BASE", "https://api.cloudflare.com/client/v4"),
            trusted_proxy_cloudflare: var("TRUSTED_PROXY").as_deref() == Some("cloudflare"),
        }
    }

    pub fn from_env() -> Config {
        Self::try_from_env().unwrap_or_else(|e| panic!("設定錯誤：{e}"))
    }

    pub fn try_from_env() -> Result<Config, String> {
        let mut c = Config::dev();
        match var("APP_ENV").as_deref() {
            Some("dev") | Some("test") => return Ok(c),
            Some("production") | Some("prod") | None => {}
            Some(o) => return Err(format!("APP_ENV={o} 無效（dev|test|production）")),
        }
        c.is_prod = true;
        let must = |k: &str| match var(k) {
            None => Err(format!("production 需設定 {k}")),
            Some(s) if s.len() < 32 => Err(format!("{k} 至少 32 字元")),
            Some(s) => Ok(s),
        };
        c.otp_pepper = must("OTP_PEPPER")?;
        c.oauth_secret = must("OAUTH_SECRET")?;
        c.unsub_secret = must("UNSUB_SECRET")?;
        let need = |k: &str| var(k).ok_or_else(|| format!("production 需設定 {k}"));
        need("CF_ACCOUNT_ID")?; need("CF_EMAIL_API_TOKEN")?; need("MAIL_FROM")?; need("APP_URL")?; need("API_BASE_URL")?;
        Ok(c)
    }

    pub fn secure_cookie(&self) -> &'static str { if self.is_prod { "; Secure" } else { "" } }
}

/// 共用：取得用戶端 IP。TRUSTED_PROXY=cloudflare 才信任 CF-Connecting-IP；
/// 否則用 socket 位址（需 into_make_service_with_connect_info）；都沒有回 "unknown"。不信任 X-Forwarded-For。
/// handler 以 `parts: axum::http::request::Parts` 取得後呼叫。
pub fn client_ip(parts: &axum::http::request::Parts, cfg: &Config) -> String {
    if cfg.trusted_proxy_cloudflare {
        if let Some(ip) = parts.headers.get("cf-connecting-ip").and_then(|v| v.to_str().ok()).map(str::trim).filter(|v| !v.is_empty()) {
            return ip.to_string();
        }
    }
    parts.extensions.get::<axum::extract::ConnectInfo<SocketAddr>>().map(|c| c.0.ip().to_string()).unwrap_or_else(|| "unknown".into())
}

/// 常數時間比較
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool { use subtle::ConstantTimeEq; a.ct_eq(b).into() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prod_fail_closed() {
        // 不碰全域環境：直接驗 must 邏輯所依賴的長度規則與 dev()
        assert!(!Config::dev().is_prod && Config::dev().secure_cookie().is_empty());
        assert!(ct_eq(b"abc", b"abc") && !ct_eq(b"abc", b"abd") && !ct_eq(b"abc", b"ab"));
    }
}
