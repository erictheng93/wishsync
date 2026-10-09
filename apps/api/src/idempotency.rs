//! Idempotency-Key（契約第 5.x 章 / 4.1 (a)(e)）：占位與業務寫入同一交易，失敗 rollback 不留痕。
use crate::error::AppError;
use axum::http::HeaderMap;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub fn key(h: &HeaderMap) -> Result<Uuid, AppError> {
    h.get("idempotency-key").and_then(|v| v.to_str().ok()).and_then(|s| Uuid::parse_str(s.trim()).ok())
        .ok_or_else(|| AppError::problem(400, "IDEMPOTENCY_KEY_REQUIRED", "缺少 Idempotency-Key（需為 UUID）"))
}

/// SHA-256(method+path + body)
pub fn request_hash(method_path: &str, body: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(method_path.as_bytes());
    h.update(body);
    h.finalize().to_vec()
}

pub enum Begin { Fresh, Replay(i32, Value) }

fn conflict() -> AppError {
    AppError::Problem { status: 409, code: "IDEMPOTENCY_CONFLICT", detail: "Idempotency-Key 已用於不同的請求內容或仍在處理中".into(),
        errors: None, retry_after: Some(1) }
}

pub async fn begin(tx: &mut Transaction<'_, Postgres>, scope: &str, key: Uuid, hash: &[u8]) -> Result<Begin, AppError> {
    // 併發同 key：後者在此等前者 commit/rollback，之後看到已提交的列或自己占位成功
    let ins: Option<(Uuid,)> = sqlx::query_as(
        "INSERT INTO idempotency_keys (key, scope, request_hash) VALUES ($1, $2, $3)
         ON CONFLICT (scope, key) DO NOTHING RETURNING key")
        .bind(key).bind(scope).bind(hash).fetch_optional(&mut **tx).await?;
    if ins.is_some() { return Ok(Begin::Fresh); }
    let (h, st, body): (Vec<u8>, Option<i32>, Option<Value>) = sqlx::query_as(
        "SELECT request_hash, response_status, response_body FROM idempotency_keys WHERE scope = $1 AND key = $2")
        .bind(scope).bind(key).fetch_one(&mut **tx).await?;
    match (h == hash, st, body) {
        (true, Some(st), Some(body)) => Ok(Begin::Replay(st, body)),
        _ => Err(conflict()),
    }
}

pub async fn finish(tx: &mut Transaction<'_, Postgres>, scope: &str, key: Uuid, status: i32, body: &Value) -> Result<(), AppError> {
    sqlx::query("UPDATE idempotency_keys SET response_status = $3, response_body = $4 WHERE scope = $1 AND key = $2")
        .bind(scope).bind(key).bind(status).bind(body).execute(&mut **tx).await?;
    Ok(())
}
