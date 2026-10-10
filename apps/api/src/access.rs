//! 清單與捐助的可見性判斷（0004_social）。所有對外讀取/認領/SSE/檢舉都經 `check_wishlist`。
use crate::{error::AppError, session::{cookie_value, hash_token}, AppState};
use axum::http::request::Parts;
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use sqlx::PgExecutor;
use uuid::Uuid;

/// 無向好友關係
pub async fn are_friends<'e, E: PgExecutor<'e>>(ex: E, a: Uuid, b: Uuid) -> Result<bool, sqlx::Error> {
    if a == b { return Ok(false); }
    let (x, y) = if a < b { (a, b) } else { (b, a) };
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM friendships WHERE user_a = $1 AND user_b = $2)")
        .bind(x).bind(y).fetch_one(ex).await
}

/// 從 ws_session cookie 取得登入者（可無）；不拒絕請求
pub async fn viewer(parts: &Parts, st: &AppState) -> Result<Option<Uuid>, AppError> {
    let Some(tok) = cookie_value(parts, "ws_session") else { return Ok(None) };
    Ok(sqlx::query_scalar(
        "SELECT u.id FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > now() AND u.deleted_at IS NULL")
        .bind(hash_token(tok)).fetch_optional(&st.pool).await?)
}

/// 密碼清單的存取權杖：HMAC(wishlist_id + 目前密碼雜湊)；改密碼即全部失效。
/// 由 POST /public/wishlists/{slug}/unlock 簽發，客戶端以 `X-List-Access` header（SSE 用 `?access=`）帶回。
pub fn access_token(wishlist_id: Uuid, pw_hash: &str) -> String {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(crate::config::get().unsub_secret.as_bytes()).unwrap();
    m.update(format!("list-access:{wishlist_id}:{pw_hash}").as_bytes());
    hex::encode(m.finalize().into_bytes())
}

pub fn access_ok(wishlist_id: Uuid, pw_hash: Option<&str>, token: Option<&str>) -> bool {
    match (pw_hash, token) { (Some(h), Some(t)) => subtle_eq(access_token(wishlist_id, h).as_bytes(), t.as_bytes()), _ => false }
}

fn subtle_eq(a: &[u8], b: &[u8]) -> bool { a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0 }

/// 已在外層排除 draft/archived/刪除/下架；這裡只看 visibility。
/// 錯誤：private → 404；friends/selected 未登入 → 403 LOGIN_REQUIRED；非好友/不在名單 → 403 FRIENDS_ONLY / NOT_ALLOWED；
/// 密碼缺或錯 → 403 PASSWORD_REQUIRED。403 皆附 `visibility` 與 `owner`（{id, display_name, handle}）供前端顯示門檻頁。
pub struct ListAccess<'a> {
    pub wishlist_id: Uuid,
    pub owner_id: Uuid,
    pub visibility: &'a str,
    pub pw_hash: Option<&'a str>,
}

pub async fn check_wishlist(pool: &sqlx::PgPool, w: &ListAccess<'_>, viewer: Option<Uuid>, access: Option<&str>) -> Result<(), AppError> {
    if viewer == Some(w.owner_id) { return Ok(()); }
    let deny = |code, detail| deny(pool, w, code, detail);
    match w.visibility {
        "public" | "link" => Ok(()),
        "private" => Err(AppError::NotFound),
        "password" if access_ok(w.wishlist_id, w.pw_hash, access) => Ok(()),
        "password" => deny("PASSWORD_REQUIRED", "此清單需要存取密碼").await,
        "friends" | "selected" if viewer.is_none() => deny("LOGIN_REQUIRED", "此清單僅限特定對象，請先登入").await,
        "friends" if are_friends(pool, w.owner_id, viewer.unwrap()).await? => Ok(()),
        "friends" => deny("FRIENDS_ONLY", "此清單僅限好友查看").await,
        "selected" => {
            let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM wishlist_allowed_users WHERE wishlist_id = $1 AND user_id = $2)")
                .bind(w.wishlist_id).bind(viewer.unwrap()).fetch_one(pool).await?;
            if ok { Ok(()) } else { deny("NOT_ALLOWED", "此清單僅限指定對象查看").await }
        }
        _ => Err(AppError::NotFound),
    }
}

async fn deny(pool: &sqlx::PgPool, w: &ListAccess<'_>, code: &'static str, detail: &str) -> Result<(), AppError> {
    let owner: Option<(String, Option<String>)> = sqlx::query_as("SELECT display_name, handle FROM users WHERE id = $1")
        .bind(w.owner_id).fetch_optional(pool).await?;
    let (name, handle) = owner.unwrap_or_default();
    Err(AppError::Extra { status: 403, code, detail: detail.into(), extra: json!({
        "visibility": w.visibility, "owner": { "id": w.owner_id, "display_name": name, "handle": handle } }) })
}

/// 捐助（認領）是否對 viewer 可見：public 任何人；friends 需為認領者好友；private 只有本人。
pub async fn claim_visible(pool: &sqlx::PgPool, level: &str, claimer: Uuid, viewer: Option<Uuid>) -> Result<bool, sqlx::Error> {
    Ok(match (level, viewer) {
        (_, Some(v)) if v == claimer => true,
        ("public", _) => true,
        ("friends", Some(v)) => are_friends(pool, claimer, v).await?,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn subtle_eq_works() {
        assert!(super::subtle_eq(b"abc", b"abc"));
        assert!(!super::subtle_eq(b"abc", b"abd"));
        assert!(!super::subtle_eq(b"abc", b"ab"));
    }
}
