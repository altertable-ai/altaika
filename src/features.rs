use serde_json::{Value, json};

pub fn compiled_feature_enabled(name: &str) -> bool {
    match name {
        "duckdb" => cfg!(feature = "duckdb"),
        "ducklake" => cfg!(feature = "ducklake"),
        "quack" => cfg!(feature = "quack"),
        "datafusion" => cfg!(feature = "datafusion"),
        "adbc" => cfg!(feature = "adbc"),
        "bigquery-adbc" => cfg!(feature = "bigquery-adbc"),
        "altertable-platform" => cfg!(feature = "altertable-platform"),
        "wasi" => cfg!(feature = "wasi"),
        _ => false,
    }
}

pub fn compile_features() -> Value {
    json!({
        "duckdb": feature_status(
            "duckdb",
            compiled_feature_enabled("duckdb"),
            true,
            "local execution, local file scans, and DuckDB-family attach"
        ),
        "ducklake": feature_status(
            "ducklake",
            compiled_feature_enabled("ducklake"),
            true,
            "DuckLake catalog support through DuckDB"
        ),
        "quack": feature_status(
            "quack",
            compiled_feature_enabled("quack"),
            true,
            "explicit remote DuckDB execution"
        ),
        "datafusion": feature_status(
            "datafusion",
            compiled_feature_enabled("datafusion"),
            false,
            "optional source fetch plane for remote and object-store connectors"
        ),
        "adbc": feature_status(
            "adbc",
            compiled_feature_enabled("adbc"),
            false,
            "optional ADBC connector runtime"
        ),
        "bigquery-adbc": feature_status(
            "bigquery-adbc",
            compiled_feature_enabled("bigquery-adbc"),
            false,
            "optional BigQuery source connector"
        ),
        "altertable-platform": feature_status(
            "altertable-platform",
            compiled_feature_enabled("altertable-platform"),
            false,
            "optional Altertable platform integration"
        ),
    })
}

pub fn source_feature(scheme: &str) -> Value {
    match scheme {
        "csv" | "parquet" | "duckdb" => feature_status(
            "duckdb",
            compiled_feature_enabled("duckdb"),
            true,
            "local execution, local file scans, and DuckDB-family attach",
        ),
        "ducklake" => feature_status(
            "ducklake",
            compiled_feature_enabled("ducklake"),
            true,
            "DuckLake catalog support through DuckDB",
        ),
        "quack" => feature_status(
            "quack",
            compiled_feature_enabled("quack"),
            true,
            "explicit remote DuckDB execution",
        ),
        "bigquery" => feature_status(
            "bigquery-adbc",
            compiled_feature_enabled("bigquery-adbc"),
            false,
            "optional BigQuery source connector",
        ),
        "altertable" => feature_status(
            "altertable-platform",
            compiled_feature_enabled("altertable-platform"),
            false,
            "optional Altertable platform integration",
        ),
        "s3" | "gs" | "az" | "http" | "https" | "snowflake" | "databricks" | "postgres"
        | "mysql" => feature_status(
            "datafusion",
            compiled_feature_enabled("datafusion"),
            false,
            "optional source fetch plane for remote and object-store connectors",
        ),
        _ => feature_status("unknown", false, false, "unsupported source scheme"),
    }
}

fn feature_status(name: &str, enabled: bool, oss_default: bool, role: &str) -> Value {
    json!({
        "name": name,
        "enabled": enabled,
        "oss_default": oss_default,
        "role": role,
    })
}
