use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("invalid argument: {message}")]
    InvalidCommandArgument { message: String, mode: &'static str },
    #[error("{source}")]
    CommandMode {
        mode: &'static str,
        source: Box<Self>,
    },
    #[error("{message}")]
    ConnectorSetup {
        message: String,
        mode: &'static str,
        stats: Value,
    },
    #[error("{message}")]
    ApprovalRequired {
        message: String,
        mode: &'static str,
        stats: Value,
    },
    #[error("duckdb error: {0}")]
    Duckdb(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{action} `{path}`: {source}")]
    IoContext {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{action} `{path}`: {source}")]
    JsonContext {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

impl Error {
    #[must_use]
    pub const fn mode(&self) -> &'static str {
        match self {
            Self::InvalidCommandArgument { mode, .. }
            | Self::CommandMode { mode, .. }
            | Self::ConnectorSetup { mode, .. }
            | Self::ApprovalRequired { mode, .. } => mode,
            Self::InvalidArgument(_)
            | Self::Duckdb(_)
            | Self::Io(_)
            | Self::IoContext { .. }
            | Self::Json(_)
            | Self::JsonContext { .. } => "local",
        }
    }

    #[must_use]
    pub const fn skill(&self) -> &'static str {
        match self {
            Self::ApprovalRequired { .. } => "approval_required",
            Self::ConnectorSetup { .. } => "connector_setup",
            _ => "error_report",
        }
    }

    #[must_use]
    pub fn stats(&self) -> Value {
        match self {
            Self::ApprovalRequired { stats, .. } | Self::ConnectorSetup { stats, .. } => {
                stats.clone()
            }
            _ => json!({}),
        }
    }

    #[must_use]
    pub fn with_mode(self, mode: &'static str) -> Self {
        match self {
            Self::InvalidCommandArgument { .. }
            | Self::CommandMode { .. }
            | Self::ApprovalRequired { .. }
            | Self::ConnectorSetup { .. } => self,
            _ => Self::CommandMode {
                mode,
                source: Box::new(self),
            },
        }
    }

    #[must_use]
    pub fn redact_secrets(self, secrets: &[&str]) -> Self {
        match self {
            Self::InvalidArgument(message) => {
                Self::InvalidArgument(redact_secret_text(&message, secrets))
            }
            Self::InvalidCommandArgument { message, mode } => Self::InvalidCommandArgument {
                message: redact_secret_text(&message, secrets),
                mode,
            },
            Self::CommandMode { mode, source } => Self::CommandMode {
                mode,
                source: Box::new(source.redact_secrets(secrets)),
            },
            Self::ConnectorSetup {
                message,
                mode,
                stats,
            } => Self::ConnectorSetup {
                message: redact_secret_text(&message, secrets),
                mode,
                stats,
            },
            Self::ApprovalRequired {
                message,
                mode,
                stats,
            } => Self::ApprovalRequired {
                message: redact_secret_text(&message, secrets),
                mode,
                stats,
            },
            Self::Duckdb(message) => Self::Duckdb(redact_secret_text(&message, secrets)),
            Self::Io(error) => Self::Io(error),
            Self::IoContext {
                action,
                path,
                source,
            } => Self::IoContext {
                action,
                path,
                source,
            },
            Self::Json(error) => Self::Json(error),
            Self::JsonContext {
                action,
                path,
                source,
            } => Self::JsonContext {
                action,
                path,
                source,
            },
        }
    }

    #[must_use]
    pub fn io_context(action: &'static str, path: &Path, source: io::Error) -> Self {
        Self::IoContext {
            action,
            path: path.to_path_buf(),
            source,
        }
    }

    #[must_use]
    pub fn json_context(action: &'static str, path: &Path, source: serde_json::Error) -> Self {
        Self::JsonContext {
            action,
            path: path.to_path_buf(),
            source,
        }
    }
}

fn redact_secret_text(input: &str, secrets: &[&str]) -> String {
    let redacted = secrets.iter().filter(|secret| !secret.is_empty()).fold(
        input.to_owned(),
        |text, secret| {
            let text = text.replace(secret, "[redacted]");
            truncated_secret_prefix(secret).map_or_else(
                || text.clone(),
                |prefix| text.replace(&format!("{prefix}..."), "[redacted]"),
            )
        },
    );
    ["token := ", "auth_token := ", "access_token := "]
        .into_iter()
        .fold(redacted, |text, marker| {
            redact_sql_named_secret(&text, marker)
        })
}

fn truncated_secret_prefix(secret: &str) -> Option<String> {
    let prefix = secret.chars().take(4).collect::<String>();
    (prefix.chars().count() == 4).then_some(prefix)
}

fn redact_sql_named_secret(input: &str, marker: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut remaining = input;
    while let Some(index) = remaining.find(marker) {
        let value_start = index + marker.len();
        output.push_str(&remaining[..value_start]);
        let after_marker = &remaining[value_start..];
        if let Some(after_quote) = after_marker.strip_prefix('\'') {
            output.push_str("'[redacted]'");
            remaining = skip_sql_string_or_line(after_quote);
        } else {
            output.push_str("[redacted]");
            remaining = skip_sql_value_or_line(after_marker);
        }
    }
    output.push_str(remaining);
    output
}

fn skip_sql_string_or_line(input: &str) -> &str {
    let quote_index = input.find('\'');
    let line_index = input.find('\n');
    match (quote_index, line_index) {
        (Some(quote_index), Some(line_index)) if quote_index < line_index => {
            &input[quote_index + 1..]
        }
        (Some(quote_index), None) => &input[quote_index + 1..],
        (_, Some(line_index)) => &input[line_index..],
        (None, None) => "",
    }
}

fn skip_sql_value_or_line(input: &str) -> &str {
    input
        .find([',', ')', '\n'])
        .map_or("", |index| &input[index..])
}
