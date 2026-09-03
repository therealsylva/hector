pub mod banner;
pub mod live;

use std::io::{self, IsTerminal};

use comfy_table::{
    Attribute, Cell, CellAlignment, ContentArrangement, Table, presets::NOTHING,
};
use console::{Style, Term};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

use crate::{
    VERSION,
    cli::ColorPolicy,
    journal::{JournalRecord, JournalState},
    orders::ExecutionOutcome,
    session::SessionStatus,
};

const MAX_COLUMNS: usize = 8;
const MAX_ROWS: usize = 100;

#[derive(Clone, Copy, Debug)]
pub struct Ui {
    rich: bool,
    color: bool,
}

impl Ui {
    #[must_use]
    pub fn new(policy: ColorPolicy, plain: bool, json: bool, interactive: bool) -> Self {
        let terminal = io::stdout().is_terminal();
        let color = !json
            && !plain
            && std::env::var_os("NO_COLOR").is_none()
            && match policy {
                ColorPolicy::Auto => terminal,
                ColorPolicy::Always => true,
                ColorPolicy::Never => false,
            };
        console::set_colors_enabled(color);
        console::set_colors_enabled_stderr(color);
        Self {
            rich: !json && !plain && (terminal || interactive),
            color,
        }
    }

    #[must_use]
    pub const fn is_rich(self) -> bool {
        self.rich
    }

    #[must_use]
    pub const fn has_color(self) -> bool {
        self.color
    }

    pub fn banner(self) {
        let width = Term::stdout().size().1;
        let art = if width >= 44 {
            banner::FULL
        } else {
            banner::COMPACT
        };
        if self.color {
            println!("{}", Style::new().cyan().bold().apply_to(art));
        } else {
            println!("{art}");
        }
        println!(
            "{}  {}  {}",
            Style::new().bold().apply_to(format!("v{VERSION}")),
            Style::new().dim().apply_to("INTERACTIVE TERMINAL"),
            Style::new().yellow().bold().apply_to("DRY RUN DEFAULT")
        );
    }

    pub fn startup_status(self, status: Result<&SessionStatus, &str>, journal_records: usize) {
        let (session, balance) = match status {
            Ok(status) => (
                Style::new().green().bold().apply_to("● AUTHENTICATED").to_string(),
                format!("{} {}", status.currency, status.available_balance),
            ),
            Err(message) => (
                Style::new().yellow().bold().apply_to("○ PUBLIC MODE").to_string(),
                Style::new().dim().apply_to(message).to_string(),
            ),
        };
        println!("\n{session}  {balance}");
        println!(
            "{} journal records  {}",
            Style::new().bold().apply_to(journal_records),
            Style::new()
                .dim()
                .apply_to("help · Ctrl-L clear · Ctrl-D exit")
        );
    }

    pub fn section(self, title: &str) {
        if self.rich {
            println!(
                "\n{} {}",
                Style::new().cyan().bold().apply_to("▰"),
                Style::new().bold().apply_to(title.to_uppercase())
            );
        }
    }

    pub fn note(self, message: impl std::fmt::Display) {
        println!("{} {message}", Style::new().cyan().bold().apply_to("›"));
    }

    pub fn warning(self, message: impl std::fmt::Display) {
        eprintln!("{} {message}", Style::new().yellow().bold().apply_to("!"));
    }

    pub fn success(self, message: impl std::fmt::Display) {
        println!("{} {message}", Style::new().green().bold().apply_to("✓"));
    }

    pub fn error(self, message: impl std::fmt::Display) {
        eprintln!("{} {message}", Style::new().red().bold().apply_to("×"));
    }

    #[must_use]
    pub fn spinner(self, message: &str) -> Option<ProgressBar> {
        if !self.rich || !io::stderr().is_terminal() {
            return None;
        }
        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::with_template("{spinner:.cyan.bold} {msg}")
                .expect("valid static spinner template")
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        spinner.set_message(message.to_owned());
        spinner.enable_steady_tick(std::time::Duration::from_millis(80));
        Some(spinner)
    }

