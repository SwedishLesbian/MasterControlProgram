use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use std::path::PathBuf;
use tracing_subscriber::{fmt, EnvFilter};

use crate::config::mcp_home;

/// Initialize tracing with console + file output.
pub fn init_logging() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    Ok(())
}

/// Get the log directory.
pub fn log_dir() -> PathBuf {
    mcp_home().join("logs")
}

/// Read agent log by ID. Returns the most recent log for that agent.
pub fn read_agent_log(id: u64) -> Result<Option<String>> {
    let dir = log_dir();
    if !dir.exists() {
        return Ok(None);
    }

    let prefix = format!("agent.{id}-");
    let mut matching: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(&prefix)
        })
        .collect();

    matching.sort_by_key(|e| e.file_name());

    if let Some(entry) = matching.last() {
        let content = std::fs::read_to_string(entry.path())?;
        Ok(Some(content))
    } else {
        Ok(None)
    }
}

/// Read agent logs since a given duration string (e.g., "10m", "1h").
pub fn read_logs_since(since: &str) -> Result<Vec<Value>> {
    let dir = log_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }

    let duration_secs = parse_duration_str(since)?;
    let cutoff = Utc::now() - chrono::Duration::seconds(duration_secs as i64);

    let mut results = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "log") {
            let metadata = std::fs::metadata(&path)?;
            if let Ok(modified) = metadata.modified() {
                let modified: chrono::DateTime<Utc> = modified.into();
                if modified >= cutoff {
                    let content = std::fs::read_to_string(&path)?;
                    if let Ok(val) = serde_json::from_str::<Value>(&content) {
                        results.push(val);
                    }
                }
            }
        }
    }

    Ok(results)
}

fn parse_duration_str(s: &str) -> Result<u64> {
    let s = s.trim();
    if let Some(mins) = s.strip_suffix('m') {
        Ok(mins.parse::<u64>()? * 60)
    } else if let Some(hours) = s.strip_suffix('h') {
        Ok(hours.parse::<u64>()? * 3600)
    } else if let Some(secs) = s.strip_suffix('s') {
        Ok(secs.parse::<u64>()?)
    } else {
        // Default to seconds
        Ok(s.parse::<u64>()?)
    }
}
