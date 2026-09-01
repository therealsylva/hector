use std::{fmt, str::FromStr};

use anyhow::{Result, bail};
use serde_json::Value;

use crate::{cli::MarketCommand, client::SportyClient};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryParam {
    key: String,
    value: String,
}

impl QueryParam {
    #[must_use]
    pub fn as_pair(&self) -> (&str, &str) {
        (&self.key, &self.value)
    }
}

impl FromStr for QueryParam {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((key, value)) = value.split_once('=') else {
            bail!("query parameters must use KEY=VALUE syntax");
        };
        let key = key.trim();
        if key.is_empty() {
            bail!("query parameter key cannot be empty");
        }
        if key.contains(['\r', '\n']) || value.contains(['\r', '\n']) {
            bail!("query parameters cannot contain newlines");
        }
        Ok(Self {
            key: key.to_owned(),
            value: value.to_owned(),
        })
    }
}

impl fmt::Display for QueryParam {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}={}", self.key, self.value)
    }
}

/// Fetches a public market-data resource without requiring a session cookie.
///
/// The parameters are intentionally passed through because `SportyBet` varies the
/// accepted filters across regions and frontend releases.
///
/// # Errors
///
/// Returns an error for invalid URLs, transport failures, rejected requests, or
/// responses that are not JSON.
pub async fn fetch(client: &SportyClient, command: &MarketCommand) -> Result<Value> {
    let path = match command {
        MarketCommand::Sports(_) => "factsCenter/sportList",
        MarketCommand::Events(_) => "factsCenter/liveOrPrematchEvents",
        MarketCommand::Event(_) => "factsCenter/event",
        MarketCommand::MarketGroups(_) => "factsCenter/marketGroups",
        MarketCommand::Outcomes(_) => "factsCenter/Outcomes",
    };
    let query: Vec<_> = command.query().iter().map(QueryParam::as_pair).collect();
    client.get_with_query(path, &query).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_query_value_at_first_equals_sign() {
        let parameter: QueryParam = "specifier=total=2.5".parse().unwrap();
        assert_eq!(parameter.as_pair(), ("specifier", "total=2.5"));
        assert_eq!(parameter.to_string(), "specifier=total=2.5");
    }

    #[test]
    fn rejects_missing_or_empty_keys() {
        assert!("eventId".parse::<QueryParam>().is_err());
        assert!("=sr:match:1".parse::<QueryParam>().is_err());
    }

    #[test]
    fn rejects_newline_injection() {
        assert!("eventId=ok\r\nHeader:value".parse::<QueryParam>().is_err());
    }
}
