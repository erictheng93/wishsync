//! 圖片上傳：presigned PUT → incoming/（私有）→ confirm 時重編碼去 EXIF/GPS → 公開 key。
//! 以手寫 SigV4 query presign 對接 S3 相容儲存（MinIO / R2），不引入 AWS SDK。
use crate::{error::AppError, session::CurrentUser, AppState};
use axum::{extract::{Json, State}, routing::post, Router};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const MAX_BYTES: usize = 5 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new().route("/uploads/presign", post(presign)).route("/uploads/confirm", post(confirm))
}

// ---------- S3 設定與簽章 ----------
#[derive(Clone)] // 同上：含 secret key
pub struct S3 { pub endpoint: String, pub bucket: String, pub ak: String, pub sk: String, pub region: String, pub public_base: String }

impl S3 {
    /// dev/test：MinIO 預設，仍可由 S3_* 覆寫。production 的必填檢查在 Config::try_from_env。
    pub fn dev() -> S3 {
        let g = |k: &str, d: &str| std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string());
        let endpoint = g("S3_ENDPOINT", "http://localhost:9000").trim_end_matches('/').to_string();
        let bucket = g("S3_BUCKET", "wishsync-media");
        let public_base = g("S3_PUBLIC_BASE", &format!("{endpoint}/{bucket}"));
        S3 { endpoint, bucket, ak: g("S3_ACCESS_KEY", "wishsync"), sk: g("S3_SECRET_KEY", "wishsync-secret"),
             region: g("S3_REGION", "us-east-1"), public_base }
    }
    /// 讀全域 Config
    pub fn get() -> S3 { crate::config::get().s3 }
    pub fn public_url(&self, key: &str) -> String { format!("{}/{}", self.public_base.trim_end_matches('/'), key) }
    fn host(&self) -> &str { self.endpoint.split("://").nth(1).unwrap_or(&self.endpoint) }
    /// 回傳完整 presigned URL（path-style）。`headers` 為要簽進去的額外標頭（小寫名稱）。
    pub fn presign(&self, method: &str, key: &str, expires: i64, headers: &[(&str, &str)], now: DateTime<Utc>) -> String {
        let path = if key.is_empty() { format!("/{}", self.bucket) } else { format!("/{}/{}", self.bucket, key) };
        sign_url(&self.endpoint, self.host(), &path, method, expires, headers, now, &self.ak, &self.sk, &self.region)
    }
}

fn enc(s: &str, keep_slash: bool) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
        b'/' if keep_slash => "/".into(),
        _ => format!("%{b:02X}"),
    }).collect()
}
fn hmac(key: &[u8], msg: &str) -> Vec<u8> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap();
    m.update(msg.as_bytes());
    m.finalize().into_bytes().to_vec()
}

#[allow(clippy::too_many_arguments)]
pub fn sign_url(endpoint: &str, host: &str, path: &str, method: &str, expires: i64, extra: &[(&str, &str)],
                now: DateTime<Utc>, ak: &str, sk: &str, region: &str) -> String {
    let dt = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = &dt[..8];
    let scope = format!("{date}/{region}/s3/aws4_request");
    let mut hdrs: Vec<(String, String)> = extra.iter().map(|(k, v)| (k.to_lowercase(), v.trim().to_string())).collect();
    hdrs.push(("host".into(), host.into()));
    hdrs.sort();
    let signed = hdrs.iter().map(|h| h.0.as_str()).collect::<Vec<_>>().join(";");
    let canon_headers: String = hdrs.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let mut q = vec![
        ("X-Amz-Algorithm", "AWS4-HMAC-SHA256".to_string()),
        ("X-Amz-Credential", format!("{ak}/{scope}")),
        ("X-Amz-Date", dt.clone()),
        ("X-Amz-Expires", expires.to_string()),
        ("X-Amz-SignedHeaders", signed.clone()),
    ];
    q.sort();
    let query = q.iter().map(|(k, v)| format!("{}={}", enc(k, false), enc(v, false))).collect::<Vec<_>>().join("&");
    let canon_path = enc(path, true);
    let canon = format!("{method}\n{canon_path}\n{query}\n{canon_headers}\n{signed}\nUNSIGNED-PAYLOAD");
    let sts = format!("AWS4-HMAC-SHA256\n{dt}\n{scope}\n{}", hex::encode(Sha256::digest(canon.as_bytes())));
    let k = hmac(&hmac(&hmac(&hmac(format!("AWS4{sk}").as_bytes(), date), region), "s3"), "aws4_request");
    let sig = hex::encode(hmac(&k, &sts));
    format!("{endpoint}{canon_path}?{query}&X-Amz-Signature={sig}")
}

