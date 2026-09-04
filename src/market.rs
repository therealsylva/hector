use std::{fmt, str::FromStr, time::{SystemTime, UNIX_EPOCH}};

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

    #[must_use]
    fn key(&self) -> &str {
        &self.key
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

fn events_query(command: &MarketCommand) -> Vec<(String, String)> {
    let mut query: Vec<_> = command
        .query()
        .iter()
        .map(|param| {
            let (key, value) = param.as_pair();
            (key.to_owned(), value.to_owned())
        })
        .collect();

    if matches!(command, MarketCommand::Events(_))
        && !command.query().iter().any(|param| param.key() == "_t")
    {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before Unix epoch")
            .as_millis();
        query.push(("_t".to_owned(), timestamp.to_string()));
    }

    query
}

/// Fetches a public market-data resource without requiring a session cookie.
///
/// The parameters are intentionally passed through because `SportyBet` varies the
/// accepted filters across regions and frontend releases. Event-list requests also
/// receive the current Unix timestamp in milliseconds because the live web client
/// sends `_t` and the upstream endpoint currently rejects requests without it.
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
    let query = events_query(command);
    client.get_with_query(path, &query).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::MarketQueryArgs;

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

    #[test]
    fn adds_timestamp_only_to_events() {
        let command = MarketCommand::Events(MarketQueryArgs {
            params: vec!["sportId=sr:sport:1".parse().unwrap()],
        });
        let query = events_query(&command);
        assert_eq!(query[0], ("sportId".to_owned(), "sr:sport:1".to_owned()));
        let timestamp = query
            .iter()
            .find(|(key, _)| key == "_t")
            .map(|(_, value)| value.parse::<u128>().unwrap())
            .unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        assert!(timestamp <= now);
        assert!(now - timestamp < 5_000);
    }

    #[test]
    fn preserves_explicit_timestamp() {
        let command = MarketCommand::Events(MarketQueryArgs {
            params: vec![
                "sportId=sr:sport:1".parse().unwrap(),
                "_t=123456789".parse().unwrap(),
            ],
        });
        let query = events_query(&command);
        assert_eq!(
            query.iter().filter(|(key, _)| key == "_t").count(),
            1
        );
        assert_eq!(
            query.iter().find(|(key, _)| key == "_t").unwrap().1,
            "123456789"
        );
    }

    #[test]
    fn does_not_add_timestamp_to_other_market_commands() {
        let command = MarketCommand::Sports(MarketQueryArgs { params: vec![] });
        let query = events_query(&command);
        assert!(query.is_empty());
    }
}
