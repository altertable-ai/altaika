use serde_json::Value;

use crate::error::Error;

#[derive(Debug)]
pub struct QueryJsonResult {
    pub rows: Value,
    pub rows_returned: usize,
    pub rows_truncated: bool,
}

pub fn split_sql_statements(input: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut single_quote = false;
    let mut double_quote = false;
    let mut line_comment = false;
    let mut block_comment = false;

    while let Some(character) = chars.next() {
        if line_comment {
            current.push(character);
            if character == '\n' {
                line_comment = false;
            }
            continue;
        }
        if block_comment {
            current.push(character);
            if character == '*' && chars.peek() == Some(&'/') {
                current.push('/');
                let _ = chars.next();
                block_comment = false;
            }
            continue;
        }
        if single_quote {
            current.push(character);
            if character == '\'' {
                if chars.peek() == Some(&'\'') {
                    current.push('\'');
                    let _ = chars.next();
                } else {
                    single_quote = false;
                }
            }
            continue;
        }
        if double_quote {
            current.push(character);
            if character == '"' {
                if chars.peek() == Some(&'"') {
                    current.push('"');
                    let _ = chars.next();
                } else {
                    double_quote = false;
                }
            }
            continue;
        }
        match character {
            '\'' => {
                single_quote = true;
                current.push(character);
            }
            '"' => {
                double_quote = true;
                current.push(character);
            }
            '-' if chars.peek() == Some(&'-') => {
                current.push(character);
                current.push('-');
                let _ = chars.next();
                line_comment = true;
            }
            '/' if chars.peek() == Some(&'*') => {
                current.push(character);
                current.push('*');
                let _ = chars.next();
                block_comment = true;
            }
            ';' => push_statement(&mut statements, &mut current),
            _ => current.push(character),
        }
    }
    push_statement(&mut statements, &mut current);
    statements
}

fn push_statement(statements: &mut Vec<String>, current: &mut String) {
    let statement = current.trim();
    if !statement.is_empty() {
        statements.push(statement.to_owned());
    }
    current.clear();
}

#[cfg(feature = "duckdb")]
mod embedded {
    use std::path::Path;

    use duckdb::types::{TimeUnit, Value as DuckValue, ValueRef};
    use duckdb::{AccessMode, Config, Connection};
    use serde_json::{Map, Number, Value, json};

    use crate::duckdb_runtime::{QueryJsonResult, split_sql_statements};
    use crate::error::Error;

    pub fn query_json_on_database(
        database: &str,
        statement: &str,
        read_only: bool,
    ) -> Result<Value, Error> {
        query_json_on_database_limited(database, statement, read_only, None)
            .map(|result| result.rows)
    }

    pub fn query_json_on_database_limited(
        database: &str,
        statement: &str,
        read_only: bool,
        max_rows: Option<usize>,
    ) -> Result<QueryJsonResult, Error> {
        let conn = open_connection(database, read_only)?;
        execute_statement_sequence(&conn, statement, max_rows)
    }

    pub fn runtime_version_check() -> Value {
        match query_json_on_database(":memory:", "PRAGMA version", false) {
            Ok(rows) => json!({
                "ok": true,
                "rows": rows,
            }),
            Err(error) => json!({
                "ok": false,
                "error": error.to_string(),
            }),
        }
    }

    pub fn extension_check() -> Value {
        let statement = "SELECT extension_name, installed, loaded, installed_from FROM duckdb_extensions() WHERE extension_name IN ('ducklake', 'quack') ORDER BY extension_name";
        match query_json_on_database(":memory:", statement, false) {
            Ok(rows) => {
                let found = rows
                    .as_array()
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| row.get("extension_name").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let has_ducklake = found.contains(&"ducklake");
                let has_quack = found.contains(&"quack");
                json!({
                    "ok": has_ducklake && has_quack,
                    "required": ["ducklake", "quack"],
                    "rows": rows,
                    "quack_functions": quack_function_check(),
                    "missing": missing_extensions(has_ducklake, has_quack),
                })
            }
            Err(error) => json!({
                "ok": false,
                "required": ["ducklake", "quack"],
                "quack_functions": quack_function_check(),
                "error": error.to_string(),
            }),
        }
    }