// ponytail: 薄層，測試可改指向本機 mock；單次請求無重試。
async fn s3_req(method: &str, key: &str, body: Option<(Vec<u8>, &str)>) -> Result<reqwest::Response, AppError> {
    let s3 = S3::get();
    let url = s3.presign(method, key, 300, &[], Utc::now());
    let c = reqwest::Client::new();
    let mut rb = c.request(method.parse().unwrap(), url);
    if let Some((b, ct)) = body { rb = rb.header("content-type", ct).body(b); }
    rb.send().await.map_err(|e| { tracing::error!(error=%e, "s3"); AppError::problem(502, "UPSTREAM_ERROR", "儲存服務暫時無法使用") })
}

async fn ensure_bucket() {
    static ONCE: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    ONCE.get_or_init(|| async { let _ = s3_req("PUT", "", None).await; }).await;
}

// ---------- 去 EXIF ----------
/// 解碼 → 套用方向 → 重新編碼（同格式）；中繼資料（EXIF/GPS/ICC/XMP）全數丟棄。回傳 (bytes, content_type)。
pub fn strip_metadata(data: &[u8]) -> Result<(Vec<u8>, &'static str), String> {
    use image::{ImageDecoder, ImageFormat, ImageReader};
    if data.len() > MAX_BYTES { return Err("檔案超過 5 MB".into()); }
    let mut rd = ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().map_err(|e| e.to_string())?;
    let fmt = rd.format().ok_or("無法辨識圖片格式")?;
    let ct = match fmt { ImageFormat::Jpeg => "image/jpeg", ImageFormat::Png => "image/png", ImageFormat::WebP => "image/webp",
                         _ => return Err("僅支援 JPEG / PNG / WebP".into()) };
    let mut lim = image::Limits::default();
    lim.max_image_width = Some(8192); lim.max_image_height = Some(8192); lim.max_alloc = Some(256 << 20);
    rd.limits(lim);
    let mut dec = rd.into_decoder().map_err(|e| e.to_string())?;
    let orient = dec.orientation().map_err(|e| e.to_string())?;
    let mut img = image::DynamicImage::from_decoder(dec).map_err(|e| e.to_string())?;
    img.apply_orientation(orient);
    let mut out = Vec::new();
    let w = std::io::Cursor::new(&mut out);
    match fmt {
        ImageFormat::Jpeg => image::DynamicImage::ImageRgb8(img.to_rgb8()).write_to(&mut { w }, ImageFormat::Jpeg),
        f => img.write_to(&mut { w }, f),
    }.map_err(|e| e.to_string())?;
    Ok((out, ct))
}

// ---------- handlers ----------
#[derive(Deserialize)]
struct PresignReq { purpose: String, content_type: String, content_length: usize }

fn ext_of(ct: &str) -> Option<&'static str> {
    match ct { "image/jpeg" => Some("jpg"), "image/png" => Some("png"), "image/webp" => Some("webp"), _ => None }
}

async fn presign(State(st): State<AppState>, user: CurrentUser, Json(r): Json<PresignReq>) -> Result<Json<Value>, AppError> {
    let dir = match r.purpose.as_str() { "cover" => "covers", "item" => "items",
        _ => return Err(AppError::invalid("/purpose", "ENUM", "purpose 必須為 cover 或 item")) };
    let ext = ext_of(&r.content_type).ok_or_else(|| AppError::invalid("/content_type", "ENUM", "只允許 image/jpeg、image/png、image/webp"))?;
    if r.content_length == 0 || r.content_length > MAX_BYTES {
        return Err(AppError::invalid("/content_length", "RANGE", "圖片大小須介於 1 byte 與 5 MB"));
    }
    crate::ratelimit::check(&st.pool, &format!("presign:{}", user.id), 60, 3600).await?;
    ensure_bucket().await;
    let s3 = S3::get();
    let object_key = format!("{dir}/{}.{ext}", Uuid::new_v4());
    let now = Utc::now();
    let len = r.content_length.to_string();
    let url = s3.presign("PUT", &format!("incoming/{}/{object_key}", user.id), 300, &[("content-type", &r.content_type), ("content-length", &len)], now);
    Ok(Json(json!({
        "upload_url": url, "method": "PUT",
        "headers": { "Content-Type": r.content_type, "Content-Length": len },
        "object_key": object_key, "public_url": s3.public_url(&object_key), "processing": true,
        "expires_at": now + Duration::minutes(5),
    })))
}

#[derive(Deserialize)]
struct ConfirmReq { object_key: String }

pub fn valid_key(k: &str) -> bool {
    let Some((dir, f)) = k.split_once('/') else { return false };
    let Some((id, ext)) = f.rsplit_once('.') else { return false };
    matches!(dir, "covers" | "items") && matches!(ext, "jpg" | "png" | "webp") && Uuid::parse_str(id).is_ok()
}

