use altaika_foundation::error::AltaikaError;

#[cfg(all(
    feature = "duckdb-runtime",
    not(any(target_os = "wasi", target_family = "wasm"))
))]
mod enabled {
    use std::time::Instant;

    use duckdb::Connection;
    use duckdb::types::{OrderedMap, TimeUnit, Value as DuckdbValue, ValueRef};
    use serde_json::{Map, Number, Value, json};
    use sqlparser::dialect::DuckDbDialect;
    use sqlparser::tokenizer::{Location, Token, Tokenizer};

    use super::AltaikaError;

    pub struct QueryResult {
        pub rows: Vec<Value>,
        pub elapsed_ms: u128,
        pub stats: Value,
    }

    pub fn execute_sql(statement: &str, limit: usize) -> Result<QueryResult, AltaikaError> {
        let started_at = Instant::now();
        let conn = open_connection()?;
        let statements = split_statements(statement)?;
        let (query, setup) = statements
            .split_last()
            .ok_or_else(|| AltaikaError::InvalidArgument("SQL statement is empty".to_string()))?;

        if !setup.is_empty() {
            conn.execute_batch(&setup.join(";\n"))
                .map_err(duckdb_error)?;
        }

        let query = bounded_query(query, limit);
        let rows = query_json_rows(&conn, &query)?;
        let elapsed_ms = started_at.elapsed().as_millis();
        let stats = json!({
            "statement_count": setup.len() + 1,
            "column_count": rows.first().and_then(Value::as_object).map_or(0, Map::len),
        });

        Ok(QueryResult {
            rows,
            elapsed_ms,
            stats,
        })
    }

    pub fn auth_status(check: bool) -> Value {
        let version = duckdb_version().unwrap_or_default();
        let quack_extension = duckdb_has_extension("quack");
        let mut missing = Vec::new();
        if version.is_empty() {
            missing.push("duckdb_runtime");
        }
        if !quack_extension {
            missing.push("quack_extension");
        }

        json!({
            "kind": "auth",
            "engine": "duckdb-beta",
            "schema_version": "1.0",
            "data": {
                "ready": missing.is_empty(),
                "checked": check,
                "connection": if missing.is_empty() { "ok" } else { "failed" },
                "runtime_only": true,
                "missing": missing,
                "present": {
                    "ALTAIKA_DUCKDB_BIN": super::env_present("ALTAIKA_DUCKDB_BIN"),
                    "duckdb_binary": Value::Null,
                    "duckdb_runtime": true,
                    "duckdb_version": version,
                    "quack_extension": quack_extension,
                },
            },
        })
    }

    fn open_connection() -> Result<Connection, AltaikaError> {
        Connection::open_in_memory().map_err(duckdb_error)
    }

    fn split_statements(statement: &str) -> Result<Vec<String>, AltaikaError> {
        let dialect = DuckDbDialect {};
        let tokens = Tokenizer::new(&dialect, statement)
            .tokenize_with_location()
            .map_err(|error| {
                AltaikaError::InvalidArgument(format!("failed to tokenize DuckDB SQL: {error}"))
            })?;
        let line_offsets = line_offsets(statement);
        let mut statements = Vec::new();
        let mut start = 0;
        for token in tokens {
            if token.token != Token::SemiColon {
                continue;
            }
            let end = location_byte_offset(statement, &line_offsets, token.span.start);
            push_statement(&mut statements, &statement[start..end]);
            start = end + 1;
        }
        push_statement(&mut statements, &statement[start..]);
        Ok(statements)
    }

    fn bounded_query(statement: &str, limit: usize) -> String {
        if limit == 0 || !can_wrap_with_limit(statement) {
            return statement.to_string();
        }
        format!(
            "SELECT * FROM ({}) AS altaika_sql LIMIT {}",
            statement.trim().trim_end_matches(';').trim(),
            limit
        )
    }

    fn can_wrap_with_limit(statement: &str) -> bool {
        let first_word = statement
            .trim_start()
            .split(|char: char| char.is_whitespace() || char == '(')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        matches!(first_word.as_str(), "SELECT" | "WITH" | "FROM")
    }

