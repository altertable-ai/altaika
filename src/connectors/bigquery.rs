use std::fs;
use std::path::{Path, PathBuf};
#[cfg(feature = "bigquery-adbc")]
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

#[cfg(feature = "bigquery-adbc")]
use crate::agent::agent_response;
#[cfg(feature = "bigquery-adbc")]
use crate::cli::LocalTarget;
use crate::cli::{Connector, PermissionMode, PullArgs, connector_name, target_name};
#[cfg(feature = "bigquery-adbc")]
use crate::duckdb_runtime::load_arrow_reader_to_table;
use crate::error::Error;
#[cfg(feature = "bigquery-adbc")]
use crate::permissions::{OperationClass, operation_permission_summary, operation_summary};

pub fn execute_bigquery_pull(
    args: &PullArgs,
    workspace: &Path,
    permission: PermissionMode,
) -> Result<Value, Error> {
    if args.connector != Some(Connector::Adbc) {
        return Err(bigquery_connector_setup_error(
            "BigQuery pull requires --connector adbc so remote reads stay explicit",
            args,
        ));
    }
    execute_bigquery_adbc_pull(args, workspace, permission)
}

#[cfg(not(feature = "bigquery-adbc"))]
fn execute_bigquery_adbc_pull(
    args: &PullArgs,
    _workspace: &Path,
    _permission: PermissionMode,
) -> Result<Value, Error> {
    Err(bigquery_connector_setup_error(
        "BigQuery ADBC pull requires building Altaika with --features bigquery-adbc and installing the BigQuery ADBC driver",
        args,
    ))
}

#[cfg(feature = "bigquery-adbc")]
#[allow(
    clippy::too_many_lines,
    reason = "ADBC reader must be consumed before the database, connection, and statement drop"
)]
fn execute_bigquery_adbc_pull(
    args: &PullArgs,
    workspace: &Path,
    permission: PermissionMode,
) -> Result<Value, Error> {
    use adbc_core::{Connection, Database, Driver, Statement};

    let started_at = Instant::now();
    let mut stats = bigquery_connector_stats(args);
    let mut driver = load_bigquery_driver_with_stats(&mut stats)?;
    require_bigquery_billing_project(args, &stats)?;
    let source_ref = required_bigquery_source_ref(args, &stats)?;

    let source_sql = bigquery_source_sql(args);
    stats["source_sql"] = Value::String(source_sql.clone());
    stats["source_project"] = Value::String(source_ref.project);
    stats["source_dataset"] = Value::String(source_ref.dataset);
    stats["source_table_name"] = Value::String(source_ref.table);
    stats["phase"] = Value::String("adbc_database".to_owned());
    let database = driver
        .new_database_with_opts(bigquery_database_options(args))
        .map_err(|error| {
            bigquery_adbc_error("BigQuery ADBC database setup failed", args, &stats, &error)
        })?;
    stats["phase"] = Value::String("adbc_connection".to_owned());
    let mut connection = database.new_connection().map_err(|error| {
        bigquery_adbc_error("BigQuery ADBC connection failed", args, &stats, &error)
    })?;
    stats["phase"] = Value::String("adbc_statement".to_owned());
    let mut statement = connection.new_statement().map_err(|error| {
        bigquery_adbc_error("BigQuery ADBC statement setup failed", args, &stats, &error)
    })?;
    statement.set_sql_query(&source_sql).map_err(|error| {
        bigquery_adbc_error("BigQuery ADBC SQL setup failed", args, &stats, &error)
    })?;
    set_bigquery_max_bytes_billed(args, &stats, &mut statement)?;
    stats["phase"] = Value::String("adbc_execute".to_owned());
    let reader = statement
        .execute()
        .map_err(|error| bigquery_adbc_error("BigQuery ADBC query failed", args, &stats, &error))?;

    let database_path = crate::workspace_database(workspace, args.target);
    crate::create_parent_dir(&database_path)?;
    let setup_sql = match args.target {
        LocalTarget::Duckdb => String::new(),
        LocalTarget::Ducklake => crate::attach_prefix(args.target, &database_path),
    };
    stats["phase"] = Value::String("local_arrow_load".to_owned());
    let row_count = load_arrow_reader_to_table(
        &crate::database_arg(args.target, &database_path),
        &setup_sql,
        &args.table,
        reader,
    )
    .map_err(|error| bigquery_runtime_error(args, &stats, &error))?;

    let manifest_path = crate::workspace_manifest(workspace, &args.table);
    let manifest = bigquery_manifest(args, &database_path, &source_sql, row_count)?;
    crate::write_json(&manifest_path, &manifest)?;
    stats["phase"] = Value::String("complete".to_owned());
    stats["row_count"] = Value::from(row_count);
    stats["manifest_written"] = Value::Bool(true);
    stats["elapsed_ms"] = Value::from(elapsed_millis(started_at));

    Ok(agent_response(
        "pull",
        "source_pull",
        "local",
        json!({
            "row_count": row_count,
            "limit": args.limit,
            "source_kind": "bigquery",
            "connector": "adbc",
            "manifest_written": true,
            "operation": operation_summary("pull", OperationClass::RemoteRead),
            "permission": operation_permission_summary(permission, "pull", OperationClass::RemoteRead),
            "driver": stats["driver"].clone(),
            "auth": stats["auth"].clone(),
            "elapsed_ms": stats["elapsed_ms"].clone(),
            "performance_hints": stats["performance_hints"].clone(),
            "migration_goal": stats["migration_goal"].clone(),
        }),
        json!({
            "engine": "duckdb",
            "source": args.source,
            "source_kind": "bigquery",
            "source_sql": source_sql,
            "target": target_name(args.target),
            "database": database_path.display().to_string(),
            "table": args.table,
            "manifest_path": manifest_path.display().to_string(),
            "row_count": row_count,
            "local_default": true,
        }),
    ))
}