    fn quack_function_check() -> Value {
        let statement = "INSTALL quack; LOAD quack; SELECT function_name, function_type FROM duckdb_functions() WHERE function_name IN ('quack_query', 'quack_serve', 'quack_stop') ORDER BY function_name";
        match query_json_on_database(":memory:", statement, false) {
            Ok(rows) => {
                let found = rows
                    .as_array()
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| row.get("function_name").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let missing = ["quack_query", "quack_serve", "quack_stop"]
                    .into_iter()
                    .filter(|function| !found.contains(function))
                    .collect::<Vec<_>>();
                json!({
                    "ok": missing.is_empty(),
                    "required": ["quack_query", "quack_serve", "quack_stop"],
                    "missing": missing,
                    "rows": rows,
                })
            }
            Err(error) => json!({
                "ok": false,
                "required": ["quack_query", "quack_serve", "quack_stop"],
                "error": error.to_string(),
            }),
        }
    }

    pub struct QuackServer {
        rows: Value,
        _conn: Connection,
    }

    impl QuackServer {
        pub const fn rows(&self) -> &Value {
            &self.rows
        }
    }

    pub fn start_quack_server(
        database: &str,
        setup_sql: &str,
        remote_uri: &str,
        token: Option<&str>,
        allow_other_hostname: bool,
        disable_ssl: bool,
    ) -> Result<QuackServer, Error> {
        let conn = open_connection(database, false)?;
        conn.execute_batch("INSTALL quack; LOAD quack;")
            .map_err(duckdb_error)?;
        if !setup_sql.trim().is_empty() {
            conn.execute_batch(setup_sql).map_err(duckdb_error)?;
        }
        let statement = quack_serve_sql(remote_uri, token, allow_other_hostname, disable_ssl);
        let rows = query_json(&conn, &statement, None)?.rows;
        Ok(QuackServer { rows, _conn: conn })
    }

    #[cfg(feature = "bigquery-adbc")]
    pub fn load_arrow_reader_to_table(
        database: &str,
        setup_sql: &str,
        table: &str,
        mut reader: Box<dyn duckdb::arrow::array::RecordBatchReader + Send + 'static>,
    ) -> Result<u64, Error> {
        use duckdb::vtab::arrow::{ArrowVTab, arrow_recordbatch_to_query_params};

        let conn = open_connection(database, false)?;
        if !setup_sql.trim().is_empty() {
            conn.execute_batch(setup_sql).map_err(duckdb_error)?;
        }
        conn.register_table_function::<ArrowVTab>("altaika_arrow")
            .map_err(duckdb_error)?;

        let target = DuckdbTargetTable::parse(table)?;
        if let Some(schema_sql) = target.create_schema_sql() {
            conn.execute_batch(&schema_sql).map_err(duckdb_error)?;
        }

        let schema = reader.schema();
        let first_batch = next_arrow_batch(reader.as_mut())?;
        let schema_batch = first_batch
            .clone()
            .unwrap_or_else(|| duckdb::arrow::record_batch::RecordBatch::new_empty(schema));
        conn.execute(
            &format!(
                "CREATE OR REPLACE TABLE {} AS SELECT * FROM altaika_arrow(?1, ?2) LIMIT 0",
                target.sql
            ),
            arrow_recordbatch_to_query_params(schema_batch),
        )
        .map_err(duckdb_error)?;

        let mut row_count = 0;
        if let Some(batch) = first_batch {
            row_count += append_arrow_batch(&conn, &target, batch)?;
        }
        while let Some(batch) = next_arrow_batch(reader.as_mut())? {
            row_count += append_arrow_batch(&conn, &target, batch)?;
        }
        Ok(row_count)
    }

    #[cfg(feature = "bigquery-adbc")]
    struct DuckdbTargetTable {
        sql: String,
        schema: Option<String>,
        table: String,
    }

