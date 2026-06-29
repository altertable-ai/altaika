use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LakePath {
    pub source: Option<String>,
    pub namespace: Vec<String>,
    pub object: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PathError {
    #[error("path segment cannot be empty")]
    EmptySegment,
}

impl FromStr for LakePath {
    type Err = PathError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let trimmed = input.trim_matches('/');
        if trimmed.is_empty() {
            return Ok(Self {
                source: None,
                namespace: Vec::new(),
                object: None,
            });
        }

        let parts = trimmed.split('/').map(str::trim).collect::<Vec<_>>();
        if parts.iter().any(|part| part.is_empty()) {
            return Err(PathError::EmptySegment);
        }

        let source = parts.first().map(|part| (*part).to_string());
        let object = (parts.len() > 1).then(|| parts[parts.len() - 1].to_string());
        let namespace = if parts.len() > 2 {
            parts[1..parts.len() - 1]
                .iter()
                .map(|part| (*part).to_string())
                .collect()
        } else {
            Vec::new()
        };

        Ok(Self {
            source,
            namespace,
            object,
        })
    }
}

impl LakePath {
    pub fn display(&self) -> String {
        self.source
            .iter()
            .chain(self.namespace.iter())
            .chain(self.object.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_source_namespace_and_object() {
        let path: LakePath = "altertable/analytics/events".parse().unwrap();
        assert_eq!(path.source.as_deref(), Some("altertable"));
        assert_eq!(path.namespace, vec!["analytics"]);
        assert_eq!(path.object.as_deref(), Some("events"));
    }

    #[test]
    fn parses_source_only() {
        let path: LakePath = "local".parse().unwrap();
        assert_eq!(path.source.as_deref(), Some("local"));
        assert!(path.namespace.is_empty());
        assert!(path.object.is_none());
    }

    #[test]
    fn rejects_empty_segment() {
        let err = "altertable//events".parse::<LakePath>().unwrap_err();
        assert_eq!(err, PathError::EmptySegment);
    }
}
