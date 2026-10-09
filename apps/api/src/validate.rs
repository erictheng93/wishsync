//! 集中式輸入驗證（F-03 / F-11 / F-16 / F-17）。回傳 422 VALIDATION_FAILED 並指出欄位 pointer。
use crate::error::AppError;
use chrono::NaiveDate;
use std::str::FromStr;

fn zero_width(c: char) -> bool { matches!(c, '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}') }
fn bidi_override(c: char) -> bool { matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}') }

/// trim 後檢查：拒絕控制字元（含 NUL；`multi` 時保留 \n \r \t）與雙向覆寫字元。
/// 去除零寬字元後為空（只剩空白/零寬）者視為空字串回傳，由呼叫端依「必填/選填」處理。
pub fn text(s: &str, pointer: &str, multi: bool) -> Result<String, AppError> {
    let s = s.trim();
    let bad = s.chars().any(|c| bidi_override(c) || (c.is_control() && !(multi && matches!(c, '\n' | '\r' | '\t'))));
    if bad { return Err(AppError::invalid(pointer, "INVALID_CHARS", "含有不允許的控制或方向覆寫字元")); }
    if s.chars().all(|c| c.is_whitespace() || zero_width(c)) { return Ok(String::new()); }
    Ok(s.to_string())
}

/// 可寄送的 Email（lettre::Address 解析）；回傳小寫、trim 後的值。
pub fn email(s: &str, pointer: &str) -> Result<String, AppError> {
    let e = s.trim().to_lowercase();
    let ok = e.len() <= 200 && !e.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>'))
        && lettre::Address::from_str(&e).is_ok();
    if ok { Ok(e) } else { Err(AppError::invalid(pointer, "FORMAT", "Email 格式不正確")) }
}

/// 活動日限制 2000-01-01..2100-12-31
pub fn event_date(d: NaiveDate, pointer: &str) -> Result<NaiveDate, AppError> {
    let (lo, hi) = (NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(), NaiveDate::from_ymd_opt(2100, 12, 31).unwrap());
    if (lo..=hi).contains(&d) { Ok(d) } else { Err(AppError::invalid(pointer, "RANGE", "日期須介於 2000-01-01 與 2100-12-31")) }
}

/// 分享代碼：10 碼 ASCII 英數（含 %00 等一律視為不存在）
pub fn slug_ok(s: &str) -> bool { s.len() == 10 && s.bytes().all(|b| b.is_ascii_alphanumeric()) }