    #[cfg(feature = "bigquery-adbc")]
    impl DuckdbTargetTable {
        fn parse(input: &str) -> Result<Self, Error> {
            let parts = input.split('.').map(str::trim).collect::<Vec<_>>();
            if parts.iter().any(|part| part.is_empty()) {
                return Err(Error::InvalidArgument(
                    "target table must not contain empty identifier segments".to_owned(),
                ));
            }
            match parts.as_slice() {
                [table] => Ok(Self {
                    sql: quote_identifier(table),
                    schema: None,
                    table: (*table).to_owned(),
                }),
                [schema, table] => Ok(Self {
                    sql: format!("{}.{}", quote_identifier(schema), quote_identifier(table)),
                    schema: Some((*schema).to_owned()),
                    table: (*table).to_owned(),
                }),
                _ => Err(Error::InvalidArgument(
                    "target table must be table or schema.table".to_owned(),
                )),
            }
        }

        fn create_schema_sql(&self) -> Option<String> {
            self.schema
                .as_ref()
                .map(|schema| format!("CREATE SCHEMA IF NOT EXISTS {}", quote_identifier(schema)))
        }
    }

    #[cfg(feature = "bigquery-adbc")]
    fn quote_identifier(input: &str) -> String {
        format!("\"{}\"", input.replace('"', "\"\""))
    }

    #[cfg(feature = "bigquery-adbc")]
    fn next_arrow_batch(
        reader: &mut dyn duckdb::arrow::array::RecordBatchReader,
    ) -> Result<Option<duckdb::arrow::record_batch::RecordBatch>, Error> {
        reader
            .next()
            .transpose()
            .map_err(|error| Error::Duckdb(format!("arrow batch read failed: {error}")))
    }

    #[cfg(feature = "bigquery-adbc")]
    fn append_arrow_batch(
        conn: &Connection,
        target: &DuckdbTargetTable,
        batch: duckdb::arrow::record_batch::RecordBatch,
    ) -> Result<u64, Error> {
        let row_count = u64::try_from(batch.num_rows()).map_err(|error| {
            Error::Duckdb(format!(
                "record batch row count does not fit in u64: {error}"
            ))
        })?;
        if row_count == 0 {
            return Ok(0);
        }
        let mut appender = target
            .schema
            .as_deref()
            .map_or_else(
                || conn.appender(&target.table),
                |schema| conn.appender_to_db(&target.table, schema),
            )
            .map_err(duckdb_error)?;
        appender.append_record_batch(batch).map_err(duckdb_error)?;
        Ok(row_count)
    }

    fn quack_serve_sql(
        remote_uri: &str,
        token: Option<&str>,
        allow_other_hostname: bool,
        disable_ssl: bool,
    ) -> String {
        let mut args = vec![sql_string_literal(remote_uri)];
        if let Some(token) = token {
            args.push(format!("token := {}", sql_string_literal(token)));
        }
        if allow_other_hostname {
            args.push("allow_other_hostname := true".to_owned());
        }
        if disable_ssl {
            args.push("disable_ssl := true".to_owned());
        }
        format!("FROM quack_serve({});", args.join(", "))
    }

