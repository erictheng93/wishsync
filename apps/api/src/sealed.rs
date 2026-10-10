//! 收件資訊欄位級加密（docs/04 shipping_addresses）：XChaCha20-Poly1305，格式 nonce(24) || ciphertext || tag。
//! 金鑰來自 SHIPPING_ENC_KEY（config.shipping_key）；key_version 固定 1。
//! ponytail: 金鑰在環境變數而非 KMS；要輪替時加 key_version 對應多把金鑰。
use chacha20poly1305::{aead::{Aead, AeadCore, KeyInit, OsRng}, XChaCha20Poly1305, XNonce};

fn cipher() -> XChaCha20Poly1305 { XChaCha20Poly1305::new(&crate::config::get().shipping_key.into()) }

pub fn seal(plain: &[u8]) -> Vec<u8> {
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    out.extend(cipher().encrypt(&nonce, plain).expect("encrypt"));
    out
}

/// 金鑰錯或密文被竄改 → None
pub fn open(sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < 24 { return None; }
    let (n, ct) = sealed.split_at(24);
    cipher().decrypt(XNonce::from_slice(n), ct).ok()
}

pub fn open_str(sealed: &[u8]) -> Option<String> { open(sealed).and_then(|b| String::from_utf8(b).ok()) }

#[cfg(test)]
mod tests {
    #[test]
    fn roundtrip_and_tamper() {
        let s = super::seal("台北市大安區".as_bytes());
        assert_eq!(super::open_str(&s).as_deref(), Some("台北市大安區"));
        assert_ne!(super::seal(b"x"), super::seal(b"x"), "nonce 必須隨機");
        let mut bad = s.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(super::open(&bad).is_none());
    }
}
