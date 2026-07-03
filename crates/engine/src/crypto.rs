//! Storage encryption — byte-compatible with the Node StorageService:
//! `oa_enc_idf_v1::` + IV(16) + AES-256-CBC(PKCS7) ciphertext. Files without
//! the magic prefix are stored plaintext and returned as-is.
//!
//! Credentials use the Node CryptoService format instead:
//! hex( salt(64) | iv(16) | gcm-tag(16) | AES-256-GCM ciphertext ), with the
//! key scrypt-derived (N=16384, r=8, p=1) from the master key *string*.

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use aes_gcm::aead::{Aead, KeyInit, Payload};

const ENCRYPTION_PREFIX: &[u8] = b"oa_enc_idf_v1::";

type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;

pub fn decrypt_storage(content: Vec<u8>, key: &Option<[u8; 32]>) -> Result<Vec<u8>, String> {
    let Some(key) = key else {
        return Ok(content);
    };
    if !content.starts_with(ENCRYPTION_PREFIX) {
        return Ok(content);
    }
    let body = &content[ENCRYPTION_PREFIX.len()..];
    if body.len() < 16 {
        return Err("encrypted file too short".into());
    }
    let (iv, ciphertext) = body.split_at(16);
    let mut buf = ciphertext.to_vec();
    let plain = Aes256CbcDec::new(key.into(), iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|_| "Failed to decrypt file. It may be corrupted or the key is incorrect.")?;
    Ok(plain.to_vec())
}

/// Write-side twin of decrypt_storage. No key → stored plaintext (Node parity).
pub fn encrypt_storage(content: &[u8], key: &Option<[u8; 32]>) -> Vec<u8> {
    let Some(key) = key else {
        return content.to_vec();
    };
    let iv: [u8; 16] = rand::random();
    let ciphertext =
        Aes256CbcEnc::new(key.into(), &iv.into()).encrypt_padded_vec_mut::<Pkcs7>(content);
    let mut out = Vec::with_capacity(ENCRYPTION_PREFIX.len() + 16 + ciphertext.len());
    out.extend_from_slice(ENCRYPTION_PREFIX);
    out.extend_from_slice(&iv);
    out.extend_from_slice(&ciphertext);
    out
}

fn scrypt_key(master_key: &str, salt: &[u8]) -> [u8; 32] {
    // Node's scryptSync defaults: N=16384 (log2=14), r=8, p=1.
    let params = scrypt::Params::new(14, 8, 1, 32).unwrap();
    let mut key = [0u8; 32];
    scrypt::scrypt(master_key.as_bytes(), salt, &params, &mut key).unwrap();
    key
}

/// CryptoService.encrypt — used for ingestion-source credentials.
pub fn encrypt_credentials(value: &str, master_key: &str) -> String {
    let salt: [u8; 64] = std::array::from_fn(|_| rand::random());
    let key = scrypt_key(master_key, &salt);
    // Node passes a 16-byte IV to aes-256-gcm — mirror it via GCM<U16>.
    let iv16: [u8; 16] = std::array::from_fn(|_| rand::random());
    let cipher = aes_gcm::AesGcm::<aes::Aes256, aes_gcm::aead::consts::U16>::new((&key).into());
    let out = cipher
        .encrypt((&iv16).into(), Payload { msg: value.as_bytes(), aad: &[] })
        .expect("gcm encrypt");
    // aes-gcm appends the tag; Node stores salt|iv|tag|ciphertext.
    let (ciphertext, tag) = out.split_at(out.len() - 16);
    let mut packed = Vec::with_capacity(64 + 16 + 16 + ciphertext.len());
    packed.extend_from_slice(&salt);
    packed.extend_from_slice(&iv16);
    packed.extend_from_slice(tag);
    packed.extend_from_slice(ciphertext);
    hex::encode(packed)
}

/// CryptoService.decrypt — needed once the Rust engine reads source
/// credentials itself (R3); until then only tests exercise it.
#[allow(dead_code)]
pub fn decrypt_credentials(encrypted_hex: &str, master_key: &str) -> Option<String> {
    let data = hex::decode(encrypted_hex).ok()?;
    if data.len() < 64 + 16 + 16 {
        return None;
    }
    let (salt, rest) = data.split_at(64);
    let (iv, rest) = rest.split_at(16);
    let (tag, ciphertext) = rest.split_at(16);
    let key = scrypt_key(master_key, salt);
    let cipher = aes_gcm::AesGcm::<aes::Aes256, aes_gcm::aead::consts::U16>::new((&key).into());
    let mut payload = ciphertext.to_vec();
    payload.extend_from_slice(tag);
    let iv: &[u8; 16] = iv.try_into().ok()?;
    let plain = cipher
        .decrypt(iv.into(), Payload { msg: &payload, aad: &[] })
        .ok()?;
    String::from_utf8(plain).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_round_trip() {
        let key = Some([7u8; 32]);
        let plain = b"hello encrypted world".to_vec();
        let enc = encrypt_storage(&plain, &key);
        assert!(enc.starts_with(ENCRYPTION_PREFIX));
        assert_eq!(decrypt_storage(enc, &key).unwrap(), plain);
    }

    #[test]
    fn credentials_round_trip() {
        let master = "0123abcd";
        let enc = encrypt_credentials("{\"localFilePath\":\"/x\"}", master);
        assert_eq!(
            decrypt_credentials(&enc, master).as_deref(),
            Some("{\"localFilePath\":\"/x\"}")
        );
    }
}
