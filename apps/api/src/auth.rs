//! Auth：Email OTP、LINE OAuth2（code flow + PKCE）、登出、GET/PATCH /me。簽發 session（DB 只存 SHA-256(token)）。
use crate::{error::AppError, session::{hash_token, CurrentUser}, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{AppendHeaders, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use hmac::{Hmac, Mac};
use rand::{Rng, RngCore};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

const SESSION_DAYS: i64 = 30;
const OTP_TTL_SECS: i64 = 600;
const OTP_MAX_ATTEMPTS: i16 = 5;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/otp/request", post(otp_request))
        .route("/auth/otp/verify", post(otp_verify))
        .route("/auth/oauth/{provider}/start", get(oauth_start))
        .route("/auth/oauth/{provider}/callback", get(oauth_callback))
        .route("/auth/logout", post(logout))
        .merge(crate::auth_ext::routes())
        .route("/me", get(me).patch(patch_me))
}

pub(crate) fn env(k: &str, d: &str) -> String { std::env::var(k).unwrap_or_else(|_| d.to_string()) }
fn pepper() -> String { crate::config::get().otp_pepper }
pub(crate) fn app_url() -> String { crate::config::get().app_url }
pub(crate) fn secure() -> &'static str { crate::config::get().secure_cookie() }

pub fn code_hash(code: &str, pepper: &str) -> Vec<u8> { Sha256::digest(format!("{code}{pepper}").as_bytes()).to_vec() }

pub(crate) fn cookie(h: &HeaderMap, name: &str) -> Option<String> {
    h.get(header::COOKIE)?.to_str().ok()?.split(';').find_map(|c| c.trim().strip_prefix(name)?.strip_prefix('=')).map(String::from)
}
pub(crate) fn session_cookie(token: &str) -> String {
    format!("ws_session={token}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax{}", SESSION_DAYS * 86400, secure())
}

// ---------- session / identity ----------
pub(crate) async fn new_session(pool: &PgPool, user_id: Uuid, ua: Option<&str>) -> Result<String, AppError> {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    let token = B64.encode(b);
    sqlx::query("INSERT INTO sessions (user_id, token_hash, user_agent, expires_at) VALUES ($1,$2,$3, now() + make_interval(days => $4))")
        .bind(user_id).bind(hash_token(&token)).bind(ua).bind(SESSION_DAYS as i32).execute(pool).await?;
    Ok(token)
}

/// 以 (provider, uid) 找或建 user；回傳 (user_id, is_new)。
pub async fn upsert_identity(pool: &PgPool, provider: &str, uid: &str, email: Option<&str>, name: &str, avatar: Option<&str>) -> Result<(Uuid, bool), AppError> {
    let mut tx = pool.begin().await?;
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT i.user_id FROM auth_identities i JOIN users u ON u.id = i.user_id
         WHERE i.provider = $1::auth_provider AND i.provider_uid = $2 AND u.deleted_at IS NULL")
        .bind(provider).bind(uid).fetch_optional(&mut *tx).await?;
    let (uid_, is_new) = if let Some(id) = found { (id, false) } else {
        let existing: Option<Uuid> = match email {
            Some(e) => sqlx::query_scalar("SELECT id FROM users WHERE email = $1 AND deleted_at IS NULL").bind(e).fetch_optional(&mut *tx).await?,
            None => None,
        };
        let (id, new) = match existing {
            Some(id) => (id, false),
            None => {
                let dn: String = name.chars().take(50).collect();
                let dn = if dn.trim().is_empty() { "user".to_string() } else { dn };
                (sqlx::query_scalar("INSERT INTO users (display_name, email, avatar_key) VALUES ($1,$2,$3) RETURNING id")
                    .bind(dn).bind(email).bind(avatar).fetch_one(&mut *tx).await?, true)
            }
        };
        sqlx::query("INSERT INTO auth_identities (user_id, provider, provider_uid, email) VALUES ($1,$2::auth_provider,$3,$4)")
            .bind(id).bind(provider).bind(uid).bind(email).execute(&mut *tx).await?;
        (id, new)
    };
    tx.commit().await?;
    Ok((uid_, is_new))
}

// ---------- Email OTP ----------
#[derive(Deserialize)]
pub(crate) struct EmailReq { email: String }

pub(crate) fn norm_email(e: &str) -> Result<String, AppError> {
    let e = e.trim().to_lowercase();
    let ok = e.len() <= 254 && e.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.'))
        && !e.chars().any(|c| c.is_whitespace() || c.is_control());
    if ok { Ok(e) } else { Err(AppError::invalid("/email", "FORMAT", "Email 格式不正確")) }
}

pub(crate) fn rate_limited(secs: i64) -> AppError {
    AppError::Problem { status: 429, code: "RATE_LIMITED", detail: format!("請求過於頻繁，請 {secs} 秒後再試。"), errors: None, retry_after: Some(secs.max(1) as u64) }
}

async fn send_mail(to: String, code: String, subject: &'static str) {
    let r = crate::notify::send_mail(&crate::config::get(), &to, subject, &format!("您的驗證碼是 {code}，10 分鐘內有效。若非本人操作請忽略此信。")).await;
    if let Err(e) = r { tracing::error!(error = %e, "send otp mail failed"); }
}

// ponytail: 只做每 email 限流（查 otp_challenges）；每 IP 20/小時需前置層（Cloudflare）或 in-memory/Redis，之後再加。
/// 限流 + 寫入挑戰 + 寄信。login / register / reset 共用同一組限流（每 email 每小時 5 封、60 秒間隔）。
/// 60 秒間隔只看「尚未使用」的驗證碼：剛驗證成功（例如註冊完）立刻重設密碼是正常操作，不應被擋；沒有信箱存取權的人無法消耗驗證碼，所以不影響防濫用。
pub(crate) async fn issue_otp(pool: &PgPool, email: &str, purpose: &str, pw_hash: Option<&str>, name: Option<&str>) -> Result<(), AppError> {
    let (cnt, last_age): (i64, Option<i64>) = sqlx::query_as(
        "SELECT count(*), (min(extract(epoch FROM now() - created_at)) FILTER (WHERE consumed_at IS NULL))::bigint FROM otp_challenges
         WHERE email = $1 AND created_at > now() - interval '1 hour'").bind(email).fetch_one(pool).await?;
    if cnt >= 5 { return Err(rate_limited(600)); }
    if let Some(age) = last_age { if age < 60 { return Err(rate_limited(60 - age)); } }
    let code = format!("{:06}", rand::thread_rng().gen_range(0..1_000_000u32));
    sqlx::query("INSERT INTO otp_challenges (email, code_hash, expires_at, purpose, pending_password_hash, pending_display_name) VALUES ($1,$2, now() + make_interval(secs => $3),$4,$5,$6)")
        .bind(email).bind(code_hash(&code, &pepper())).bind(OTP_TTL_SECS as f64).bind(purpose).bind(pw_hash).bind(name).execute(pool).await?;
    let subject = match purpose { "register" => "WishSync 註冊驗證碼", "reset" => "WishSync 重設密碼驗證碼", _ => "WishSync 登入驗證碼" };
    tokio::spawn(send_mail(email.to_string(), code, subject)); // 寄信失敗不影響回應（避免帳號列舉與外部依賴）
    Ok(())
}

async fn otp_request(State(st): State<AppState>, Json(r): Json<EmailReq>) -> Result<(StatusCode, Json<Value>), AppError> {
    let email = norm_email(&r.email)?;
    issue_otp(&st.pool, &email, "login", None, None).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "expires_in": OTP_TTL_SECS, "resend_after": 60 }))))
}