#[cfg(feature = "bigquery-adbc")]
fn load_bigquery_driver(
    driver_path: Option<&Path>,
) -> adbc_core::error::Result<adbc_driver_manager::ManagedDriver> {
    use adbc_core::LOAD_FLAG_DEFAULT;
    use adbc_core::options::AdbcVersion;
    use adbc_driver_manager::ManagedDriver;

    if let Some(driver_path) = driver_path {
        return ManagedDriver::load_dynamic_from_filename(driver_path, None, AdbcVersion::V110);
    }
    ManagedDriver::load_from_name("bigquery", None, AdbcVersion::V110, LOAD_FLAG_DEFAULT, None)
}

#[cfg(feature = "bigquery-adbc")]
fn load_bigquery_driver_with_stats(
    stats: &mut Value,
) -> Result<adbc_driver_manager::ManagedDriver, Error> {
    let driver_path = std::env::var("ALTAIKA_BIGQUERY_ADBC_DRIVER_PATH")
        .ok()
        .filter(|path| !path.trim().is_empty())
        .map(|path| (PathBuf::from(path), "env"))
        .or_else(discover_bigquery_driver_path);
    if let Some((driver_path, source)) = driver_path.as_ref() {
        stats["driver_path"] = Value::String(driver_path.display().to_string());
        stats["driver_path_source"] = Value::String((*source).to_owned());
    }
    load_bigquery_driver(driver_path.as_ref().map(|(path, _)| path.as_path()))
        .inspect(|_| {
            stats["driver_checked"] = Value::Bool(true);
            stats["driver_available"] = Value::Bool(true);
        })
        .map_err(|error| {
            stats["driver_checked"] = Value::Bool(true);
            Error::ConnectorSetup {
                message: format!(
                    "BigQuery ADBC driver is not available: {error}. Install it with `dbc install bigquery` or set ALTAIKA_BIGQUERY_ADBC_DRIVER_PATH"
                ),
                mode: "remote",
                stats: stats.clone(),
            }
        })
}

