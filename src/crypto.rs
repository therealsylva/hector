use std::fmt;

use aes::Aes128;
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rsa::{BigUint, Pkcs1v15Encrypt, RsaPublicKey, rand_core::OsRng, rand_core::RngCore};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use zeroize::ZeroizeOnDrop;

use crate::{
    client::SportyClient,
    session::{ApiEnvelope, SUCCESS_BIZ_CODE},
};

const PUBLIC_MODULUS_HEX: &str = "c207d6f2db3fa03c7545b7e73d97550f30ef5c3019c4ba5f303fe7b1d2059c22eebde8a8cbd87310e91f84f7ecf697d365dfdb7ad5522b6fb185ea33281cf456ed845320a2e59cbaf507b1741554a478f7c21e06996018a20f00b0baffc65aeeb3b36193f6992b0f240ce84bce3bc07a4187d5ba5c5543884a6b624aab0507cf";
const PUBLIC_EXPONENT_HEX: &str = "10001";
const AES_KEY_LEN: usize = 16;

type Aes128CbcEncryptor = cbc::Encryptor<Aes128>;
type Aes128CbcDecryptor = cbc::Decryptor<Aes128>;

#[derive(ZeroizeOnDrop)]
pub struct TransactionCipher {
    key: [u8; AES_KEY_LEN],
    trans_id: String,
}

impl fmt::Debug for TransactionCipher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransactionCipher")
            .field("key", &"<redacted>")
            .field("trans_id", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Deserialize)]
struct CipherData {
    #[serde(rename = "transId")]
    trans_id: String,
}

impl TransactionCipher {
    /// Negotiates the one-hour transaction cipher used by protected write endpoints.
    ///
    /// # Errors
    ///
    /// Returns an error when no browser session is configured, key generation or
    /// RSA encryption fails, or the upstream cipher endpoint rejects the request.
    pub async fn bootstrap(client: &SportyClient) -> Result<Self> {
        client.settings().require_cookie()?;
        let mut key = [0_u8; AES_KEY_LEN];
        OsRng.fill_bytes(&mut key);
        let body = rsa_bootstrap_body(&key)?;
        let response: ApiEnvelope<CipherData> = client.post_text("base/cipher", body).await?;
        if response.biz_code != SUCCESS_BIZ_CODE {
            bail!(
                "transaction cipher bootstrap failed with bizCode {}: {}",
                response.biz_code,
                response.message
            );
        }
        let trans_id = response
            .data
            .context("transaction cipher response did not include transId")?
            .trans_id;
        if trans_id.is_empty() {
            bail!("transaction cipher response returned an empty transId");
        }
        Ok(Self { key, trans_id })
    }

    #[must_use]
    pub fn trans_id(&self) -> &str {
        &self.trans_id
    }

    /// Serializes and encrypts an upstream JSON payload using a fresh IV.
    ///
    /// # Errors
    ///
    /// Returns an error when the value cannot be serialized.
    pub fn encrypt_json<T: Serialize + ?Sized>(&self, value: &T) -> Result<String> {
        let plaintext = serde_json::to_vec(value)?;
        let mut iv = [0_u8; AES_KEY_LEN];
        OsRng.fill_bytes(&mut iv);
        Ok(encrypt_bytes(&self.key, &iv, &plaintext))
    }

    /// Decrypts an IV-prefixed Base64 response and parses its JSON value.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid Base64, a missing IV, invalid padding, or
    /// incompatible JSON.
    pub fn decrypt_json<T: DeserializeOwned>(&self, body: &str) -> Result<T> {
        let plaintext = decrypt_bytes(&self.key, body)?;
        serde_json::from_slice(&plaintext).context("decrypted transaction response was not JSON")
    }
}

fn public_key() -> Result<RsaPublicKey> {
    let modulus = BigUint::parse_bytes(PUBLIC_MODULUS_HEX.as_bytes(), 16)
        .context("embedded RSA modulus is invalid")?;
    let exponent = BigUint::parse_bytes(PUBLIC_EXPONENT_HEX.as_bytes(), 16)
        .context("embedded RSA exponent is invalid")?;
    RsaPublicKey::new(modulus, exponent).context("embedded RSA public key is invalid")
}

fn rsa_bootstrap_body(key: &[u8; AES_KEY_LEN]) -> Result<String> {
    let encoded_key = STANDARD.encode(key);
    let password = format!(
        "password={}",
        utf8_percent_encode(&encoded_key, NON_ALPHANUMERIC)
    );
    let encrypted = public_key()?
        .encrypt(&mut OsRng, Pkcs1v15Encrypt, password.as_bytes())
        .context("failed to RSA-encrypt transaction key")?;
    Ok(STANDARD.encode(encrypted))
}

fn encrypt_bytes(key: &[u8; AES_KEY_LEN], iv: &[u8; AES_KEY_LEN], plaintext: &[u8]) -> String {
    let ciphertext =
        Aes128CbcEncryptor::new(key.into(), iv.into()).encrypt_padded_vec_mut::<Pkcs7>(plaintext);
    let mut framed = Vec::with_capacity(AES_KEY_LEN + ciphertext.len());
    framed.extend_from_slice(iv);
    framed.extend_from_slice(&ciphertext);
    STANDARD.encode(framed)
}

fn decrypt_bytes(key: &[u8; AES_KEY_LEN], body: &str) -> Result<Vec<u8>> {
    let framed = STANDARD
        .decode(body)
        .context("transaction response was not valid Base64")?;
    if framed.len() <= AES_KEY_LEN {
        bail!("transaction response did not include an IV and ciphertext");
    }
    let (iv, ciphertext) = framed.split_at(AES_KEY_LEN);
    Aes128CbcDecryptor::new(key.into(), iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|_| anyhow!("transaction response had invalid AES-CBC padding"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encrypts_iv_before_ciphertext_and_round_trips() {
        let key = [0x11; AES_KEY_LEN];
        let iv = [0x22; AES_KEY_LEN];
        let plaintext = br#"{"stake":10000,"odds":"2.10"}"#;
        let body = encrypt_bytes(&key, &iv, plaintext);
        let framed = STANDARD.decode(&body).unwrap();
        assert_eq!(&framed[..AES_KEY_LEN], &iv);
        assert_eq!(decrypt_bytes(&key, &body).unwrap(), plaintext);
    }

    #[test]
    fn decrypts_json_payload() {
        let key = [0x44; AES_KEY_LEN];
        let iv = [0x55; AES_KEY_LEN];
        let expected = json!({"bizCode": 10000, "data": {"orderId": "123"}});
        let body = encrypt_bytes(&key, &iv, &serde_json::to_vec(&expected).unwrap());
        let cipher = TransactionCipher {
            key,
            trans_id: "test".to_owned(),
        };
        let decoded: serde_json::Value = cipher.decrypt_json(&body).unwrap();
        assert_eq!(decoded, expected);
    }

    #[test]
    fn bootstrap_body_is_one_rsa_block() {
        let body = rsa_bootstrap_body(&[0x01; AES_KEY_LEN]).unwrap();
        assert_eq!(STANDARD.decode(body).unwrap().len(), 128);
    }

    #[test]
    fn debug_output_redacts_key_and_transaction_id() {
        let cipher = TransactionCipher {
            key: [0xAA; AES_KEY_LEN],
            trans_id: "private-transaction".to_owned(),
        };
        let output = format!("{cipher:?}");
        assert!(!output.contains("private-transaction"));
        assert!(output.contains("<redacted>"));
    }
}