#[derive(Deserialize)]
pub(crate) struct VerifyReq { email: String, code: String }

pub(crate) fn otp_invalid() -> AppError { AppError::problem(400, "OTP_INVALID", "驗證碼錯誤或已過期，請重新取得。") }

/// 驗證並消耗指定用途的 OTP；回傳 (pending_password_hash, pending_display_name)。
pub(crate) async fn consume_otp(pool: &PgPool, email: &str, code: &str, purpose: &str) -> Result<(Option<String>, Option<String>), AppError> {
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) { return Err(AppError::invalid("/code", "FORMAT", "驗證碼為 6 位數字")); }
    // 每 email 每小時最多 10 次驗證嘗試
    let tries: i64 = sqlx::query_scalar("SELECT coalesce(sum(attempts),0)::bigint FROM otp_challenges WHERE email=$1 AND created_at > now() - interval '1 hour'")
        .bind(email).fetch_one(pool).await?;
    if tries >= 10 { return Err(rate_limited(600)); }
    let ch: Option<(Uuid, Vec<u8>, i16, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, code_hash, attempts, pending_password_hash, pending_display_name FROM otp_challenges
         WHERE email=$1 AND purpose=$2 AND consumed_at IS NULL AND expires_at > now()
         ORDER BY created_at DESC LIMIT 1").bind(email).bind(purpose).fetch_optional(pool).await?;
    let (cid, hash, attempts, pw, name) = ch.ok_or_else(otp_invalid)?;
    if attempts >= OTP_MAX_ATTEMPTS { return Err(otp_invalid()); }
    if !crate::config::ct_eq(&hash, &code_hash(code, &pepper())) {
        sqlx::query("UPDATE otp_challenges SET attempts = attempts + 1 WHERE id=$1").bind(cid).execute(pool).await?;
        return Err(otp_invalid());
    }
    // 一次性：只有第一個成功 UPDATE 的請求能通過
    let used = sqlx::query("UPDATE otp_challenges SET consumed_at = now() WHERE id=$1 AND consumed_at IS NULL").bind(cid).execute(pool).await?;
    if used.rows_affected() != 1 { return Err(otp_invalid()); }
    Ok((pw, name))
}