#[cfg(feature = "bigquery-adbc")]
fn set_bigquery_max_bytes_billed<S>(
    args: &PullArgs,
    stats: &Value,
    statement: &mut S,
) -> Result<(), Error>
where
    S: adbc_core::Optionable<Option = adbc_core::options::OptionStatement>,
{
    let Some(max_bytes_billed) = args.max_bytes_billed else {
        return Ok(());
    };
    statement
        .set_option(
            adbc_core::options::OptionStatement::Other(
                "adbc.bigquery.sql.query.max_bytes_billed".to_owned(),
            ),
            i64::try_from(max_bytes_billed)
                .map_err(|error| Error::ConnectorSetup {
                    message: format!("--max-bytes-billed does not fit in i64: {error}"),
                    mode: "remote",
                    stats: stats.clone(),
                })?
                .into(),
        )
        .map_err(|error| {
            bigquery_adbc_error(
                "BigQuery ADBC billing limit setup failed",
                args,
                stats,
                &error,
            )
        })
}

#[cfg(feature = "bigquery-adbc")]
fn bigquery_database_options(
    args: &PullArgs,
) -> Vec<(
    adbc_core::options::OptionDatabase,
    adbc_core::options::OptionValue,
)> {
    use adbc_core::options::OptionDatabase;

    let mut options = Vec::new();
    if let Some(project_id) = bigquery_project_id(args) {
        options.push((
            OptionDatabase::Other("adbc.bigquery.sql.project_id".to_owned()),
            project_id.into(),
        ));
    }
    if let Ok(source_ref) = bigquery_source_ref(args) {
        options.push((
            OptionDatabase::Other("adbc.bigquery.sql.dataset_id".to_owned()),
            source_ref.dataset.into(),
        ));
    }
    if let Some(credentials_path) = non_empty_env("GOOGLE_APPLICATION_CREDENTIALS") {
        options.push((
            OptionDatabase::Other("adbc.bigquery.sql.auth_type".to_owned()),
            "adbc.bigquery.sql.auth_type.json_credential_file".into(),
        ));
        options.push((
            OptionDatabase::Other("adbc.bigquery.sql.auth_credentials".to_owned()),
            credentials_path.into(),
        ));
    } else {
        options.push((
            OptionDatabase::Other("adbc.bigquery.sql.auth_type".to_owned()),
            "adbc.bigquery.sql.auth_type.auth_bigquery".into(),
        ));
    }
    options
}

#[derive(Debug)]
struct BigQuerySourceRef {
    project: String,
    dataset: String,
    table: String,
}

fn bigquery_source_ref(args: &PullArgs) -> Result<BigQuerySourceRef, String> {
    let source_table = bigquery_source_table(args);
    let parts = source_table.split('.').collect::<Vec<_>>();
    match parts.as_slice() {
        [project_id, dataset_id, table_id]
            if !project_id.is_empty() && !dataset_id.is_empty() && !table_id.is_empty() =>
        {
            Ok(BigQuerySourceRef {
                project: (*project_id).to_owned(),
                dataset: (*dataset_id).to_owned(),
                table: (*table_id).to_owned(),
            })
        }
        _ => Err(format!(
            "BigQuery source must be bigquery://project.dataset.table; got `{source_table}`"
        )),
    }
}

#[cfg(feature = "bigquery-adbc")]
fn require_bigquery_billing_project(args: &PullArgs, stats: &Value) -> Result<(), Error> {
    if bigquery_project_id(args).is_some() {
        return Ok(());
    }
    Err(Error::ConnectorSetup {
        message: "BigQuery ADBC pull requires --billing-project or GOOGLE_CLOUD_PROJECT, GCLOUD_PROJECT, or CLOUDSDK_CORE_PROJECT"
            .to_owned(),
        mode: "remote",
        stats: stats.clone(),
    })
}

#[cfg(feature = "bigquery-adbc")]
fn required_bigquery_source_ref(
    args: &PullArgs,
    stats: &Value,
) -> Result<BigQuerySourceRef, Error> {
    bigquery_source_ref(args).map_err(|message| Error::ConnectorSetup {
        message,
        mode: "remote",
        stats: stats.clone(),
    })
}

#[cfg(feature = "bigquery-adbc")]
fn bigquery_project_id(args: &PullArgs) -> Option<String> {
    args.billing_project
        .clone()
        .or_else(|| non_empty_env("GOOGLE_CLOUD_PROJECT"))
        .or_else(|| non_empty_env("GCLOUD_PROJECT"))
        .or_else(|| non_empty_env("CLOUDSDK_CORE_PROJECT"))
}