    pub fn render_session(self, status: &SessionStatus) {
        if !self.rich {
            println!("session: authenticated");
            println!("balance: {} {}", status.currency, status.available_balance);
            if status.available_coins != "0.0000" {
                println!("coins: {}", status.available_coins);
            }
            return;
        }
        self.section("Account");
        let rows = vec![
            vec!["SESSION".to_owned(), "AUTHENTICATED".to_owned()],
            vec![
                "BALANCE".to_owned(),
                format!("{} {}", status.currency, status.available_balance),
            ],
            vec!["COINS".to_owned(), status.available_coins.clone()],
        ];
        println!("{}", table(&["", ""], &rows));
    }

    pub fn render_topic(self, topic: &str) {
        if self.rich {
            self.section("Realtime topic");
            println!("{}", Style::new().cyan().apply_to(topic));
            self.note("Run `watch <topic>` here to open the live monitor.");
        } else {
            println!("{topic}");
        }
    }

    pub fn render_value(self, title: &str, value: &Value) {
        if !self.rich {
            println!(
                "{}",
                serde_json::to_string_pretty(value).expect("JSON value always serializes")
            );
            return;
        }
        self.section(title);
        let value = envelope_data(value);
        if let Some(items) = best_object_array(value) {
            render_object_array(items);
        } else if let Some(object) = value.as_object() {
            let rows = object
                .iter()
                .map(|(key, value)| vec![humanize(key), compact_value(value)])
                .collect::<Vec<_>>();
            println!("{}", table(&["FIELD", "VALUE"], &rows));
        } else if let Some(items) = value.as_array() {
            let rows = items
                .iter()
                .enumerate()
                .map(|(index, value)| vec![(index + 1).to_string(), compact_value(value)])
                .collect::<Vec<_>>();
            println!("{}", table(&["#", "VALUE"], &rows));
        } else {
            println!("{}", compact_value(value));
        }
    }