    fn push_statement(statements: &mut Vec<String>, statement: &str) {
        let statement = statement.trim();
        if !statement.is_empty() {
            statements.push(statement.to_string());
        }
    }

    fn line_offsets(statement: &str) -> Vec<usize> {
        let mut offsets = vec![0];
        offsets.extend(
            statement
                .match_indices('\n')
                .map(|(offset, newline)| offset + newline.len()),
        );
        offsets
    }

    fn location_byte_offset(statement: &str, line_offsets: &[usize], location: Location) -> usize {
        let line_index = location.line.saturating_sub(1) as usize;
        let line_start = line_offsets.get(line_index).copied().unwrap_or(0);
        let column = location.column.saturating_sub(1) as usize;
        statement[line_start..]
            .char_indices()
            .map(|(offset, _)| line_start + offset)
            .nth(column)
            .unwrap_or(statement.len())
    }

    fn query_json_rows(conn: &Connection, statement: &str) -> Result<Vec<Value>, AltaikaError> {
        let mut stmt = match conn.prepare(statement) {
            Ok(stmt) => stmt,
            Err(_) => {
                conn.execute_batch(statement).map_err(duckdb_error)?;
                return Ok(Vec::new());
            }
        };
        let mut rows = match stmt.query([]) {
            Ok(rows) => rows,
            Err(_) => {
                conn.execute_batch(statement).map_err(duckdb_error)?;
                return Ok(Vec::new());
            }
        };
        let columns = rows
            .as_ref()
            .map(|stmt| stmt.column_names())
            .unwrap_or_default();
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(duckdb_error)? {
            let mut object = Map::with_capacity(columns.len());
            for (index, column) in columns.iter().enumerate() {
                let value = row.get_ref(index).map_err(duckdb_error)?;
                object.insert(column.clone(), value_ref_to_json(value));
            }
            out.push(Value::Object(object));
        }
        Ok(out)
    }

    fn duckdb_version() -> Result<String, AltaikaError> {
        let conn = open_connection()?;
        conn.query_row("SELECT version()", [], |row| row.get::<_, String>(0))
            .map_err(duckdb_error)
    }

    fn duckdb_has_extension(extension_name: &str) -> bool {
        let Ok(conn) = open_connection() else {
            return false;
        };
        let Ok(mut stmt) = conn
            .prepare("SELECT installed, loaded FROM duckdb_extensions() WHERE extension_name = ?")
        else {
            return false;
        };
        let Ok(mut rows) = stmt.query([extension_name]) else {
            return false;
        };
        loop {
            match rows.next() {
                Ok(Some(row)) => {
                    let installed = row.get::<_, bool>(0).unwrap_or(false);
                    let loaded = row.get::<_, bool>(1).unwrap_or(false);
                    if installed || loaded {
                        return true;
                    }
                }
                Ok(None) => return false,
                Err(_) => return false,
            }
        }
    }

    fn value_ref_to_json(value: ValueRef<'_>) -> Value {
        duckdb_value_to_json(value.to_owned())
    }

