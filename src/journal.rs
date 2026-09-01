use std::{
    collections::VecDeque,
    env,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

const JOURNAL_ENV: &str = "HECTOR_JOURNAL_PATH";

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalState {
    Pending,
    Confirmed,
    Rejected,
    Ambiguous,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct JournalRecord {
    pub attempt_id: String,
    pub timestamp_ms: u64,
    pub state: JournalState,
    pub details: Value,
}

#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
}

impl Journal {
    /// Resolves the journal from `HECTOR_JOURNAL_PATH` or the user state directory.
    ///
    /// # Errors
    ///
    /// Returns an error if neither a configured path nor a usable process directory
    /// can be resolved.
    pub fn from_env() -> Result<Self> {
        if let Some(path) = env::var_os(JOURNAL_ENV).filter(|value| !value.is_empty()) {
            return Ok(Self::at(path));
        }
        if let Some(state) = env::var_os("XDG_STATE_HOME").filter(|value| !value.is_empty()) {
            return Ok(Self::at(PathBuf::from(state).join("hector/orders.jsonl")));
        }
        if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
            return Ok(Self::at(
                PathBuf::from(home).join(".local/state/hector/orders.jsonl"),
            ));
        }
        Ok(Self::at(
            env::current_dir()
                .context("failed to resolve a fallback journal directory")?
                .join(".hector/orders.jsonl"),
        ))
    }

    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Starts a durable order attempt before the network submission occurs.
    ///
    /// # Errors
    ///
    /// Returns an error if the request cannot be serialized or durably appended.
    pub fn begin<T: Serialize>(&self, request: &T) -> Result<String> {
        let attempt_id = Uuid::new_v4().to_string();
        self.append(&JournalRecord {
            attempt_id: attempt_id.clone(),
            timestamp_ms: timestamp_ms(),
            state: JournalState::Pending,
            details: serde_json::json!({"request": request}),
        })?;
        Ok(attempt_id)
    }

    /// Appends a state transition for an existing attempt.
    ///
    /// # Errors
    ///
    /// Returns an error if the transition cannot be durably appended.
    pub fn transition(&self, attempt_id: &str, state: JournalState, details: Value) -> Result<()> {
        self.append(&JournalRecord {
            attempt_id: attempt_id.to_owned(),
            timestamp_ms: timestamp_ms(),
            state,
            details,
        })
    }

    /// Reads the newest journal records while preserving chronological order.
    ///
    /// # Errors
    ///
    /// Returns an error if the journal cannot be read or contains an invalid line.
    pub fn load(&self, limit: usize) -> Result<Vec<JournalRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = File::open(&self.path)
            .with_context(|| format!("failed to open journal {}", self.path.display()))?;
        let mut records = VecDeque::new();
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line =
                line.with_context(|| format!("failed to read journal line {}", index + 1))?;
            let record = serde_json::from_str(&line)
                .with_context(|| format!("invalid journal line {}", index + 1))?;
            records.push_back(record);
            if limit != 0 && records.len() > limit {
                records.pop_front();
            }
        }
        Ok(records.into())
    }

    fn append(&self, record: &JournalRecord) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create journal directory {}", parent.display())
            })?;
        }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&self.path)
            .with_context(|| format!("failed to open journal {}", self.path.display()))?;
        let mut line = serde_json::to_vec(record)?;
        line.push(b'\n');
        file.write_all(&line)
            .context("failed to append journal record")?;
        file.sync_data().context("failed to sync journal record")
    }
}

fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_transitions_and_limits_reads() {
        let directory = env::temp_dir().join(format!("hector-journal-{}", Uuid::new_v4()));
        let path = directory.join("orders.jsonl");
        let journal = Journal::at(&path);
        let attempt = journal
            .begin(&serde_json::json!({"stake": 10_000}))
            .unwrap();
        journal
            .transition(
                &attempt,
                JournalState::Confirmed,
                serde_json::json!({"orderId": "42"}),
            )
            .unwrap();

        let all = journal.load(0).unwrap();
        assert_eq!(all.len(), 2);
        assert!(matches!(all[0].state, JournalState::Pending));
        let newest = journal.load(1).unwrap();
        assert_eq!(newest.len(), 1);
        assert!(matches!(newest[0].state, JournalState::Confirmed));

        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