    fn sql_string_literal(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    fn open_connection(database: &str, read_only: bool) -> Result<Connection, Error> {
        let config = connection_config(database, read_only)?;
        if database == ":memory:" {
            Connection::open_in_memory_with_flags(config).map_err(duckdb_error)
        } else {
            Connection::open_with_flags(Path::new(database), config).map_err(duckdb_error)
        }
    }

    fn connection_config(database: &str, read_only: bool) -> Result<Config, Error> {
        let config = Config::default();
        if read_only && database != ":memory:" {
            config
                .access_mode(AccessMode::ReadOnly)
                .map_err(duckdb_error)
        } else {
            Ok(config)
        }
    }

    fn execute_statement_sequence(
        conn: &Connection,
        statement: &str,
        max_rows: Option<usize>,
    ) -> Result<QueryJsonResult, Error> {
        let statements = split_sql_statements(statement);
        let Some((final_statement, setup_statements)) = statements.split_last() else {
            return Ok(QueryJsonResult {
                rows: Value::Array(Vec::new()),
                rows_returned: 0,
                rows_truncated: false,
            });
        };
        let setup_sql = setup_statements.join(";\n");
        if !setup_sql.trim().is_empty() {
            conn.execute_batch(&setup_sql).map_err(duckdb_error)?;
        }
        if returns_rows(final_statement) {
            query_json(conn, final_statement, max_rows)
        } else {
            conn.execute_batch(final_statement).map_err(duckdb_error)?;
            Ok(QueryJsonResult {
                rows: Value::Array(Vec::new()),
                rows_returned: 0,
                rows_truncated: false,
            })
        }
    }

    fn query_json(
        conn: &Connection,
        statement: &str,
        max_rows: Option<usize>,
    ) -> Result<QueryJsonResult, Error> {
        let mut stmt = conn.prepare(statement).map_err(duckdb_error)?;
        let mut rows = stmt.query([]).map_err(duckdb_error)?;
        let column_names = rows
            .as_ref()
            .map(duckdb::Statement::column_names)
            .unwrap_or_default();
        let mut values = Vec::new();
        let mut rows_truncated = false;
        while let Some(row) = rows.next().map_err(duckdb_error)? {
            if max_rows.is_some_and(|max_rows| values.len() >= max_rows) {
                rows_truncated = true;
                break;
            }
            let mut object = Map::with_capacity(column_names.len());
            for (index, column_name) in column_names.iter().enumerate() {
                let value = row.get_ref(index).map_err(duckdb_error)?;
                object.insert(column_name.clone(), value_ref_to_json(value));
            }
            values.push(Value::Object(object));
        }
        let rows_returned = values.len();
        Ok(QueryJsonResult {
            rows: Value::Array(values),
            rows_returned,
            rows_truncated,
        })
    }

    fn returns_rows(statement: &str) -> bool {
        matches!(
            first_sql_token(statement).as_str(),
            "select" | "from" | "describe" | "show" | "summarize" | "explain" | "pragma"
        )
    }

    fn first_sql_token(statement: &str) -> String {
        statement
            .trim_start()
            .split(|character: char| character.is_whitespace() || character == ';')
            .find(|token| !token.is_empty())
            .unwrap_or_default()
            .to_ascii_lowercase()
    }

    fn value_ref_to_json(value: ValueRef<'_>) -> Value {
        duck_value_to_json(DuckValue::from(value))
    }

    fn duck_value_to_json(value: DuckValue) -> Value {
        match value {
            DuckValue::Null => Value::Null,
            DuckValue::Boolean(value) => Value::Bool(value),
            DuckValue::TinyInt(value) => json!(value),
            DuckValue::SmallInt(value) => json!(value),
            DuckValue::Int(value) => json!(value),
            DuckValue::BigInt(value) => json!(value),
            DuckValue::HugeInt(value) => Value::String(value.to_string()),
            DuckValue::UTinyInt(value) => json!(value),
            DuckValue::USmallInt(value) => json!(value),
            DuckValue::UInt(value) => json!(value),
            DuckValue::UBigInt(value) => json!(value),
            DuckValue::Float(value) => json_float(f64::from(value)),
            DuckValue::Double(value) => json_float(value),
            DuckValue::Decimal(value) => Value::String(value.to_string()),
            DuckValue::Timestamp(unit, value) => json!({
                "type": "timestamp",
                "unit": time_unit_name(unit),
                "value": value,
            }),
            DuckValue::Text(value) | DuckValue::Enum(value) => Value::String(value),
            DuckValue::Blob(value) => json!({
                "type": "blob",
                "hex": bytes_to_hex(&value),
            }),
            DuckValue::Date32(value) => json!({
                "type": "date32",
                "days_since_epoch": value,
            }),
            DuckValue::Time64(unit, value) => json!({
                "type": "time64",
                "unit": time_unit_name(unit),
                "value": value,
            }),
            DuckValue::Interval {
                months,
                days,
                nanos,
            } => json!({
                "type": "interval",
                "months": months,
                "days": days,
                "nanos": nanos,
            }),
            DuckValue::List(values) | DuckValue::Array(values) => {
                Value::Array(values.into_iter().map(duck_value_to_json).collect())
            }
            DuckValue::Struct(values) => struct_to_json(&values),
            DuckValue::Map(values) => map_to_json(&values),
            DuckValue::Union(value) => duck_value_to_json(*value),
        }
    }

    fn struct_to_json(values: &duckdb::types::OrderedMap<String, DuckValue>) -> Value {
        values
            .iter()
            .map(|(key, value)| (key.clone(), duck_value_to_json(value.clone())))
            .collect::<Map<_, _>>()
            .into()
    }

    fn map_to_json(values: &duckdb::types::OrderedMap<DuckValue, DuckValue>) -> Value {
        values
            .iter()
            .map(|(key, value)| {
                json!({
                    "key": duck_value_to_json(key.clone()),
                    "value": duck_value_to_json(value.clone()),
                })
            })
            .collect()
    }

    fn json_float(value: f64) -> Value {
        Number::from_f64(value).map_or_else(|| Value::String(value.to_string()), Value::Number)
    }

    const fn time_unit_name(unit: TimeUnit) -> &'static str {
        match unit {
            TimeUnit::Second => "second",
            TimeUnit::Millisecond => "millisecond",
            TimeUnit::Microsecond => "microsecond",
            TimeUnit::Nanosecond => "nanosecond",
        }
    }