    fn duckdb_value_to_json(value: DuckdbValue) -> Value {
        match value {
            DuckdbValue::Null => Value::Null,
            DuckdbValue::Boolean(value) => Value::Bool(value),
            DuckdbValue::TinyInt(value) => Number::from(value).into(),
            DuckdbValue::SmallInt(value) => Number::from(value).into(),
            DuckdbValue::Int(value) => Number::from(value).into(),
            DuckdbValue::BigInt(value) => Number::from(value).into(),
            DuckdbValue::HugeInt(value) => Value::String(value.to_string()),
            DuckdbValue::UTinyInt(value) => Number::from(value).into(),
            DuckdbValue::USmallInt(value) => Number::from(value).into(),
            DuckdbValue::UInt(value) => Number::from(value).into(),
            DuckdbValue::UBigInt(value) => Number::from(value).into(),
            DuckdbValue::Float(value) => float_json(value.into()),
            DuckdbValue::Double(value) => float_json(value),
            DuckdbValue::Decimal(value) => Value::String(value.to_string()),
            DuckdbValue::Timestamp(unit, value) => time_json(unit, value),
            DuckdbValue::Text(value) => Value::String(value),
            DuckdbValue::Blob(value) => Value::Array(
                value
                    .into_iter()
                    .map(|byte| Value::Number(Number::from(byte)))
                    .collect(),
            ),
            DuckdbValue::Date32(value) => Number::from(value).into(),
            DuckdbValue::Time64(unit, value) => time_json(unit, value),
            DuckdbValue::Interval {
                months,
                days,
                nanos,
            } => json!({
                "months": months,
                "days": days,
                "nanos": nanos,
            }),
            DuckdbValue::List(values) | DuckdbValue::Array(values) => {
                Value::Array(values.into_iter().map(duckdb_value_to_json).collect())
            }
            DuckdbValue::Enum(value) => Value::String(value),
            DuckdbValue::Struct(values) => ordered_map_to_json_object(values),
            DuckdbValue::Map(values) => Value::Array(
                values
                    .iter()
                    .map(|(key, value)| {
                        json!({
                            "key": duckdb_value_to_json(key.clone()),
                            "value": duckdb_value_to_json(value.clone()),
                        })
                    })
                    .collect(),
            ),
            DuckdbValue::Union(value) => duckdb_value_to_json(*value),
        }
    }

    fn ordered_map_to_json_object(values: OrderedMap<String, DuckdbValue>) -> Value {
        Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), duckdb_value_to_json(value.clone())))
                .collect(),
        )
    }

    fn float_json(value: f64) -> Value {
        Number::from_f64(value).map_or(Value::Null, Value::Number)
    }

    fn time_json(unit: TimeUnit, value: i64) -> Value {
        json!({
            "unit": match unit {
                TimeUnit::Second => "second",
                TimeUnit::Millisecond => "millisecond",
                TimeUnit::Microsecond => "microsecond",
                TimeUnit::Nanosecond => "nanosecond",
            },
            "value": value,
        })
    }

    fn duckdb_error(error: duckdb::Error) -> AltaikaError {
        AltaikaError::Engine(error.to_string())
    }
}

#[cfg(not(all(
    feature = "duckdb-runtime",
    not(any(target_os = "wasi", target_family = "wasm"))
)))]
mod disabled {
    use serde_json::{Value, json};

    use super::AltaikaError;

    pub struct QueryResult {
        pub rows: Vec<Value>,
        pub elapsed_ms: u128,
        pub stats: Value,
    }

    pub fn execute_sql(_statement: &str, _limit: usize) -> Result<QueryResult, AltaikaError> {
        Err(AltaikaError::UnsupportedOperation(
            "duckdb-beta requires the duckdb-runtime feature on a native target".to_string(),
        ))
    }

    pub fn auth_status(check: bool) -> Value {
        json!({
            "kind": "auth",
            "engine": "duckdb-beta",
            "schema_version": "1.0",
            "data": {
                "ready": false,
                "checked": check,
                "connection": "unsupported",
                "runtime_only": true,
                "missing": ["duckdb_runtime"],
                "present": {
                    "ALTAIKA_DUCKDB_BIN": super::env_present("ALTAIKA_DUCKDB_BIN"),
                    "duckdb_binary": Value::Null,
                    "duckdb_runtime": false,
                    "duckdb_version": "",
                    "quack_extension": false,
                },
            },
        })
    }
}

#[cfg(not(all(
    feature = "duckdb-runtime",
    not(any(target_os = "wasi", target_family = "wasm"))
)))]
pub use disabled::auth_status;
#[cfg(not(all(
    feature = "duckdb-runtime",
    not(any(target_os = "wasi", target_family = "wasm"))
)))]
pub use disabled::execute_sql;
#[cfg(all(
    feature = "duckdb-runtime",
    not(any(target_os = "wasi", target_family = "wasm"))
))]
pub use enabled::auth_status;
#[cfg(all(
    feature = "duckdb-runtime",
    not(any(target_os = "wasi", target_family = "wasm"))
))]
pub use enabled::execute_sql;

fn env_present(key: &str) -> bool {
    std::env::var(key).is_ok_and(|value| !value.trim().is_empty())
}