    pub fn render_orders(self, records: &[JournalRecord]) {
        if !self.rich {
            println!(
                "{}",
                serde_json::to_string_pretty(records).expect("journal always serializes")
            );
            return;
        }
        self.section("Order journal");
        if records.is_empty() {
            self.note("No journaled order attempts yet.");
            return;
        }
        let rows = records
            .iter()
            .map(|record| {
                vec![
                    record.attempt_id.chars().take(8).collect(),
                    record.timestamp_ms.to_string(),
                    state_label(record.state),
                    compact_value(&record.details),
                ]
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            table(&["ATTEMPT", "TIME (MS)", "STATE", "DETAILS"], &rows)
        );
    }

    pub fn render_execution(self, outcome: &ExecutionOutcome) {
        let value = serde_json::to_value(outcome).expect("execution outcome always serializes");
        match outcome {
            ExecutionOutcome::DryRun { .. } => {
                self.warning("Dry run only — no wager was submitted.");
                self.render_value("Order preview", &value);
            }
            ExecutionOutcome::Confirmed { attempt_id, .. } => {
                self.success(format!("Order confirmed · attempt {attempt_id}"));
                self.render_value("Execution result", &value);
            }
        }
    }
}

fn envelope_data(value: &Value) -> &Value {
    if value.get("bizCode").is_some() {
        value.get("data").unwrap_or(value)
    } else {
        value
    }
}

fn best_object_array(value: &Value) -> Option<&[Value]> {
    fn visit<'a>(value: &'a Value, best: &mut Option<&'a [Value]>, depth: usize) {
        if depth > 7 {
            return;
        }
        match value {
            Value::Array(items) => {
                if items.iter().any(Value::is_object)
                    && best.is_none_or(|current| items.len() > current.len())
                {
                    *best = Some(items);
                }
                for item in items {
                    visit(item, best, depth + 1);
                }
            }
            Value::Object(object) => {
                for child in object.values() {
                    visit(child, best, depth + 1);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    let mut best = None;
    visit(value, &mut best, 0);
    best
}

fn render_object_array(items: &[Value]) {
    let priority = [
        "eventId",
        "sportName",
        "name",
        "homeTeamName",
        "awayTeamName",
        "status",
        "marketId",
        "outcomeId",
        "odds",
        "probability",
        "startTime",
    ];
    let mut keys = Vec::<String>::new();
    for key in priority {
        if items.iter().any(|item| item.get(key).is_some()) {
            keys.push(key.to_owned());
        }
    }
    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        for (key, value) in object {
            if keys.len() >= MAX_COLUMNS {
                break;
            }
            if is_scalar(value) && !keys.iter().any(|existing| existing == key) {
                keys.push(key.clone());
            }
        }
    }
    keys.truncate(MAX_COLUMNS);
    if keys.is_empty() {
        println!("{}", compact_value(&Value::Array(items.to_vec())));
        return;
    }
    let rows = items
        .iter()
        .filter_map(Value::as_object)
        .take(MAX_ROWS)
        .map(|object| {
            keys.iter()
                .map(|key| object.get(key).map_or_else(String::new, compact_value))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let headers = keys.iter().map(|key| humanize(key)).collect::<Vec<_>>();
    let header_refs = headers.iter().map(String::as_str).collect::<Vec<_>>();
    println!("{}", table(&header_refs, &rows));
    if items.len() > MAX_ROWS {
        println!("… {} more rows", items.len() - MAX_ROWS);
    }
}

fn is_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

fn compact_value(value: &Value) -> String {
    let raw = match value {
        Value::Null => "—".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => format!("[{} items]", values.len()),
        Value::Object(object) => {
            for key in ["name", "title", "description", "message"] {
                if let Some(Value::String(value)) = object.get(key) {
                    return truncate(value, 72);
                }
            }
            serde_json::to_string(value).unwrap_or_else(|_| "{…}".to_owned())
        }
    };
    truncate(&raw, 72)
}

fn truncate(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_owned();
    }
    let mut output = String::new();
    for character in value.chars() {
        if UnicodeWidthStr::width(output.as_str()) + character.len_utf8() + 1 > width {
            break;
        }
        output.push(character);
    }
    output.push('…');
    output
}

fn humanize(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if character.is_uppercase() && !output.is_empty() {
            output.push(' ');
        }
        output.push(character.to_ascii_uppercase());
    }
    output
}

fn state_label(state: JournalState) -> String {
    match state {
        JournalState::Pending => "PENDING",
        JournalState::Confirmed => "CONFIRMED",
        JournalState::Rejected => "REJECTED",
        JournalState::Ambiguous => "AMBIGUOUS",
    }
    .to_owned()
}

fn table(headers: &[&str], rows: &[Vec<String>]) -> Table {
    let mut table = Table::new();
    table
        .load_preset(NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers.iter().map(|header| {
            Cell::new(*header)
                .add_attribute(Attribute::Bold)
                .set_alignment(CellAlignment::Left)
        }));
    for row in rows {
        table.add_row(row.iter().map(|value| Cell::new(value.as_str())));
    }
    table
}

pub fn print_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supplied_banner_is_preserved() {
        assert!(banner::FULL.starts_with(" _   _"));
        assert!(banner::FULL.contains("\\_| |_/\\___|"));
        assert_eq!(banner::FULL.lines().count(), 6);
    }

    #[test]
    fn finds_the_most_useful_nested_table() {
        let value = serde_json::json!({
            "data": {"groups": [{"name": "small"}], "events": [
                {"eventId": "1", "homeTeamName": "A"},
                {"eventId": "2", "homeTeamName": "B"}
            ]}
        });
        let result = best_object_array(&value).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn truncates_wide_values() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
    }
}