#[cfg(feature = "bigquery-adbc")]
fn elapsed_millis(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(feature = "bigquery-adbc")]
fn bigquery_adbc_error(
    message: &str,
    args: &PullArgs,
    stats: &Value,
    error: &adbc_core::error::Error,
) -> Error {
    let mut stats = stats.clone();
    stats["phase_error"] = Value::String(error.to_string());
    stats["source_sql"] = Value::String(bigquery_source_sql(args));
    Error::ConnectorSetup {
        message: format!("{message}: {error}"),
        mode: "remote",
        stats,
    }
}

#[cfg(feature = "bigquery-adbc")]
fn bigquery_runtime_error(args: &PullArgs, stats: &Value, error: &Error) -> Error {
    let mut stats = stats.clone();
    stats["phase_error"] = Value::String(error.to_string());
    stats["source_sql"] = Value::String(bigquery_source_sql(args));
    Error::ConnectorSetup {
        message: format!("BigQuery ADBC result load into local DuckDB or DuckLake failed: {error}"),
        mode: "local",
        stats,
    }
}

fn bigquery_connector_setup_error(message: &str, args: &PullArgs) -> Error {
    Error::ConnectorSetup {
        message: message.to_owned(),
        mode: "remote",
        stats: bigquery_connector_stats(args),
    }
}