/// 簽發 session 並回 {user, is_new_user} + Set-Cookie（OTP / 註冊 / 密碼登入共用）。
pub(crate) async fn session_response(pool: &PgPool, headers: &HeaderMap, uid: Uuid, is_new: bool) -> Result<Response, AppError> {
    let token = new_session(pool, uid, headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok())).await?;
    let u = user_json(pool, uid).await?;
    let body = json!({ "user": { "id": u["id"], "display_name": u["display_name"], "email": u["email"], "avatar_url": u["avatar_url"] }, "is_new_user": is_new });
    Ok(([(header::SET_COOKIE, session_cookie(&token))], Json(body)).into_response())
}

async fn otp_verify(State(st): State<AppState>, headers: HeaderMap, Json(r): Json<VerifyReq>) -> Result<Response, AppError> {
    let email = norm_email(&r.email)?;
    consume_otp(&st.pool, &email, &r.code, "login").await?;
    let name = email.split('@').next().unwrap_or("user");
    let (uid, is_new) = upsert_identity(&st.pool, "email", &email, Some(&email), name, None).await?;
    session_response(&st.pool, &headers, uid, is_new).await
}

// ---------- logout / me ----------
async fn logout(_u: CurrentUser, State(st): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    if let Some(t) = cookie(&headers, "ws_session") {
        sqlx::query("UPDATE sessions SET revoked_at = now() WHERE token_hash=$1 AND revoked_at IS NULL").bind(hash_token(&t)).execute(&st.pool).await?;
    }
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, format!("ws_session=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax{}", secure()))]).into_response())
}

pub(crate) async fn user_json(pool: &PgPool, id: Uuid) -> Result<Value, AppError> {
    let (dn, email, avatar, staff, prefs): (String, Option<String>, Option<String>, bool, Value) =
        sqlx::query_as("SELECT display_name, email, avatar_key, is_staff, notification_prefs FROM users WHERE id=$1 AND deleted_at IS NULL")
            .bind(id).fetch_optional(pool).await?.ok_or(AppError::Unauthorized)?;
    let idents: Vec<String> = sqlx::query_scalar("SELECT provider::text FROM auth_identities WHERE user_id=$1 ORDER BY created_at").bind(id).fetch_all(pool).await?;
    // avatar_key：LINE 頭像存完整 URL；其餘暫無公開網址規則
    let avatar_url = avatar.filter(|a| a.starts_with("http"));
    Ok(json!({ "id": id, "display_name": dn, "email": email, "avatar_url": avatar_url, "is_staff": staff,
               "notification_prefs": prefs, "identities": idents.iter().map(|p| json!({"provider": p})).collect::<Vec<_>>(),
               "orgs": [] })) // orgs 為 P2-D，MVP 恆空
}

async fn me(u: CurrentUser, State(st): State<AppState>) -> Result<Json<Value>, AppError> { Ok(Json(user_json(&st.pool, u.id).await?)) }

#[derive(Deserialize)]
struct PatchMe { display_name: Option<String>, notification_prefs: Option<Value> }

async fn patch_me(u: CurrentUser, State(st): State<AppState>, Json(p): Json<PatchMe>) -> Result<Json<Value>, AppError> {
    if let Some(n) = &p.display_name {
        if !(1..=50).contains(&n.trim().chars().count()) { return Err(AppError::invalid("/display_name", "RANGE", "暱稱長度須為 1–50 字")); }
        sqlx::query("UPDATE users SET display_name=$2 WHERE id=$1").bind(u.id).bind(n.trim()).execute(&st.pool).await?;
    }
    if let Some(prefs) = p.notification_prefs {
        let ok = prefs.as_object().is_some_and(|o| !o.is_empty() && o.iter().all(|(k, v)| k == "email_claims" && v.is_boolean()));
        if !ok { return Err(AppError::invalid("/notification_prefs", "FORMAT", "僅支援布林值的 email_claims")); }
        sqlx::query("UPDATE users SET notification_prefs = notification_prefs || $2 WHERE id=$1").bind(u.id).bind(prefs).execute(&st.pool).await?;
    }
    Ok(Json(user_json(&st.pool, u.id).await?))
}

