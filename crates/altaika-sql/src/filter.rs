use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FilterError {
    #[error(
        "unsupported filter `{0}`; use column:=value, column:>value, column:>=value, column:<value, or column:<=value"
    )]
    Unsupported(String),
    #[error("filter column cannot be empty")]
    EmptyColumn,
    #[error("invalid identifier `{0}`")]
    InvalidIdentifier(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate {
    Compare {
        column: String,
        op: ComparisonOp,
        value: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    Eq,
    Gt,
    Gte,
    Lt,
    Lte,
}

pub fn parse_filter(input: &str) -> Result<Predicate, FilterError> {
    let (column, op, value) = parse_comparison(input)?;

    let column = column.trim();
    if column.is_empty() {
        return Err(FilterError::EmptyColumn);
    }

    validate_identifier(column)?;

    Ok(Predicate::Compare {
        column: column.to_string(),
        op,
        value: value.trim().to_string(),
    })
}

pub fn render_predicate(predicate: &Predicate) -> Result<String, FilterError> {
    match predicate {
        Predicate::Compare { column, op, value } => {
            let column = render_identifier(column)?;
            Ok(format!(
                "{column} {} {}",
                op.as_sql(),
                render_literal(value)
            ))
        }
    }
}

fn parse_comparison(input: &str) -> Result<(&str, ComparisonOp, &str), FilterError> {
    for (delimiter, op) in [
        (":>=", ComparisonOp::Gte),
        (":<=", ComparisonOp::Lte),
        (":>", ComparisonOp::Gt),
        (":<", ComparisonOp::Lt),
        (":=", ComparisonOp::Eq),
    ] {
        if let Some((column, value)) = input.split_once(delimiter) {
            return Ok((column, op, value));
        }
    }
    Err(FilterError::Unsupported(input.to_string()))
}

impl ComparisonOp {
    fn as_sql(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::Lt => "<",
            Self::Lte => "<=",
        }
    }
}

fn render_literal(value: &str) -> String {
    let value = value.trim();
    if value.parse::<i64>().is_ok() || value.parse::<f64>().is_ok() {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

pub fn render_identifier(input: &str) -> Result<String, FilterError> {
    let input = input.trim();
    validate_identifier(input)?;
    Ok(input.to_string())
}

fn validate_identifier(input: &str) -> Result<(), FilterError> {
    let mut chars = input.chars();
    let Some(first) = chars.next() else {
        return Err(FilterError::EmptyColumn);
    };

    if !(first == '_' || first.is_ascii_alphabetic()) {
        return Err(FilterError::InvalidIdentifier(input.to_string()));
    }

    if chars.any(|char| !(char == '_' || char.is_ascii_alphanumeric())) {
        return Err(FilterError::InvalidIdentifier(input.to_string()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_eq_filter() {
        let predicate = parse_filter("name:=Ada").unwrap();
        assert_eq!(render_predicate(&predicate).unwrap(), "name = 'Ada'");
    }

    #[test]
    fn renders_numeric_comparison_filter() {
        let predicate = parse_filter("age:>=42").unwrap();
        assert_eq!(render_predicate(&predicate).unwrap(), "age >= 42");
    }

    #[test]
    fn escapes_quotes_in_filter_value() {
        let predicate = parse_filter("name:=O'Malley").unwrap();
        assert_eq!(render_predicate(&predicate).unwrap(), "name = 'O''Malley'");
    }

    #[test]
    fn rejects_invalid_identifier() {
        assert_eq!(
            parse_filter("name;drop:=Ada").unwrap_err(),
            FilterError::InvalidIdentifier("name;drop".to_string())
        );
    }
}