/// 前端 PUT 完成後呼叫：驗證、去中繼資料、寫入公開 key、刪除 incoming。（契約未列，補充端點）
async fn confirm(user: CurrentUser, Json(r): Json<ConfirmReq>) -> Result<Json<Value>, AppError> {
    if !valid_key(&r.object_key) { return Err(AppError::invalid("/object_key", "FORMAT", "object_key 格式不正確")); }
    let inc = format!("incoming/{}/{}", user.id, r.object_key);
    let res = s3_req("GET", &inc, None).await?;
    if res.status().as_u16() == 404 { return Err(AppError::NotFound); }
    if !res.status().is_success() { return Err(AppError::problem(502, "UPSTREAM_ERROR", "儲存服務暫時無法使用")); }
    if res.content_length().is_some_and(|l| l as usize > MAX_BYTES) {
        let _ = s3_req("DELETE", &inc, None).await;
        return Err(AppError::invalid("/object_key", "IMAGE_REJECTED", "檔案超過 5 MB"));
    }
    let bytes = res.bytes().await.map_err(|_| AppError::problem(502, "UPSTREAM_ERROR", "儲存服務暫時無法使用"))?;
    let want_ext = r.object_key.rsplit('.').next().unwrap_or("");
    match strip_metadata(&bytes) {
        Ok((clean, ct)) if ext_of(ct) == Some(want_ext) => {
            s3_req("PUT", &r.object_key, Some((clean, ct))).await?;
            let _ = s3_req("DELETE", &inc, None).await;
            Ok(Json(json!({ "object_key": r.object_key, "public_url": S3::get().public_url(&r.object_key), "image_status": "ready" })))
        }
        other => {
            let _ = s3_req("DELETE", &inc, None).await;
            let why = other.err().unwrap_or_else(|| "副檔名與實際格式不符".into());
            Err(AppError::invalid("/object_key", "IMAGE_REJECTED", &format!("圖片驗證失敗：{why}，請重新上傳")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigv4_matches_aws_doc_example() {
        let now = DateTime::parse_from_rfc3339("2013-05-24T00:00:00Z").unwrap().with_timezone(&Utc);
        let u = sign_url("https://examplebucket.s3.amazonaws.com", "examplebucket.s3.amazonaws.com", "/test.txt", "GET", 86400, &[],
            now, "AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "us-east-1");
        assert!(u.ends_with("X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"), "{u}");
    }

    /// 組一張帶 EXIF（含 GPS IFD）的 JPEG
    fn jpeg_with_gps() -> Vec<u8> {
        let mut base = Vec::new();
        image::DynamicImage::new_rgb8(8, 8).write_to(&mut std::io::Cursor::new(&mut base), image::ImageFormat::Jpeg).unwrap();
        let mut tiff: Vec<u8> = vec![b'I', b'I', 0x2A, 0, 8, 0, 0, 0];
        tiff.extend([1, 0, 0x25, 0x88, 4, 0, 1, 0, 0, 0, 26, 0, 0, 0, 0, 0, 0, 0]); // IFD0: GPSInfo -> offset 26
        tiff.extend([1, 0, 1, 0, 2, 0, 2, 0, 0, 0, b'N', 0, 0, 0, 0, 0, 0, 0]);     // GPS IFD: LatitudeRef "N"
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(tiff);
        let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
        out.extend(((app1.len() + 2) as u16).to_be_bytes());
        out.extend(app1);
        out.extend(&base[2..]);
        out
    }

    #[test]
    fn exif_gps_is_stripped() {
        let src = jpeg_with_gps();
        assert!(src.windows(4).any(|w| w == b"Exif"));
        let (out, ct) = strip_metadata(&src).unwrap();
        assert_eq!(ct, "image/jpeg");
        assert!(!out.windows(4).any(|w| w == b"Exif"), "EXIF 殘留");
        assert!(!out.windows(2).any(|w| w == [0xFF, 0xE1]), "APP1 殘留");
        assert_eq!(image::load_from_memory(&out).unwrap().width(), 8);
    }

    #[test]
    fn rejects_non_images_and_bad_keys() {
        assert!(strip_metadata(b"GIF89a not really").is_err());
        assert!(strip_metadata(&[0u8; 100]).is_err());
        assert!(valid_key(&format!("items/{}.webp", Uuid::new_v4())));
        assert!(!valid_key("items/../x.png") && !valid_key("incoming/a/b.png"));
    }
}

// 手寫 Debug：不印 access / secret key
impl std::fmt::Debug for S3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_struct("S3").field("endpoint", &self.endpoint).field("bucket", &self.bucket).finish_non_exhaustive() }
}