// ---------- LINE Login ----------
/// HTTP 薄層設定：全部由環境變數驅動，URL 可指向 mock。
pub struct LineCfg { pub auth_url: String, pub token_url: String, pub profile_url: String, pub client_id: String, pub secret: String, pub redirect_uri: String }
impl LineCfg {
    pub fn from_env() -> Option<LineCfg> {
        Some(LineCfg {
            client_id: crate::auth_ext::nonempty("LINE_CHANNEL_ID")?, secret: crate::auth_ext::nonempty("LINE_CHANNEL_SECRET")?,
            redirect_uri: env("LINE_REDIRECT_URI", "http://localhost:8080/api/v1/auth/oauth/line/callback"),
            auth_url: env("LINE_AUTH_URL", "https://access.line.me/oauth2/v2.1/authorize"),
            token_url: env("LINE_TOKEN_URL", "https://api.line.me/oauth2/v2.1/token"),
            profile_url: env("LINE_PROFILE_URL", "https://api.line.me/v2/profile"),
        })
    }
}
pub struct LineProfile { pub user_id: String, pub display_name: String, pub picture_url: Option<String> }

pub async fn line_fetch_profile(cfg: &LineCfg, code: &str, verifier: &str) -> Result<LineProfile, AppError> {
    let bad = |e: String| { tracing::error!(error = %e, "line oauth"); AppError::problem(502, "OAUTH_PROVIDER_ERROR", "LINE 登入暫時無法使用") };
    let c = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().map_err(|e| bad(e.to_string()))?;
    let tok: Value = c.post(&cfg.token_url)
        .form(&[("grant_type", "authorization_code"), ("code", code), ("redirect_uri", &cfg.redirect_uri),
                ("client_id", &cfg.client_id), ("client_secret", &cfg.secret), ("code_verifier", verifier)])
        .send().await.and_then(|r| r.error_for_status()).map_err(|e| bad(e.to_string()))?
        .json().await.map_err(|e| bad(e.to_string()))?;
    let at = tok["access_token"].as_str().ok_or_else(|| bad("no access_token".into()))?;
    let p: Value = c.get(&cfg.profile_url).bearer_auth(at).send().await.and_then(|r| r.error_for_status()).map_err(|e| bad(e.to_string()))?
        .json().await.map_err(|e| bad(e.to_string()))?;
    Ok(LineProfile {
        user_id: p["userId"].as_str().ok_or_else(|| bad("no userId".into()))?.to_string(),
        display_name: p["displayName"].as_str().unwrap_or("LINE 使用者").to_string(),
        picture_url: p["pictureUrl"].as_str().map(String::from),
    })
}

pub(crate) fn sign(payload: &str) -> String {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(crate::config::get().oauth_secret.as_bytes()).unwrap();
    m.update(payload.as_bytes());
    hex::encode(m.finalize().into_bytes())
}
pub(crate) fn safe_redirect(r: &str) -> bool { r.starts_with('/') && !r.starts_with("//") && !r.contains('\\') && !r.contains(|c: char| c.is_control()) }
pub(crate) fn rand_b64(n: usize) -> String { let mut b = vec![0u8; n]; rand::thread_rng().fill_bytes(&mut b); B64.encode(b) }

#[derive(Deserialize)]
struct StartQ { redirect: Option<String> }

async fn oauth_start(Path(provider): Path<String>, Query(q): Query<StartQ>) -> Result<Response, AppError> {
    if provider != "line" && provider != "google" { return Err(AppError::invalid("/provider", "ENUM", "provider 須為 line 或 google")); }
    let redirect = q.redirect.unwrap_or_else(|| "/dashboard".into());
    if !safe_redirect(&redirect) { return Err(AppError::invalid("/redirect", "FORMAT", "redirect 必須是以 / 開頭的相對路徑")); }
    // 未設憑證：瀏覽器是整頁導向進來的，回 302 到登入頁讓前端顯示提示，而不是一頁 JSON
    let unavailable = || (StatusCode::FOUND, [(header::LOCATION, format!("{}/login?error=oauth_unavailable", app_url()))]).into_response();
    let (state, verifier) = (rand_b64(16), rand_b64(32));
    let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
    let (auth_url, params): (String, Vec<(&str, String)>) = if provider == "line" {
        let Some(cfg) = LineCfg::from_env() else { return Ok(unavailable()) };
        (cfg.auth_url, vec![("client_id", cfg.client_id), ("redirect_uri", cfg.redirect_uri), ("scope", "profile openid".into())])
    } else {
        let Some(cfg) = crate::auth_ext::GoogleCfg::from_env() else { return Ok(unavailable()) };
        (cfg.auth_url, vec![("client_id", cfg.client_id), ("redirect_uri", cfg.redirect_uri), ("scope", "openid email profile".into())])
    };
    let mut all: Vec<(&str, String)> = vec![("response_type", "code".into()), ("state", state.clone()), ("code_challenge", challenge), ("code_challenge_method", "S256".into())];
    all.extend(params);
    let url = reqwest::Url::parse_with_params(&auth_url, all)
        .map_err(|e| AppError::problem(502, "OAUTH_PROVIDER_ERROR", e.to_string()))?;
    let payload = format!("{state}.{verifier}.{}", B64.encode(&redirect));
    let ck = format!("ws_oauth_state={payload}.{}; Path=/api/v1/auth/oauth; Max-Age=600; HttpOnly; SameSite=Lax{}", sign(&payload), secure());
    Ok((StatusCode::FOUND, [(header::LOCATION, url.to_string()), (header::SET_COOKIE, ck)]).into_response())
}