fn bigquery_source_sql(args: &PullArgs) -> String {
    let source_table = bigquery_source_table(args);
    let columns = if args.columns.is_empty() {
        "*".to_owned()
    } else {
        args.columns
            .iter()
            .map(|column| quote_bigquery_identifier(column))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let filter = args
        .filter
        .as_ref()
        .map(|filter| format!(" WHERE {filter}"))
        .unwrap_or_default();
    format!(
        "SELECT {columns} FROM {}{filter} LIMIT {}",
        quote_bigquery_identifier(source_table),
        args.limit
    )
}

fn bigquery_source_table(args: &PullArgs) -> &str {
    args.source
        .strip_prefix("bigquery://")
        .unwrap_or(args.source.as_str())
}

fn quote_bigquery_identifier(input: &str) -> String {
    format!("`{}`", input.replace('`', "\\`"))
}

#[cfg(feature = "bigquery-adbc")]
fn bigquery_manifest(
    args: &PullArgs,
    database_path: &Path,
    source_sql: &str,
    row_count: u64,
) -> Result<Value, Error> {
    Ok(json!({
        "kind": "pull_manifest",
        "schema_version": "1.0",
        "source": {
            "uri": args.source,
            "connector": "adbc",
            "format": "bigquery",
            "query": source_sql,
            "billing_project": args.billing_project,
            "max_bytes_billed": args.max_bytes_billed,
        },
        "target": {
            "kind": target_name(args.target),
            "database": database_path.display().to_string(),
            "table": args.table,
        },
        "row_count": row_count,
        "limit": args.limit,
        "generated_at_unix_seconds": unix_seconds()?,
    }))
}

fn bigquery_connector_stats(args: &PullArgs) -> Value {
    let source_table = bigquery_source_table(args);
    let source_ref = bigquery_source_ref(args).ok();
    let source_sql = bigquery_source_sql(args);
    json!({
        "connector": "bigquery",
        "requested_connector": args.connector.map(connector_name),
        "required_connector": "adbc",
        "required_feature": "bigquery-adbc",
        "required_driver": "adbc_driver_bigquery",
        "install_hint": "dbc install bigquery or set ALTAIKA_BIGQUERY_ADBC_DRIVER_PATH",
        "source": args.source,
        "source_table": source_table,
        "source_project": source_ref.as_ref().map(|source_ref| source_ref.project.as_str()),
        "source_dataset": source_ref.as_ref().map(|source_ref| source_ref.dataset.as_str()),
        "source_table_name": source_ref.as_ref().map(|source_ref| source_ref.table.as_str()),
        "source_sql": source_sql,
        "target_table": args.table,
        "local_target": target_name(args.target),
        "limit": args.limit,
        "billing_project": args.billing_project,
        "max_bytes_billed": args.max_bytes_billed,
        "auth": bigquery_auth_stats(),
        "driver": bigquery_driver_stats(),
        "migration_goal": {
            "source_query": source_sql,
            "target_table": args.table,
            "local_target": target_name(args.target),
            "parity_checks": [
                format!("SELECT count(*) AS rows FROM {}", args.table),
                format!("DESCRIBE {}", args.table),
                format!("SELECT * FROM {} LIMIT 5", args.table)
            ],
        },
        "performance_hints": [
            "Use --limit while validating connector setup.",
            "Use --max-bytes-billed for BigQuery cost control.",
            "Push projection and filters into the BigQuery source SQL before localizing large tables.",
            "Store the bounded result in DuckLake when the agent will run repeated local checks."
        ],
        "next_commands": [
            "cargo build --features bigquery-adbc",
            "dbc install bigquery or export ALTAIKA_BIGQUERY_ADBC_DRIVER_PATH=/path/to/adbc_driver_bigquery",
            "gcloud auth application-default login or provide platform-native BigQuery credentials",
            "rerun altaika --permission allow pull ... --connector adbc"
        ],
    })
}

fn bigquery_auth_stats() -> Value {
    let adc_path = std::env::var("HOME")
        .ok()
        .map(|home| format!("{home}/.config/gcloud/application_default_credentials.json"));
    json!({
        "google_application_credentials_env": env_present("GOOGLE_APPLICATION_CREDENTIALS"),
        "application_default_credentials_file": adc_path.as_deref().is_some_and(|path| Path::new(path).is_file()),
        "application_default_credentials_path": adc_path,
        "billing_project_env": env_present("GOOGLE_CLOUD_PROJECT") || env_present("GCLOUD_PROJECT") || env_present("CLOUDSDK_CORE_PROJECT"),
    })
}

fn bigquery_driver_stats() -> Value {
    let discovered_path = discover_bigquery_driver_path();
    json!({
        "adbc_driver_path_env": env_present("ALTAIKA_BIGQUERY_ADBC_DRIVER_PATH"),
        "adbc_driver_path": std::env::var("ALTAIKA_BIGQUERY_ADBC_DRIVER_PATH").ok(),
        "discovered_driver_path": discovered_path.as_ref().map(|(path, _)| path.display().to_string()),
        "discovered_driver_path_source": discovered_path.as_ref().map(|(_, source)| *source),
        "dbc_cli_available": command_available("dbc"),
    })
}

fn discover_bigquery_driver_path() -> Option<(PathBuf, &'static str)> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    python_site_package_driver_candidates(&home)
        .into_iter()
        .find(|path| path.is_file())
        .map(|path| (path, "python_site_packages"))
}

fn python_site_package_driver_candidates(home: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mac_python = home.join("Library").join("Python");
    push_python_site_package_driver_candidates(&mut candidates, &mac_python, "lib/python");
    let local_python = home.join(".local").join("lib");
    push_direct_python_site_package_driver_candidates(&mut candidates, &local_python);
    candidates
}

fn push_python_site_package_driver_candidates(
    candidates: &mut Vec<PathBuf>,
    root: &Path,
    site_prefix: &str,
) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let version_root = entry.path();
        candidates.push(
            version_root
                .join(site_prefix)
                .join("site-packages")
                .join("adbc_driver_bigquery")
                .join("libadbc_driver_bigquery.so"),
        );
    }
}

fn push_direct_python_site_package_driver_candidates(candidates: &mut Vec<PathBuf>, root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let version_root = entry.path();
        candidates.push(
            version_root
                .join("site-packages")
                .join("adbc_driver_bigquery")
                .join("libadbc_driver_bigquery.so"),
        );
    }
}

fn env_present(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
}

#[cfg(feature = "bigquery-adbc")]
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(feature = "bigquery-adbc")]
fn unix_seconds() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| {
            Error::InvalidArgument(format!("system clock is before UNIX epoch: {error}"))
        })
}

fn command_available(command: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|path| path.join(command).is_file()))
}
