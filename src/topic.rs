use anyhow::{Result, bail};

use crate::cli::{EventTopicArgs, MarketTopicArgs, TopicCommand};

/// Builds an upstream caret-separated subscription topic.
///
/// # Errors
///
/// Returns an error when a field is empty, contains a caret, or contains a
/// control character.
pub fn build(command: &TopicCommand) -> Result<String> {
    match command {
        TopicCommand::Event(args) => event(args),
        TopicCommand::Market(args) => market(args),
    }
}

fn event(args: &EventTopicArgs) -> Result<String> {
    Ok([
        field("sport-id", &args.sport_id)?,
        field("category-id", &args.category_id)?,
        field("tournament-id", &args.tournament_id)?,
        field("event-id", &args.event_id)?,
    ]
    .join("^"))
}

fn market(args: &MarketTopicArgs) -> Result<String> {
    Ok([
        field("sport-id", &args.event.sport_id)?,
        field("category-id", &args.event.category_id)?,
        field("tournament-id", &args.event.tournament_id)?,
        field("event-id", &args.event.event_id)?,
        field("product-id", &args.product_id)?,
        field("market-id", &args.market_id)?,
        field("specifier", &args.specifier)?,
    ]
    .join("^"))
}

fn field<'a>(name: &str, value: &'a str) -> Result<&'a str> {
    if value.is_empty() || value.contains('^') || value.chars().any(char::is_control) {
        bail!("--{name} cannot be empty or contain carets/control characters");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_args() -> EventTopicArgs {
        EventTopicArgs {
            sport_id: "1".to_owned(),
            category_id: "1".to_owned(),
            tournament_id: "sr:tournament:17".to_owned(),
            event_id: "sr:match:42".to_owned(),
        }
    }

    #[test]
    fn builds_event_topic() {
        assert_eq!(
            event(&event_args()).unwrap(),
            "1^1^sr:tournament:17^sr:match:42"
        );
    }

    #[test]
    fn builds_market_topic_with_specifier() {
        let args = MarketTopicArgs {
            event: event_args(),
            product_id: "3".to_owned(),
            market_id: "18".to_owned(),
            specifier: "total=2.5".to_owned(),
        };
        assert_eq!(
            market(&args).unwrap(),
            "1^1^sr:tournament:17^sr:match:42^3^18^total=2.5"
        );
    }

    #[test]
    fn rejects_topic_delimiter_in_fields() {
        let mut args = event_args();
        args.event_id = "bad^topic".to_owned();
        assert!(event(&args).is_err());
    }
}