    fn bytes_to_hex(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut output = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        output
    }

    fn missing_extensions(has_ducklake: bool, has_quack: bool) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !has_ducklake {
            missing.push("ducklake");
        }
        if !has_quack {
            missing.push("quack");
        }
        missing
    }

    fn duckdb_error(error: impl std::fmt::Display) -> Error {
        Error::Duckdb(error.to_string())
    }
}

#[cfg(not(feature = "duckdb"))]
mod embedded {
    use serde_json::{Value, json};

    use crate::duckdb_runtime::QueryJsonResult;
    use crate::error::Error;

    pub struct QuackServer;

    impl QuackServer {
        pub fn rows(&self) -> &Value {
            &Value::Null
        }
    }

    pub fn query_json_on_database(
        _database: &str,
        _statement: &str,
        _read_only: bool,
    ) -> Result<Value, Error> {
        Err(Error::Duckdb(
            "duckdb runtime is not enabled; rebuild with --features duckdb".to_owned(),
        ))
    }

    pub fn query_json_on_database_limited(
        _database: &str,
        _statement: &str,
        _read_only: bool,
        _max_rows: Option<usize>,
    ) -> Result<QueryJsonResult, Error> {
        Err(Error::Duckdb(
            "duckdb runtime is not enabled; rebuild with --features duckdb".to_owned(),
        ))
    }

    pub fn runtime_version_check() -> Value {
        json!({
            "ok": false,
            "error": "duckdb runtime is not enabled; rebuild with --features duckdb",
        })
    }

    pub fn extension_check() -> Value {
        json!({
            "ok": false,
            "required": ["ducklake", "quack"],
            "error": "duckdb runtime is not enabled; rebuild with --features duckdb",
        })
    }

    pub fn start_quack_server(
        _database: &str,
        _setup_sql: &str,
        _remote_uri: &str,
        _token: Option<&str>,
        _allow_other_hostname: bool,
        _disable_ssl: bool,
    ) -> Result<QuackServer, Error> {
        Err(Error::Duckdb(
            "duckdb runtime is not enabled; rebuild with --features duckdb".to_owned(),
        ))
    }
}

pub use embedded::QuackServer;

pub fn query_json_on_database(
    database: &str,
    statement: &str,
    read_only: bool,
) -> Result<Value, Error> {
    embedded::query_json_on_database(database, statement, read_only)
}

pub fn query_json_on_database_limited(
    database: &str,
    statement: &str,
    read_only: bool,
    max_rows: Option<usize>,
) -> Result<QueryJsonResult, Error> {
    embedded::query_json_on_database_limited(database, statement, read_only, max_rows)
}

pub fn runtime_version_check() -> Value {
    embedded::runtime_version_check()
}

pub fn extension_check() -> Value {
    embedded::extension_check()
}

pub fn start_quack_server(
    database: &str,
    setup_sql: &str,
    remote_uri: &str,
    token: Option<&str>,
    allow_other_hostname: bool,
    disable_ssl: bool,
) -> Result<QuackServer, Error> {
    embedded::start_quack_server(
        database,
        setup_sql,
        remote_uri,
        token,
        allow_other_hostname,
        disable_ssl,
    )
}

#[cfg(all(feature = "duckdb", feature = "bigquery-adbc"))]
pub fn load_arrow_reader_to_table(
    database: &str,
    setup_sql: &str,
    table: &str,
    reader: Box<dyn duckdb::arrow::array::RecordBatchReader + Send + 'static>,
) -> Result<u64, Error> {
    embedded::load_arrow_reader_to_table(database, setup_sql, table, reader)
}
