use thiserror::Error;

use crate::envelope::ErrorEnvelope;

#[derive(Debug, Error)]
pub enum AltaikaError {
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),
    #[error("engine error: {0}")]
    Engine(String),
}

impl AltaikaError {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::InvalidArgument(_) => 2,
            Self::UnsupportedOperation(_) => 6,
            Self::Engine(message) if is_not_found(message) => 4,
            Self::Engine(_) => 1,
        }
    }

    pub fn envelope(&self) -> ErrorEnvelope {
        let message = self.to_string();
        ErrorEnvelope {
            error_code: self.error_code().to_string(),
            exit_code: self.exit_code(),
            hint: hint_for(&message),
            message,
        }
    }

    fn error_code(&self) -> &'static str {
        match self {
            Self::InvalidArgument(_) => "invalid_argument",
            Self::UnsupportedOperation(_) => "unsupported_operation",
            Self::Engine(message) if is_not_found(message) => "not_found",
            Self::Engine(_) => "engine_error",
        }
    }
}

fn is_not_found(message: &str) -> bool {
    let message = message.to_lowercase();
    message.contains("no table named")
        || message.contains("no field named")
        || message.contains("table not found")
        || message.contains("column not found")
}

fn hint_for(message: &str) -> Option<String> {
    let message = message.to_lowercase();
    if message.contains("no table named") || message.contains("table not found") {
        Some(
            "Run `altaika ls <source>/<namespace>` to list tables, then `altaika describe <path>` before reading."
                .to_string(),
        )
    } else if message.contains("no field named") || message.contains("column not found") {
        Some("Run `altaika describe <path>` to see the available columns.".to_string())
    } else if message.contains("parsererror")
        || message.contains("sql error")
        || message.contains("syntax error")
    {
        Some(
            "Check the SQL syntax. Use `altaika describe <path>` to confirm column names and types."
                .to_string(),
        )
    } else if message.contains("not implemented") {
        Some(
            "This engine or operation is not available yet. Run `altaika agent schema` for supported engines and commands."
                .to_string(),
        )
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_table_classifies_as_not_found_with_hint() {
        let envelope =
            AltaikaError::Engine("Error during planning: No table named 'missing'".to_string())
                .envelope();
        assert_eq!(envelope.error_code, "not_found");
        assert_eq!(envelope.exit_code, 4);
        assert!(envelope.hint.is_some());
    }

    #[test]
    fn missing_column_classifies_as_not_found() {
        let envelope =
            AltaikaError::Engine("Schema error: No field named nope.".to_string()).envelope();
        assert_eq!(envelope.error_code, "not_found");
        assert_eq!(envelope.exit_code, 4);
    }

    #[test]
    fn generic_engine_error_stays_engine_error_without_hint() {
        let envelope = AltaikaError::Engine("connection reset by peer".to_string()).envelope();
        assert_eq!(envelope.error_code, "engine_error");
        assert_eq!(envelope.exit_code, 1);
        assert!(envelope.hint.is_none());
    }

    #[test]
    fn sql_parse_error_keeps_engine_code_but_adds_hint() {
        let envelope =
            AltaikaError::Engine("SQL error: ParserError(\"Expected\")".to_string()).envelope();
        assert_eq!(envelope.error_code, "engine_error");
        assert!(envelope.hint.is_some());
    }
}
