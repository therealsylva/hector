use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::client::SportyClient;

pub const SUCCESS_BIZ_CODE: i64 = 10_000;
pub const MONEY_SCALE: i64 = 10_000;

#[derive(Debug, Deserialize)]
pub struct ApiEnvelope<T> {
    #[serde(rename = "bizCode")]
    pub biz_code: i64,
    #[serde(default)]
    pub message: String,
    pub data: Option<T>,
}

#[derive(Debug, Default, Deserialize)]
pub struct BalanceData {
    #[serde(rename = "avlBal")]
    pub available_balance: Option<i64>,
    #[serde(rename = "avlCoins")]
    pub available_coins: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SessionStatus {
    pub authenticated: bool,
    pub currency: String,
    pub available_balance: String,
    pub available_coins: String,
}

/// Verifies the imported session through the read-only account-balance endpoint.
///
/// # Errors
///
/// Returns an error when no cookie is configured, the request fails, or `SportyBet` rejects the session.
pub async fn check(client: &SportyClient) -> Result<SessionStatus> {
    client.settings().require_cookie()?;
    let currency = &client.settings().currency;
    let path = format!("pocket/v1/finAccs/finAcc/userBal/{currency}");
    let response: ApiEnvelope<BalanceData> = client.get(&path).await?;

    if response.biz_code != SUCCESS_BIZ_CODE {
        bail!(
            "session check failed with bizCode {}: {}",
            response.biz_code,
            response.message
        );
    }

    let balance = response.data.unwrap_or_default();
    Ok(SessionStatus {
        authenticated: true,
        currency: currency.clone(),
        available_balance: format_scaled(balance.available_balance.unwrap_or_default()),
        available_coins: format_scaled(balance.available_coins.unwrap_or_default()),
    })
}

#[must_use]
pub fn format_scaled(value: i64) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let absolute = value.unsigned_abs();
    let major = absolute / MONEY_SCALE.unsigned_abs();
    let minor = absolute % MONEY_SCALE.unsigned_abs();
    format!("{sign}{major}.{minor:04}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_scaled_money_without_float_rounding() {
        assert_eq!(format_scaled(100_001), "10.0001");
        assert_eq!(format_scaled(-25_000), "-2.5000");
        assert_eq!(format_scaled(0), "0.0000");
    }
}
