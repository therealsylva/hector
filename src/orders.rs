use std::{fmt, str::FromStr};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;

use crate::{
    cli::SingleBetArgs,
    client::SportyClient,
    crypto::TransactionCipher,
    journal::{Journal, JournalState},
    session::{ApiEnvelope, SUCCESS_BIZ_CODE},
};

const MONEY_SCALE: u64 = 10_000;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ScaledAmount(u64);

impl ScaledAmount {
    #[must_use]
    pub const fn minor_units(self) -> u64 {
        self.0
    }
}

impl FromStr for ScaledAmount {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() || input.starts_with(['+', '-']) {
            bail!("amount must be a positive decimal");
        }
        let mut parts = input.split('.');
        let whole = parts.next().unwrap_or_default();
        let fraction = parts.next().unwrap_or_default();
        if parts.next().is_some()
            || whole.is_empty()
            || fraction.len() > 4
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            bail!("amount must have at most four decimal places");
        }
        let major = whole.parse::<u64>().context("amount is too large")?;
        let padded_fraction = format!("{fraction:0<4}");
        let minor = if padded_fraction.is_empty() {
            0
        } else {
            padded_fraction
                .parse::<u64>()
                .context("invalid amount fraction")?
        };
        let value = major
            .checked_mul(MONEY_SCALE)
            .and_then(|value| value.checked_add(minor))
            .context("amount is too large")?;
        if value == 0 {
            bail!("amount must be greater than zero");
        }
        Ok(Self(value))
    }
}

impl fmt::Display for ScaledAmount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}.{:04}",
            self.0 / MONEY_SCALE,
            self.0 % MONEY_SCALE
        )
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderRequest {
    biz_type: u8,
    ticket: Ticket,
    order_type: u8,
    payment_type: u32,
    is_bonus_factor: bool,
    sub_biz_type: u8,
    actual_pay_amount: u64,
}

#[derive(Debug, Serialize)]
struct Ticket {
    selections: Vec<Selection>,
    bets: Vec<Bet>,
}

#[derive(Debug, Serialize)]
struct Selection {
    #[serde(rename = "eventId")]
    event_id: String,
    id: String,
    odds: String,
    banker: bool,
    probability: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Bet {
    selected_systems: Vec<u8>,
    stake: Stake,
}

#[derive(Debug, Serialize)]
struct Stake {
    value: u64,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ExecutionOutcome {
    DryRun {
        request: OrderRequest,
    },
    Confirmed {
        attempt_id: String,
        response: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        journal_warning: Option<String>,
    },
}

/// Builds a single-selection request and, when explicitly armed, submits it once.
///
/// # Errors
///
/// Returns an error for invalid identifiers, missing execution guards, session or
/// cipher failures, an ambiguous transport result, or an upstream rejection.
pub async fn single(client: &SportyClient, args: &SingleBetArgs) -> Result<ExecutionOutcome> {
    let request = build_single(args)?;
    if !args.execute {
        return Ok(ExecutionOutcome::DryRun { request });
    }
    if !args.confirm_order {
        bail!("execution requires both --execute and --confirm-order");
    }
    let max_stake = args
        .max_stake
        .context("execution requires --max-stake as a hard loss ceiling")?;
    if args.stake > max_stake {
        bail!(
            "stake {} exceeds configured --max-stake {}",
            args.stake,
            max_stake
        );
    }
    client.settings().require_account_cookie()?;
    let cipher = TransactionCipher::bootstrap(client).await?;
    let body = cipher.encrypt_json(&request)?;
    let journal = Journal::from_env()?;
    let attempt_id = journal.begin(&request)?;
    let response = match client
        .post_ciphertext("orders/order", cipher.trans_id(), body)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            let journal_error = journal
                .transition(
                    &attempt_id,
                    JournalState::Ambiguous,
                    serde_json::json!({"error": error.to_string()}),
                )
                .err();
            let suffix = journal_error.map_or_else(String::new, |journal_error| {
                format!("; journal update also failed: {journal_error}")
            });
            bail!(
                "order attempt {attempt_id} may be ambiguous; do not retry automatically—check Bet History first: {error}{suffix}"
            );
        }
    };

    if response.trim_start().starts_with('{') {
        let error: Value = serde_json::from_str(&response)
            .context("protected endpoint returned an invalid plaintext error")?;
        let _ = journal.transition(
            &attempt_id,
            JournalState::Rejected,
            serde_json::json!({"response": &error}),
        );
        bail!("order was rejected before decryption: {error}");
    }