#[derive(Deserialize)]
struct CbQ { code: Option<String>, state: Option<String>, error: Option<String> }

/// 驗證 ws_oauth_state cookie 與 query state；回傳 (verifier, redirect)。
fn check_state(cookie_val: Option<&str>, state: Option<&str>) -> Option<(String, String)> {
    let c = cookie_val?;
    let (payload, sig) = c.rsplit_once('.')?;
    if !crate::config::ct_eq(sign(payload).as_bytes(), sig.as_bytes()) { return None; }
    let mut it = payload.split('.');
    let (s, v, r) = (it.next()?, it.next()?, it.next()?);
    if !crate::config::ct_eq(s.as_bytes(), state?.as_bytes()) { return None; }
    let redir = String::from_utf8(B64.decode(r).ok()?).ok()?;
    safe_redirect(&redir).then(|| (v.to_string(), redir))
}

async fn oauth_callback(State(st): State<AppState>, Path(provider): Path<String>, Query(q): Query<CbQ>, headers: HeaderMap) -> Result<Response, AppError> {
    let clear = format!("ws_oauth_state=; Max-Age=0; Path=/api/v1/auth/oauth; HttpOnly; SameSite=Lax{}", secure());
    let fail = || (StatusCode::FOUND, AppendHeaders([(header::LOCATION, format!("{}/login?error=oauth_failed", app_url())), (header::SET_COOKIE, clear.clone())])).into_response();
    if (provider != "line" && provider != "google") || q.error.is_some() { return Ok(fail()); }
    let (Some(code), Some((verifier, redirect))) = (q.code.as_deref(), check_state(cookie(&headers, "ws_oauth_state").as_deref(), q.state.as_deref())) else { return Ok(fail()) };
    let (uid, _) = if provider == "line" {
        let cfg = LineCfg::from_env().ok_or_else(|| AppError::problem(502, "OAUTH_PROVIDER_ERROR", "LINE 登入尚未設定"))?;
        let p = line_fetch_profile(&cfg, code, &verifier).await?;
        upsert_identity(&st.pool, "line", &p.user_id, None, &p.display_name, p.picture_url.as_deref()).await?
    } else {
        // id_token 驗證失敗（簽章 / iss / aud / exp / email_verified）→ 與 state 失敗同樣 302 login?error=oauth_failed
        let Some(g) = crate::auth_ext::google_identity(code, &verifier).await? else { return Ok(fail()) };
        upsert_identity(&st.pool, "google", &g.sub, g.email.as_deref(), &g.name, g.picture.as_deref()).await?
    };
    let token = new_session(&st.pool, uid, headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok())).await?;
    Ok((StatusCode::FOUND, AppendHeaders([(header::LOCATION, format!("{}{redirect}", app_url())), (header::SET_COOKIE, session_cookie(&token)), (header::SET_COOKIE, clear)])).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_cookie_roundtrip_and_tamper() {
        let payload = format!("st1.ver1.{}", B64.encode("/dashboard"));
        let c = format!("{payload}.{}", sign(&payload));
        assert_eq!(check_state(Some(&c), Some("st1")), Some(("ver1".into(), "/dashboard".into())));
        assert!(check_state(Some(&c), Some("other")).is_none());
        assert!(check_state(Some(&c.replace("ver1", "verX")), Some("st1")).is_none());
        assert!(check_state(None, Some("st1")).is_none());
        assert!(!safe_redirect("//evil.com") && !safe_redirect("https://x") && safe_redirect("/a/b"));
    }
}