    let envelope: ApiEnvelope<Value> = match cipher.decrypt_json(&response) {
        Ok(envelope) => envelope,
        Err(error) => {
            let _ = journal.transition(
                &attempt_id,
                JournalState::Ambiguous,
                serde_json::json!({"error": error.to_string()}),
            );
            bail!(
                "order attempt {attempt_id} returned an undecodable response and is ambiguous; check Bet History before retrying: {error}"
            );
        }
    };
    if envelope.biz_code != SUCCESS_BIZ_CODE {
        let _ = journal.transition(
            &attempt_id,
            JournalState::Rejected,
            serde_json::json!({"bizCode": envelope.biz_code, "message": &envelope.message}),
        );
        bail!(
            "order rejected with bizCode {}: {}",
            envelope.biz_code,
            envelope.message
        );
    }
    let response = envelope.data.unwrap_or(Value::Null);
    let journal_warning = journal
        .transition(
            &attempt_id,
            JournalState::Confirmed,
            serde_json::json!({"response": &response}),
        )
        .err()
        .map(|error| error.to_string());
    Ok(ExecutionOutcome::Confirmed {
        attempt_id,
        response,
        journal_warning,
    })
}

fn build_single(args: &SingleBetArgs) -> Result<OrderRequest> {
    for (name, value) in [
        ("event-id", args.event_id.as_str()),
        ("sport-id", args.sport_id.as_str()),
        ("market-id", args.market_id.as_str()),
        ("outcome-id", args.outcome_id.as_str()),
        ("odds", args.odds.as_str()),
        ("probability", args.probability.as_str()),
    ] {
        validate_value(name, value)?;
    }
    if let Some(specifier) = &args.specifier {
        validate_value("specifier", specifier)?;
    }
    let suffix = args
        .specifier
        .as_ref()
        .map_or_else(String::new, |value| format!("?{value}"));
    let selection_id = format!(
        "uof:{}/{}/{}/{}{}",
        args.product_id, args.sport_id, args.market_id, args.outcome_id, suffix
    );
    let sub_biz_type = match args.product_id {
        1 => 2,
        3 => 1,
        _ => 0,
    };
    Ok(OrderRequest {
        biz_type: 1,
        ticket: Ticket {
            selections: vec![Selection {
                event_id: args.event_id.clone(),
                id: selection_id,
                odds: args.odds.clone(),
                banker: false,
                probability: args.probability.clone(),
            }],
            bets: vec![Bet {
                selected_systems: vec![1],
                stake: Stake {
                    value: args.stake.minor_units(),
                },
            }],
        },
        order_type: 1,
        payment_type: args.payment_type,
        is_bonus_factor: false,
        sub_biz_type,
        actual_pay_amount: args.stake.minor_units(),
    })
}

fn validate_value(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        bail!("--{name} cannot be empty or contain control characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> SingleBetArgs {
        SingleBetArgs {
            event_id: "sr:match:42".to_owned(),
            sport_id: "sr:sport:1".to_owned(),
            product_id: 3,
            market_id: "1".to_owned(),
            outcome_id: "1".to_owned(),
            specifier: None,
            odds: "2.10".to_owned(),
            probability: "0.48".to_owned(),
            stake: "25.50".parse().unwrap(),
            payment_type: 0,
            execute: false,
            confirm_order: false,
            max_stake: None,
        }
    }

    #[test]
    fn amount_parser_is_exact() {
        assert_eq!(
            "25.5".parse::<ScaledAmount>().unwrap().minor_units(),
            255_000
        );
        assert_eq!("0.0001".parse::<ScaledAmount>().unwrap().minor_units(), 1);
        assert!("1.00001".parse::<ScaledAmount>().is_err());
        assert!("-1".parse::<ScaledAmount>().is_err());
        assert!("0".parse::<ScaledAmount>().is_err());
    }

    #[test]
    fn builds_deployed_single_order_shape() {
        let request = build_single(&args()).unwrap();
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["bizType"], 1);
        assert_eq!(value["orderType"], 1);
        assert_eq!(value["subBizType"], 1);
        assert_eq!(value["actualPayAmount"], 255_000);
        assert_eq!(value["ticket"]["bets"][0]["stake"]["value"], 255_000);
        assert_eq!(
            value["ticket"]["selections"][0]["id"],
            "uof:3/sr:sport:1/1/1"
        );
    }

    #[test]
    fn appends_market_specifier() {
        let mut args = args();
        args.specifier = Some("total=2.5".to_owned());
        let value = serde_json::to_value(build_single(&args).unwrap()).unwrap();
        assert_eq!(
            value["ticket"]["selections"][0]["id"],
            "uof:3/sr:sport:1/1/1?total=2.5"
        );
    }
}
