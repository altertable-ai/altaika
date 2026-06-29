#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use clap::Parser;
use clap::error::ErrorKind;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

mod agent;
pub mod cli;
mod connectors;
mod duckdb_runtime;
pub mod error;
mod features;
mod permissions;

use crate::agent::agent_response;
use crate::cli::{
    CatArgs, Cli, Command, DoctorArgs, ExecArgs, ExecManifestInput, ExecutionMode, InspectArgs,
    LocalTarget, PackPlan, PermissionMode, QuackCommand, QuackQueryArgs, QuackServeArgs,
    RouteSignals, SkillsCommand, SkillsInstallArgs, SourceFormat, SourceSpec, SourceSql, SqlEngine,
    SqlExecution, TableInspectArgs, engine_name, mode_name, source_format_name, target_name,
};
use crate::connectors::bigquery::execute_bigquery_pull;
use crate::duckdb_runtime::{
    extension_check as duckdb_extension_check, query_json_on_database,
    query_json_on_database_limited, runtime_version_check as duckdb_runtime_version_check,
    split_sql_statements, start_quack_server,
};
use crate::error::Error;
use crate::features::{compile_features, compiled_feature_enabled, source_feature};
use crate::permissions::{
    OperationClass, ensure_permission, operation_permission_summary, operation_summary,
};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
const LARGE_LOCAL_FILE_BYTES: u64 = 2 * GIB;
const LARGE_REMOTE_RESULT_BYTES: u64 = 10 * GIB;
const HIGH_LATENCY_MS: u64 = 250;
const SOURCE_ALIAS: &str = "altaika_source";

#[derive(Clone, Copy)]
struct SqlRuntimeInput<'a> {
    workspace: &'a Path,
    engine: SqlEngine,
    mode: ExecutionMode,
    remote: Option<&'a str>,
    quack_token: Option<&'a str>,
    disable_ssl: bool,
    read_only: bool,
    max_rows: Option<usize>,
    statement: &'a str,
}

pub fn run_cli() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            error.exit();
        }
        Err(error) => {
            eprintln!(
                "{}",
                json!({
                    "kind": "error",
                    "schema_version": "1.0",
                    "skill": "error_report",
                    "mode": "local",
                    "stats": {},
                    "error": error.to_string()
                })
            );
            std::process::exit(error.exit_code());
        }
    };
    match run(cli) {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!(
                "{}",
                json!({
                    "kind": "error",
                    "schema_version": "1.0",
                    "skill": error.skill(),
                    "mode": error.mode(),
                    "stats": error.stats(),
                    "error": error.to_string()
                })
            );
            std::process::exit(1);
        }
    }
}

/// Execute one parsed Altaika command and return its agent JSON payload.
///
/// # Errors
///
/// Returns an error when command arguments are invalid, a permission mode blocks
/// the requested operation, I/O fails, a connector is unavailable, or `DuckDB`
/// returns an execution error.
pub fn run(cli: Cli) -> Result<Value, Error> {
    match cli.command {
        Command::Init(args) => {
            ensure_permission(cli.permission, OperationClass::LocalWrite, "init")?;
            init_workspace(&cli.workspace, args.target, cli.permission)
        }
        Command::Explain(args) => {
            let observed_file_size_bytes = observed_local_file_size(&args.source);
            let signals = RouteSignals {
                estimated_bytes: args.estimated_bytes,
                estimated_rows: args.estimated_rows,
                latency_ms: args.latency_ms,
                observed_file_size_bytes,
            };
            route_source(&args.source, args.target, &signals, cli.permission)
        }
        Command::Pull(args) => {
            let scheme = source_scheme(&args.source)?;
            ensure_permission(cli.permission, pull_operation_class(scheme), "pull")?;
            if scheme == "bigquery" {
                return execute_bigquery_pull(&args, &cli.workspace, cli.permission);
            }
            let database = workspace_database(&cli.workspace, args.target);
            let manifest = workspace_manifest(&cli.workspace, &args.table);
            let plan = PackPlan {
                source: parse_source_uri(&args.source)?,
                target: args.target,
                database,
                table: args.table,
                manifest,
                columns: args.columns,
                filter: args.filter,
                limit: args.limit,
            };
            execute_pack_plan(&plan, "pull", cli.permission)
        }
        Command::Query(args) => run_sql_command(&cli.workspace, &args, cli.permission),
        Command::Doctor(args) => doctor_command(&args, cli.permission),
        Command::Ls(args) => list_tables_command(&cli.workspace, &args, cli.permission),
        Command::Describe(args) => describe_table_command(&cli.workspace, &args, cli.permission),
        Command::Show(args) => cat_table_command(&cli.workspace, &args, cli.permission),
        Command::Inspect => status_workspace(&cli.workspace, cli.permission),
        Command::Quack(args) => match args.command {
            QuackCommand::Serve(serve_args) => {
                quack_serve_command(&cli.workspace, &serve_args, cli.permission)
            }
            QuackCommand::Query(query_args) => {
                quack_query_command(&cli.workspace, &query_args, cli.permission)
            }
        },
        Command::Skills(args) => match args.command {
            SkillsCommand::Install(install_args) => {
                install_skill_command(&install_args, cli.permission)
            }
        },
    }
}

fn source_scheme(source: &str) -> Result<&str, Error> {
    source
        .split_once("://")
        .map(|(scheme, _)| scheme)
        .ok_or_else(|| {
            Error::InvalidArgument(format!(
                "source `{source}` must use a URI scheme such as csv://, parquet://, or bigquery://"
            ))
        })
}

fn pull_operation_class(scheme: &str) -> OperationClass {
    match scheme {
        "csv" | "parquet" | "duckdb" | "ducklake" => OperationClass::LocalWrite,
        _ => OperationClass::RemoteRead,
    }
}

fn run_operation_class(mode: ExecutionMode, statement: &str) -> OperationClass {
    match mode {
        ExecutionMode::Local => OperationClass::LocalWrite,
        ExecutionMode::Remote => sql_operation_class(mode, statement),
    }
}

fn sql_operation_class(mode: ExecutionMode, statement: &str) -> OperationClass {
    match mode {
        ExecutionMode::Local => {
            if is_read_only_sql(statement) {
                OperationClass::LocalRead
            } else {
                OperationClass::LocalWrite
            }
        }
        ExecutionMode::Remote => {
            if is_read_only_sql(statement) {
                OperationClass::RemoteRead
            } else {
                OperationClass::RemoteWrite
            }
        }
    }
}

fn is_read_only_sql(statement: &str) -> bool {
    let first_token = first_sql_token(statement);
    matches!(
        first_token.as_str(),
        "select" | "from" | "describe" | "show" | "summarize" | "explain"
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

fn init_workspace(
    workspace: &Path,
    target: LocalTarget,
    permission: PermissionMode,
) -> Result<Value, Error> {
    fs::create_dir_all(workspace.join("manifests"))?;
    fs::create_dir_all(workspace.join("profiles"))?;
    let database = workspace_database(workspace, target);
    create_parent_dir(&database)?;
    let statement = match target {
        LocalTarget::Duckdb => "SELECT 1".to_owned(),
        LocalTarget::Ducklake => format!("{} SELECT 1 AS ok", attach_prefix(target, &database)),
    };
    let _ = query_json_on_database(&database_arg(target, &database), &statement, false)?;
    Ok(agent_response(
        "workspace",
        "workspace_init",
        "local",
        json!({
            "target": target_name(target),
            "directories_created": ["manifests", "profiles"],
            "local_default": true,
            "operation": operation_summary("init", OperationClass::LocalWrite),
            "permission": operation_permission_summary(permission, "init", OperationClass::LocalWrite),
        }),
        json!({
            "workspace": workspace.display().to_string(),
            "target": target_name(target),
            "database": database.display().to_string(),
            "local_default": true,
        }),
    ))
}

fn route_source(
    source: &str,
    target: LocalTarget,
    signals: &RouteSignals,
    permission: PermissionMode,
) -> Result<Value, Error> {
    let Some((scheme, _)) = source.split_once("://") else {
        return Err(Error::InvalidArgument(format!(
            "source `{source}` must use a URI scheme such as csv://, parquet://, or bigquery://"
        )));
    };

    let plan = source_plan(scheme)?;

    Ok(agent_response(
        "explain",
        "routing_explain",
        route_mode(scheme),
        route_stats(scheme, signals, permission),
        json!({
            "source": source,
            "source_scheme": scheme,
            "preferred_local_target": target_name(target),
            "decision": route_decision(scheme, signals),
            "signals": route_signals(signals),
            "features": compile_features(),
            "source_feature": source_feature(scheme),
            "operation": operation_summary("explain", OperationClass::Plan),
            "permission": operation_permission_summary(permission, "explain", OperationClass::Plan),
            "planned_operation": operation_summary("pull", pull_operation_class(scheme)),
            "planned_permission": operation_permission_summary(permission, "pull", pull_operation_class(scheme)),
            "boundaries": integration_boundaries(scheme),
            "thresholds": {
                "large_local_file_bytes": LARGE_LOCAL_FILE_BYTES,
                "large_remote_result_bytes": LARGE_REMOTE_RESULT_BYTES,
                "high_latency_ms": HIGH_LATENCY_MS,
            },
            "workspace": {
                "engine": "duckdb",
                "target": target_name(target),
                "local_default": true,
            },
            "plan": plan,
        }),
    ))
}

fn source_plan(scheme: &str) -> Result<Value, Error> {
    let plan = match scheme {
        "csv" | "parquet" => json!({
            "source_location": "local_file",
            "fetch_engine": "duckdb_scanner",
            "execution": "local",
            "remote_reads": false,
            "supported_now": cfg!(feature = "duckdb"),
            "reason": "DuckDB can scan this file format directly, then persist the bounded result into the local workspace manifest."
        }),
        "s3" | "gs" | "az" | "http" | "https" => json!({
            "source_location": "object_store",
            "fetch_engine": "duckdb_scanner_or_datafusion_object_store",
            "execution": "explicit_pull_then_local",
            "remote_reads": true,
            "supported_now": false,
            "reason": "Object stores should be pulled explicitly so credentials, limits, and cached local state stay visible to agents."
        }),
        "bigquery" => json!({
            "source_location": "remote_platform",
            "fetch_engine": if compiled_feature_enabled("bigquery-adbc") {
                "bigquery_adbc_arrow_stream"
            } else {
                "datafusion_connector_or_source_connector"
            },
            "execution": "explicit_pull_then_local",
            "remote_reads": true,
            "supported_now": compiled_feature_enabled("bigquery-adbc"),
            "reason": if compiled_feature_enabled("bigquery-adbc") {
                "BigQuery uses ADBC to execute bounded SQL and stream Arrow batches into local DuckDB or DuckLake."
            } else {
                "BigQuery is optional; enable bigquery-adbc to fetch bounded Arrow batches before DuckDB or DuckLake becomes the local workspace."
            }
        }),
        "snowflake" | "databricks" | "postgres" | "mysql" | "altertable" => json!({
            "source_location": "remote_platform",
            "fetch_engine": "datafusion_connector_or_source_connector",
            "execution": "explicit_pull_then_local",
            "remote_reads": true,
            "supported_now": false,
            "reason": "Remote platforms should cross the network only during pull. DataFusion or a source connector can fetch bounded Arrow batches before DuckDB or DuckLake becomes the local workspace."
        }),
        "quack" => json!({
            "source_location": "remote_duckdb",
            "fetch_engine": "duckdb_remote_protocol",
            "execution": "explicit_remote_duckdb",
            "remote_reads": true,
            "supported_now": cfg!(feature = "quack"),
            "reason": "Quack is supported for explicit remote DuckDB SQL through query --engine duckdb --mode remote."
        }),
        "duckdb" | "ducklake" => {
            json!({
            "source_location": "duckdb_family",
            "fetch_engine": "duckdb_attach_or_copy",
            "execution": "explicit_pull_then_local",
            "remote_reads": false,
            "supported_now": compiled_feature_enabled(scheme),
            "reason": "DuckDB-family tables can be attached read-only, copied into the local workspace, and recorded in a manifest."
            })
        }
        _ => {
            return Err(Error::InvalidArgument(format!(
                "unsupported source scheme `{scheme}`"
            )));
        }
    };
    Ok(plan)
}

fn integration_boundaries(scheme: &str) -> Value {
    json!({
        "oss_core": {
            "required": true,
            "capabilities": [
                "routing_explain",
                "local_duckdb_workspace",
                "local_ducklake_workspace",
                "manifested_pull",
                "local_query",
                "remote_query",
            ],
        },
        "datafusion": {
            "required_for_oss_core": false,
            "required_for_this_plan": datafusion_required_for_scheme(scheme),
            "feature_enabled": compiled_feature_enabled("datafusion"),
            "role": "source_fetch_plane_for_remote_connectors",
        },
        "quack": {
            "required_for_oss_core": false,
            "required_for_this_plan": scheme == "quack",
            "feature_enabled": compiled_feature_enabled("quack"),
            "role": "optional_remote_duckdb_transport",
        },
        "altertable": {
            "required_for_oss_core": false,
            "required_for_this_plan": scheme == "altertable",
            "feature_enabled": compiled_feature_enabled("altertable-platform"),
            "role": "optional_source_connector_or_publish_target",
        },
    })
}

fn datafusion_required_for_scheme(scheme: &str) -> bool {
    matches!(
        scheme,
        "snowflake" | "databricks" | "postgres" | "mysql" | "altertable"
    ) || (scheme == "bigquery" && !compiled_feature_enabled("bigquery-adbc"))
}

#[allow(clippy::too_many_lines)]
fn route_decision(scheme: &str, signals: &RouteSignals) -> Value {
    let effective_bytes = effective_data_size_bytes(signals);
    let latency_ms = signals.latency_ms;
    let missing = missing_decision_signals(scheme, signals);

    let (recommendation, confidence, requires_remote_confirmation, reasons) = match scheme {
        "csv" | "parquet" => {
            if effective_bytes.is_some_and(|bytes| bytes > LARGE_LOCAL_FILE_BYTES) {
                (
                    "local_workspace",
                    "medium",
                    false,
                    vec![
                        "source is already local",
                        "DuckDB can scan the file directly",
                        "file is large enough to display local disk and runtime pressure",
                    ],
                )
            } else {
                (
                    "local_workspace",
                    "high",
                    false,
                    vec![
                        "source is already local",
                        "DuckDB can scan the file directly",
                        "no remote latency or warehouse cost is involved",
                    ],
                )
            }
        }
        "s3" | "gs" | "az" | "http" | "https" => {
            if effective_bytes.is_some_and(|bytes| bytes > LARGE_REMOTE_RESULT_BYTES) {
                (
                    "remote_filter_then_bounded_pull",
                    "medium",
                    true,
                    vec![
                        "source is remote",
                        "estimated data size is large",
                        "apply projection and filters before creating a local copy",
                    ],
                )
            } else {
                (
                    "explicit_pull_then_local",
                    "medium",
                    true,
                    vec![
                        "source is remote",
                        "local cache avoids repeated network reads",
                        "object-store credentials should cross only during pull",
                    ],
                )
            }
        }
        "bigquery" | "snowflake" | "databricks" | "postgres" | "mysql" | "altertable" => {
            if !missing.is_empty() {
                (
                    "estimate_then_route",
                    "low",
                    true,
                    vec![
                        "source is a remote platform",
                        "estimated data size and latency are needed for a confident route",
                        "remote reads must remain explicit",
                    ],
                )
            } else if effective_bytes.is_some_and(|bytes| bytes > LARGE_REMOTE_RESULT_BYTES) {
                (
                    "remote_pushdown_then_bounded_pull",
                    "medium",
                    true,
                    vec![
                        "source is a remote platform",
                        "estimated data size is large",
                        "push filters and projection before localizing the result",
                    ],
                )
            } else if latency_ms.is_some_and(|latency| latency >= HIGH_LATENCY_MS) {
                (
                    "explicit_pull_then_local",
                    "high",
                    true,
                    vec![
                        "source is a remote platform",
                        "latency makes repeated agent queries expensive",
                        "a bounded local copy improves iteration speed",
                    ],
                )
            } else {
                (
                    "explicit_pull_then_local",
                    "medium",
                    true,
                    vec![
                        "source is a remote platform",
                        "estimated result is bounded enough for a local workspace",
                        "manifested local state makes agent work reproducible",
                    ],
                )
            }
        }
        "quack" => (
            "explicit_remote_duckdb",
            "medium",
            true,
            vec![
                "source is a remote DuckDB session",
                "Quack should be selected explicitly",
                "remote SQL access should not be hidden behind local query",
            ],
        ),
        "duckdb" | "ducklake" => (
            "explicit_attach_or_copy",
            "medium",
            false,
            vec![
                "source is already in the DuckDB family",
                "attach or copy should preserve lineage",
                "avoid surprising cross-database reads",
            ],
        ),
        _ => (
            "unsupported",
            "low",
            true,
            vec!["source scheme is not supported"],
        ),
    };

    json!({
        "recommendation": recommendation,
        "confidence": confidence,
        "requires_remote_confirmation": requires_remote_confirmation,
        "missing_signals": missing,
        "reasons": reasons,
    })
}

fn route_signals(signals: &RouteSignals) -> Value {
    let effective_bytes = effective_data_size_bytes(signals);
    json!({
        "estimated_data_size_bytes": effective_bytes,
        "user_estimated_bytes": signals.estimated_bytes,
        "observed_file_size_bytes": signals.observed_file_size_bytes,
        "estimated_rows": signals.estimated_rows,
        "latency_ms": signals.latency_ms,
        "size_class": size_class(effective_bytes),
        "latency_class": latency_class(signals.latency_ms),
    })
}

fn route_stats(scheme: &str, signals: &RouteSignals, permission: PermissionMode) -> Value {
    let effective_bytes = effective_data_size_bytes(signals);
    json!({
        "source_scheme": scheme,
        "mode": route_mode(scheme),
        "operation": operation_summary("explain", OperationClass::Plan),
        "permission": operation_permission_summary(permission, "explain", OperationClass::Plan),
        "estimated_data_size_bytes": effective_bytes,
        "observed_file_size_bytes": signals.observed_file_size_bytes,
        "estimated_rows": signals.estimated_rows,
        "latency_ms": signals.latency_ms,
        "missing_signal_count": missing_decision_signals(scheme, signals).len(),
    })
}

fn route_mode(scheme: &str) -> &'static str {
    match scheme {
        "csv" | "parquet" | "duckdb" | "ducklake" => "local",
        _ => "remote",
    }
}

fn effective_data_size_bytes(signals: &RouteSignals) -> Option<u64> {
    signals.estimated_bytes.or(signals.observed_file_size_bytes)
}

fn missing_decision_signals(scheme: &str, signals: &RouteSignals) -> Vec<&'static str> {
    match scheme {
        "bigquery" | "snowflake" | "databricks" | "postgres" | "mysql" | "altertable" => {
            let mut missing = Vec::new();
            if effective_data_size_bytes(signals).is_none() {
                missing.push("estimated_bytes");
            }
            if signals.latency_ms.is_none() {
                missing.push("latency_ms");
            }
            missing
        }
        "s3" | "gs" | "az" | "http" | "https" => {
            if effective_data_size_bytes(signals).is_none() {
                vec!["estimated_bytes"]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn observed_local_file_size(source: &str) -> Option<u64> {
    let (scheme, path) = source.split_once("://")?;
    let local_path = match scheme {
        "csv" | "parquet" => Path::new(path),
        "duckdb" | "ducklake" => duckdb_family_database_path(path)?,
        _ => return None,
    };
    fs::metadata(local_path).ok().map(|metadata| metadata.len())
}

fn duckdb_family_database_path(path: &str) -> Option<&Path> {
    let (database_path, _table) = path.rsplit_once('/')?;
    Some(Path::new(database_path))
}

const fn size_class(bytes: Option<u64>) -> &'static str {
    match bytes {
        None => "unknown",
        Some(bytes) if bytes <= 128 * MIB => "small",
        Some(bytes) if bytes <= LARGE_LOCAL_FILE_BYTES => "medium",
        Some(bytes) if bytes <= LARGE_REMOTE_RESULT_BYTES => "large",
        Some(_) => "xlarge",
    }
}

const fn latency_class(latency_ms: Option<u64>) -> &'static str {
    match latency_ms {
        None => "unknown",
        Some(latency) if latency < 100 => "low",
        Some(latency) if latency < HIGH_LATENCY_MS => "medium",
        Some(_) => "high",
    }
}

fn execute_pack_plan(
    plan: &PackPlan,
    kind: &str,
    permission: PermissionMode,
) -> Result<Value, Error> {
    create_parent_dir(&plan.database)?;
    create_parent_dir(&plan.manifest)?;
    let source_sql = source_sql(plan)?;
    let statement = load_sql(
        plan.target,
        &plan.database,
        &plan.table,
        &source_sql.setup,
        &source_sql.query,
    )?;
    let rows = query_json_on_database(
        &database_arg(plan.target, &plan.database),
        &statement,
        false,
    )?;
    let row_count = rows
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("row_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let manifest = manifest(plan, &source_sql, row_count)?;
    write_json(&plan.manifest, &manifest)?;
    Ok(agent_response(
        kind,
        "source_pull",
        "local",
        json!({
            "row_count": row_count,
            "limit": plan.limit,
            "column_count": plan.columns.len(),
            "source_kind": source_format_name(plan.source.format),
            "manifest_written": true,
            "operation": operation_summary(kind, OperationClass::LocalWrite),
            "permission": operation_permission_summary(permission, "pull", OperationClass::LocalWrite),
        }),
        json!({
            "engine": "duckdb",
            "source": &plan.source.uri,
            "source_kind": source_format_name(plan.source.format),
            "target": target_name(plan.target),
            "database": plan.database.display().to_string(),
            "table": &plan.table,
            "manifest_path": plan.manifest.display().to_string(),
            "row_count": row_count,
            "local_default": true,
        }),
    ))
}

fn status_workspace(workspace: &Path, permission: PermissionMode) -> Result<Value, Error> {
    let manifests_dir = workspace.join("manifests");
    let mut manifest_entries = Vec::new();
    if manifests_dir.exists() {
        for entry in fs::read_dir(&manifests_dir)
            .map_err(|error| Error::io_context("read directory", &manifests_dir, error))?
        {
            let entry = entry.map_err(|error| {
                Error::io_context("read directory entry", &manifests_dir, error)
            })?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let manifest = read_json(&path)?;
            let path_display = path.display().to_string();
            let summary = json!({
                "path": path_display,
                "source": manifest.pointer("/source/uri").cloned().unwrap_or(Value::Null),
                "target": manifest.pointer("/target/kind").cloned().unwrap_or(Value::Null),
                "database": manifest.pointer("/target/database").cloned().unwrap_or(Value::Null),
                "table": manifest.pointer("/target/table").cloned().unwrap_or(Value::Null),
                "row_count": manifest.get("row_count").cloned().unwrap_or(Value::Null),
                "generated_at_unix_seconds": manifest
                    .get("generated_at_unix_seconds")
                    .cloned()
                    .unwrap_or(Value::Null),
            });
            manifest_entries.push((path_display, summary));
        }
    }
    manifest_entries.sort_by(|left, right| left.0.cmp(&right.0));
    let manifests = manifest_entries
        .into_iter()
        .map(|(_, manifest)| manifest)
        .collect::<Vec<_>>();
    let runs_dir = workspace.join("runs");
    let mut run_entries = Vec::new();
    if runs_dir.exists() {
        for entry in fs::read_dir(&runs_dir)
            .map_err(|error| Error::io_context("read directory", &runs_dir, error))?
        {
            let entry = entry
                .map_err(|error| Error::io_context("read directory entry", &runs_dir, error))?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let run = read_json(&path)?;
            let path_display = path.display().to_string();
            let summary = json!({
                "path": path_display,
                "target": run.pointer("/target/kind").cloned().unwrap_or(Value::Null),
                "database": run.pointer("/target/database").cloned().unwrap_or(Value::Null),
                "statement_source": run.get("statement_source").cloned().unwrap_or(Value::Null),
                "statement_sha256": run.get("statement_sha256").cloned().unwrap_or(Value::Null),
                "rows_returned": run.get("rows_returned").cloned().unwrap_or(Value::Null),
                "generated_at_unix_seconds": run
                    .get("generated_at_unix_seconds")
                    .cloned()
                    .unwrap_or(Value::Null),
            });
            run_entries.push((path_display, summary));
        }
    }
    run_entries.sort_by(|left, right| left.0.cmp(&right.0));
    let runs = run_entries
        .into_iter()
        .map(|(_, run)| run)
        .collect::<Vec<_>>();

    Ok(agent_response(
        "inspect",
        "workspace_status",
        "local",
        json!({
            "manifest_count": manifests.len(),
            "run_count": runs.len(),
            "operation": operation_summary("inspect", OperationClass::LocalRead),
            "permission": operation_permission_summary(permission, "inspect", OperationClass::LocalRead),
        }),
        json!({
            "workspace": workspace.display().to_string(),
            "duckdb_database": workspace_database(workspace, LocalTarget::Duckdb).display().to_string(),
            "ducklake_catalog": workspace_database(workspace, LocalTarget::Ducklake).display().to_string(),
            "manifest_count": manifests.len(),
            "manifests": manifests,
            "run_count": runs.len(),
            "runs": runs,
        }),
    ))
}

fn doctor_command(args: &DoctorArgs, permission: PermissionMode) -> Result<Value, Error> {
    validate_runtime_args(
        args.mode,
        SqlEngine::Duckdb,
        args.remote.as_deref(),
        args.disable_ssl,
    )?;
    let operation_class = match args.mode {
        ExecutionMode::Local => OperationClass::LocalRead,
        ExecutionMode::Remote => OperationClass::RemoteRead,
    };
    ensure_permission(permission, operation_class, "doctor")?;
    let version = duckdb_runtime_version_check();
    let extensions = duckdb_extension_check();
    let remote = if args.mode == ExecutionMode::Remote {
        duckdb_remote_check(
            args.remote.as_deref(),
            args.quack_token.as_deref(),
            args.disable_ssl,
        )
    } else {
        Value::Null
    };
    let checks = json!({
        "duckdb_runtime": {
            "client": "duckdb-rs",
            "version": version,
            "extensions": extensions,
        },
        "remote": remote,
        "compiled_features": compile_features(),
        "wasi_build": cfg!(feature = "wasi"),
    });
    let ok = doctor_checks_ok(&checks);
    Ok(agent_response(
        "doctor",
        "environment_doctor",
        mode_name(args.mode),
        json!({
            "ok": ok,
            "operation": operation_summary("doctor", operation_class),
            "permission": operation_permission_summary(permission, "doctor", operation_class),
            "warnings": runtime_warnings(args.mode),
        }),
        json!({
            "ok": ok,
            "checks": checks,
            "recommendations": doctor_recommendations(&checks),
        }),
    ))
}

fn install_skill_command(
    args: &SkillsInstallArgs,
    permission: PermissionMode,
) -> Result<Value, Error> {
    let operation_class = if args.dry_run {
        OperationClass::LocalRead
    } else {
        OperationClass::LocalWrite
    };
    ensure_permission(permission, operation_class, "skills_install")?;
    let target_dir = skill_target_dir(args.target_dir.as_deref())?;
    let skill_dir = target_dir.join("altaika");
    let skill_path = skill_dir.join("SKILL.md");
    let content = altaika_skill_content();
    let content_hash = bytes_sha256(content.as_bytes());
    if !args.dry_run {
        fs::create_dir_all(&skill_dir)
            .map_err(|error| Error::io_context("create directory", &skill_dir, error))?;
        fs::write(&skill_path, content)
            .map_err(|error| Error::io_context("write", &skill_path, error))?;
    }
    let files_written = u64::from(!args.dry_run);
    Ok(agent_response(
        "skills_install",
        "skills_install",
        "local",
        json!({
            "dry_run": args.dry_run,
            "files_written": files_written,
            "content_sha256": content_hash,
            "operation": operation_summary("skills_install", operation_class),
            "permission": operation_permission_summary(permission, "skills_install", operation_class),
        }),
        json!({
            "dry_run": args.dry_run,
            "target_dir": target_dir.display().to_string(),
            "skill_dir": skill_dir.display().to_string(),
            "skill_path": skill_path.display().to_string(),
            "files_written": files_written,
            "content_sha256": content_hash,
        }),
    ))
}

fn list_tables_command(
    workspace: &Path,
    args: &InspectArgs,
    permission: PermissionMode,
) -> Result<Value, Error> {
    inspect_sql_command(
        workspace,
        args,
        permission,
        "ls",
        "catalog_list",
        "SELECT table_schema, table_name, table_type FROM information_schema.tables WHERE table_schema NOT IN ('information_schema', 'pg_catalog') ORDER BY table_schema, table_name",
    )
}

fn describe_table_command(
    workspace: &Path,
    args: &TableInspectArgs,
    permission: PermissionMode,
) -> Result<Value, Error> {
    let table = quote_qualified_identifier(&args.table)?;
    inspect_sql_command(
        workspace,
        &args.inspect,
        permission,
        "describe",
        "table_describe",
        &format!("DESCRIBE {table}"),
    )
}

fn cat_table_command(
    workspace: &Path,
    args: &CatArgs,
    permission: PermissionMode,
) -> Result<Value, Error> {
    let table = quote_qualified_identifier(&args.table)?;
    inspect_sql_command(
        workspace,
        &args.inspect,
        permission,
        "show",
        "table_preview",
        &format!("SELECT * FROM {table} LIMIT {}", args.limit),
    )
}

fn quack_serve_command(
    workspace: &Path,
    args: &QuackServeArgs,
    permission: PermissionMode,
) -> Result<Value, Error> {
    ensure_permission(permission, OperationClass::RemoteWrite, "quack_serve")?;
    let target = local_target_for_engine(args.engine)?;
    let quack_uri = normalize_quack_uri(&args.remote)?;
    validate_quack_server_binding(&quack_uri, args.allow_other_hostname)?;
    let database = workspace_database(workspace, target);
    create_parent_dir(&database)?;
    let setup_sql = match target {
        LocalTarget::Duckdb => String::new(),
        LocalTarget::Ducklake => attach_prefix(target, &database),
    };
    let started_at = Instant::now();
    let server = start_quack_server(
        &database_arg(target, &database),
        &setup_sql,
        &quack_uri,
        args.quack_token.as_deref(),
        args.allow_other_hostname,
        args.disable_ssl,
    )
    .map_err(|error| redact_remote_error(error, args.quack_token.as_deref()).with_mode("remote"))?;
    let startup_ms = elapsed_ms(started_at);
    let server_rows = redact_sensitive_json(server.rows());
    let response = agent_response(
        "quack_serve",
        "quack_server",
        "remote",
        json!({
            "server_started": true,
            "startup_ms": startup_ms,
            "engine": engine_name(args.engine),
            "target": target_name(target),
            "token_required": args.quack_token.is_some(),
            "allow_other_hostname": args.allow_other_hostname,
            "disable_ssl": args.disable_ssl,
            "operation": operation_summary("quack_serve", OperationClass::RemoteWrite),
            "permission": operation_permission_summary(permission, "quack_serve", OperationClass::RemoteWrite),
            "warnings": runtime_warnings(ExecutionMode::Remote),
        }),
        json!({
            "remote": {
                "protocol": "quack",
                "uri": quack_uri,
                "token_required": args.quack_token.is_some(),
                "disable_ssl": args.disable_ssl,
            },
            "workspace": workspace.display().to_string(),
            "engine": engine_name(args.engine),
            "database": database.display().to_string(),
            "rows": server_rows,
            "server_lifecycle": "foreground_process",
            "stop_hint": "stop the altaika quack serve process, or call quack_stop from a DuckDB client",
            "query_hint": format!(
                "altaika --permission allow quack query --remote {} \"SELECT 1\"",
                quack_uri.replace("quack:", "quack://")
            ),
            "backend_hint": "run this behind backend-owned process supervision, TLS termination, and token management",
        }),
    );
    println!("{response}");
    io::stdout().flush()?;
    let _server = server;
    loop {
        std::thread::park();
    }
}

fn quack_query_command(
    workspace: &Path,
    args: &QuackQueryArgs,
    permission: PermissionMode,
) -> Result<Value, Error> {
    let (statement, statement_source, statement_file) =
        exec_statement_input(args.file.as_ref(), &args.statement)?;
    let operation_class = sql_operation_class(ExecutionMode::Remote, &statement);
    ensure_permission(permission, operation_class, "quack_query")?;
    let started_at = Instant::now();
    let execution = execute_remote_quack_sql(
        Some(&args.remote),
        args.quack_token.as_deref(),
        args.disable_ssl,
        Some(args.max_rows),
        &statement,
    )
    .map_err(|error| error.with_mode("remote"))?;
    let duration_ms = elapsed_ms(started_at);
    let statement_hash = statement_sha256(&statement);
    Ok(agent_response(
        "quack_query",
        "remote_query",
        "remote",
        json!({
            "engine": "duckdb",
            "duration_ms": duration_ms,
            "rows_returned": execution.rows_returned,
            "rows_truncated": execution.rows_truncated,
            "row_limit": args.max_rows,
            "statement_bytes": statement.len(),
            "statement_sha256": statement_hash,
            "manifest_written": false,
            "remote_calls": execution.remote_calls,
            "operation": operation_summary("quack_query", operation_class),
            "permission": operation_permission_summary(permission, "quack_query", operation_class),
            "warnings": runtime_warnings(ExecutionMode::Remote),
        }),
        json!({
            "engine": "duckdb",
            "mode": "remote",
            "workspace": workspace.display().to_string(),
            "remote": execution.remote,
            "statement_source": statement_source,
            "statement_file": statement_file.map(|path| path.display().to_string()),
            "statement_sha256": statement_hash,
            "rows_returned": execution.rows_returned,
            "rows_truncated": execution.rows_truncated,
            "row_limit": args.max_rows,
            "rows": execution.rows,
            "local_only": false,
        }),
    ))
}

fn inspect_sql_command(
    workspace: &Path,
    args: &InspectArgs,
    permission: PermissionMode,
    kind: &str,
    skill: &'static str,
    statement: &str,
) -> Result<Value, Error> {
    validate_runtime_args(
        args.mode,
        args.engine,
        args.remote.as_deref(),
        args.disable_ssl,
    )?;
    let operation_class = sql_operation_class(args.mode, statement);
    ensure_permission(permission, operation_class, kind)?;
    let execution = execute_sql_with_runtime(SqlRuntimeInput {
        workspace,
        engine: args.engine,
        mode: args.mode,
        remote: args.remote.as_deref(),
        quack_token: args.quack_token.as_deref(),
        disable_ssl: args.disable_ssl,
        read_only: true,
        max_rows: None,
        statement,
    })
    .map_err(|error| error.with_mode(mode_name(args.mode)))?;
    let statement_hash = statement_sha256(statement);
    Ok(agent_response(
        kind,
        skill,
        mode_name(args.mode),
        json!({
            "engine": engine_name(args.engine),
            "rows_returned": execution.rows_returned,
            "rows_truncated": execution.rows_truncated,
            "statement_bytes": statement.len(),
            "statement_sha256": statement_hash,
            "manifest_written": false,
            "remote_calls": execution.remote_calls,
            "operation": operation_summary(kind, operation_class),
            "permission": operation_permission_summary(permission, kind, operation_class),
            "warnings": runtime_warnings(args.mode),
        }),
        json!({
            "engine": "duckdb",
            "sql_engine": engine_name(args.engine),
            "mode": mode_name(args.mode),
            "database": execution.database.display().to_string(),
            "remote": execution.remote,
            "statement_sha256": statement_hash,
            "rows_returned": execution.rows_returned,
            "rows_truncated": execution.rows_truncated,
            "rows": execution.rows,
            "local_only": args.mode == ExecutionMode::Local,
        }),
    ))
}

fn run_sql_command(
    workspace: &Path,
    args: &ExecArgs,
    permission: PermissionMode,
) -> Result<Value, Error> {
    validate_exec_args(args)?;
    let (statement, statement_source, statement_file) =
        exec_statement_input(args.file.as_ref(), &args.statement)?;
    if args.dry_run {
        return dry_run_sql_command(
            workspace,
            args,
            &statement,
            &statement_source,
            statement_file.as_deref(),
            permission,
        );
    }
    let operation_class = run_operation_class(args.mode, &statement);
    ensure_permission(permission, operation_class, "query")?;
    fs::create_dir_all(workspace.join("runs"))?;
    let execution = execute_sql(workspace, args, &statement)
        .map_err(|error| error.with_mode(mode_name(args.mode)))?;
    let statement_hash = statement_sha256(&statement);
    let manifest_path = args
        .manifest
        .clone()
        .unwrap_or_else(|| workspace_run_manifest(workspace, args.name.as_deref()));
    let manifest = exec_manifest(&ExecManifestInput {
        args,
        database: &execution.database,
        statement: &statement,
        statement_source: &statement_source,
        statement_file: statement_file.as_deref(),
        rows_returned: execution.rows_returned,
        statement_hash: &statement_hash,
        remote: &execution.remote,
    })?;
    write_json(&manifest_path, &manifest)?;
    Ok(agent_response(
        "query",
        execution.skill,
        mode_name(args.mode),
        json!({
            "engine": engine_name(args.engine),
            "rows_returned": execution.rows_returned,
            "rows_truncated": execution.rows_truncated,
            "row_limit": args.max_rows,
            "statement_bytes": statement.len(),
            "statement_sha256": statement_hash,
            "manifest_written": true,
            "remote_calls": execution.remote_calls,
            "operation": operation_summary("query", operation_class),
            "permission": operation_permission_summary(permission, "query", operation_class),
            "warnings": runtime_warnings(args.mode),
        }),
        json!({
            "engine": "duckdb",
            "sql_engine": engine_name(args.engine),
            "mode": mode_name(args.mode),
            "database": execution.database.display().to_string(),
            "remote": execution.remote,
            "manifest_path": manifest_path.display().to_string(),
            "statement_source": statement_source,
            "statement_sha256": statement_hash,
            "rows_returned": execution.rows_returned,
            "rows_truncated": execution.rows_truncated,
            "row_limit": args.max_rows,
            "rows": execution.rows,
            "local_only": args.mode == ExecutionMode::Local,
        }),
    ))
}

fn dry_run_sql_command(
    workspace: &Path,
    args: &ExecArgs,
    statement: &str,
    statement_source: &str,
    statement_file: Option<&Path>,
    permission: PermissionMode,
) -> Result<Value, Error> {
    validate_dry_run_statement(statement)?;
    let explain_statement = explain_statement(statement);
    let operation_class = sql_operation_class(args.mode, &explain_statement);
    ensure_permission(permission, operation_class, "query_dry_run")?;
    let execution = execute_sql_with_runtime(SqlRuntimeInput {
        workspace,
        engine: args.engine,
        mode: args.mode,
        remote: args.remote.as_deref(),
        quack_token: args.quack_token.as_deref(),
        disable_ssl: args.disable_ssl,
        read_only: true,
        max_rows: Some(args.max_rows),
        statement: &explain_statement,
    })
    .map_err(|error| error.with_mode(mode_name(args.mode)))?;
    let statement_hash = statement_sha256(statement);
    let explain_statement_hash = statement_sha256(&explain_statement);
    Ok(agent_response(
        "query",
        "query_explain",
        mode_name(args.mode),
        json!({
            "engine": engine_name(args.engine),
            "dry_run": true,
            "rows_returned": execution.rows_returned,
            "rows_truncated": execution.rows_truncated,
            "row_limit": args.max_rows,
            "statement_bytes": statement.len(),
            "statement_sha256": statement_hash,
            "explain_statement_sha256": explain_statement_hash,
            "manifest_written": false,
            "remote_calls": execution.remote_calls,
            "operation": operation_summary("query_dry_run", operation_class),
            "permission": operation_permission_summary(permission, "query_dry_run", operation_class),
            "warnings": runtime_warnings(args.mode),
        }),
        json!({
            "engine": "duckdb",
            "sql_engine": engine_name(args.engine),
            "mode": mode_name(args.mode),
            "dry_run": true,
            "database": execution.database.display().to_string(),
            "remote": execution.remote,
            "manifest_path": Value::Null,
            "statement_source": statement_source,
            "statement_file": statement_file.map(|path| path.display().to_string()),
            "statement_sha256": statement_hash,
            "explain_statement_sha256": explain_statement_hash,
            "rows_returned": execution.rows_returned,
            "rows_truncated": execution.rows_truncated,
            "row_limit": args.max_rows,
            "rows": execution.rows,
            "local_only": args.mode == ExecutionMode::Local,
        }),
    ))
}

fn explain_statement(statement: &str) -> String {
    if first_sql_token(statement) == "explain" {
        statement.to_owned()
    } else {
        format!("EXPLAIN {statement}")
    }
}

fn validate_dry_run_statement(statement: &str) -> Result<(), Error> {
    let statements = split_sql_statements(statement);
    if statements.len() != 1 {
        return Err(Error::InvalidArgument(
            "query --dry-run accepts exactly one SQL statement because later statements can still execute side effects"
                .to_owned(),
        ));
    }
    let keywords = leading_sql_keywords(statement, 2);
    if keywords.first().is_some_and(|keyword| keyword == "explain")
        && keywords.get(1).is_some_and(|keyword| keyword == "analyze")
    {
        return Err(Error::InvalidArgument(
            "query --dry-run rejects EXPLAIN ANALYZE because DuckDB executes the statement while analyzing it"
                .to_owned(),
        ));
    }
    Ok(())
}

fn leading_sql_keywords(statement: &str, limit: usize) -> Vec<String> {
    let mut keywords = Vec::with_capacity(limit);
    let mut index = 0;
    while keywords.len() < limit {
        index = skip_sql_gap(statement, index);
        let Some((next_index, keyword)) = read_sql_keyword(statement, index) else {
            break;
        };
        keywords.push(keyword.to_ascii_lowercase());
        index = next_index;
    }
    keywords
}

fn skip_sql_gap(statement: &str, mut index: usize) -> usize {
    while index < statement.len() {
        let rest = &statement[index..];
        if let Some(character) = rest
            .chars()
            .next()
            .filter(|character| character.is_whitespace())
        {
            index += character.len_utf8();
        } else if let Some(comment_end) = rest.strip_prefix("--").and_then(|rest| rest.find('\n')) {
            index += 2 + comment_end + 1;
        } else if let Some(comment_end) = rest.strip_prefix("/*").and_then(|rest| rest.find("*/")) {
            index += 2 + comment_end + 2;
        } else {
            break;
        }
    }
    index
}

fn read_sql_keyword(statement: &str, index: usize) -> Option<(usize, &str)> {
    let rest = statement.get(index..)?;
    let mut end = index;
    for character in rest.chars() {
        if character.is_ascii_alphabetic() || character == '_' {
            end += character.len_utf8();
        } else {
            break;
        }
    }
    (end > index).then(|| (end, &statement[index..end]))
}

fn execute_sql(workspace: &Path, args: &ExecArgs, statement: &str) -> Result<SqlExecution, Error> {
    execute_sql_with_runtime(SqlRuntimeInput {
        workspace,
        engine: args.engine,
        mode: args.mode,
        remote: args.remote.as_deref(),
        quack_token: args.quack_token.as_deref(),
        disable_ssl: args.disable_ssl,
        read_only: false,
        max_rows: Some(args.max_rows),
        statement,
    })
}

fn execute_sql_with_runtime(input: SqlRuntimeInput<'_>) -> Result<SqlExecution, Error> {
    match (input.mode, input.engine) {
        (ExecutionMode::Local, SqlEngine::Duckdb | SqlEngine::Ducklake) => {
            execute_local_engine_sql(
                input.workspace,
                input.engine,
                input.read_only,
                input.max_rows,
                input.statement,
            )
        }
        (ExecutionMode::Local, SqlEngine::Datafusion) => Err(Error::InvalidArgument(
            "local DataFusion SQL execution is planned but not implemented yet".to_owned(),
        )),
        (ExecutionMode::Remote, SqlEngine::Duckdb) => execute_remote_quack_sql(
            input.remote,
            input.quack_token,
            input.disable_ssl,
            input.max_rows,
            input.statement,
        ),
        (ExecutionMode::Remote, SqlEngine::Ducklake) => Err(Error::InvalidArgument(
            "remote DuckLake should run through a remote DuckDB Quack endpoint with DuckLake attached on the server".to_owned(),
        )),
        (ExecutionMode::Remote, SqlEngine::Datafusion) => Err(Error::InvalidArgument(
            "remote DataFusion SQL execution is planned but not implemented yet".to_owned(),
        )),
    }
}

fn execute_local_engine_sql(
    workspace: &Path,
    engine: SqlEngine,
    read_only: bool,
    max_rows: Option<usize>,
    statement: &str,
) -> Result<SqlExecution, Error> {
    let target = local_target_for_engine(engine)?;
    let database = workspace_database(workspace, target);
    if !read_only {
        create_parent_dir(&database)?;
    }
    let sql = match target {
        LocalTarget::Duckdb => statement.to_owned(),
        LocalTarget::Ducklake => format!(
            "{} {statement}",
            attach_prefix_with_mode(target, &database, read_only)
        ),
    };
    let query_result = query_json_on_database_limited(
        &database_arg(target, &database),
        &sql,
        read_only && target == LocalTarget::Duckdb,
        max_rows,
    )?;
    Ok(SqlExecution {
        rows: query_result.rows,
        rows_returned: query_result.rows_returned,
        rows_truncated: query_result.rows_truncated,
        database,
        remote: Value::Null,
        remote_calls: 0,
        skill: "local_query",
    })
}

fn validate_exec_args(args: &ExecArgs) -> Result<(), Error> {
    validate_runtime_args(
        args.mode,
        args.engine,
        args.remote.as_deref(),
        args.disable_ssl,
    )
}

fn validate_runtime_args(
    mode: ExecutionMode,
    engine: SqlEngine,
    remote: Option<&str>,
    disable_ssl: bool,
) -> Result<(), Error> {
    match mode {
        ExecutionMode::Local => {
            if remote.is_some() {
                return Err(Error::InvalidCommandArgument {
                    message: "local mode does not accept --remote; use --mode remote for Quack"
                        .to_owned(),
                    mode: "local",
                });
            }
            if disable_ssl {
                return Err(Error::InvalidCommandArgument {
                    message: "local mode does not accept --quack-disable-ssl".to_owned(),
                    mode: "local",
                });
            }
        }
        ExecutionMode::Remote => {
            if engine != SqlEngine::Duckdb {
                return Err(Error::InvalidCommandArgument {
                    message: "remote mode currently supports only --engine duckdb through Quack"
                        .to_owned(),
                    mode: "remote",
                });
            }
            if remote.is_none() {
                return Err(Error::InvalidCommandArgument {
                    message: "remote DuckDB execution requires --remote quack://host:port"
                        .to_owned(),
                    mode: "remote",
                });
            }
        }
    }
    Ok(())
}

fn execute_remote_quack_sql(
    remote: Option<&str>,
    quack_token: Option<&str>,
    disable_ssl: bool,
    max_rows: Option<usize>,
    statement: &str,
) -> Result<SqlExecution, Error> {
    let Some(remote) = remote else {
        return Err(Error::InvalidArgument(
            "remote DuckDB execution requires --remote quack://host:port".to_owned(),
        ));
    };
    let quack_uri = normalize_quack_uri(remote)?;
    let query_sql = quack_query_sql(&quack_uri, statement, quack_token, disable_ssl);
    let query_result = query_json_on_database_limited(":memory:", &query_sql, false, max_rows)
        .map_err(|error| redact_remote_error(error, quack_token))?;
    Ok(SqlExecution {
        rows: query_result.rows,
        rows_returned: query_result.rows_returned,
        rows_truncated: query_result.rows_truncated,
        database: PathBuf::from(":memory:"),
        remote: json!({
            "protocol": "quack",
            "uri": quack_uri,
            "token_provided": quack_token.is_some(),
            "disable_ssl": disable_ssl,
        }),
        remote_calls: 1,
        skill: "remote_query",
    })
}

fn quack_query_sql(
    quack_uri: &str,
    statement: &str,
    token: Option<&str>,
    disable_ssl: bool,
) -> String {
    let mut args = vec![sql_string_literal(quack_uri), sql_string_literal(statement)];
    if let Some(token) = token {
        args.push(format!("token := {}", sql_string_literal(token)));
    }
    if disable_ssl {
        args.push("disable_ssl := true".to_owned());
    }
    format!(
        "INSTALL quack; LOAD quack; FROM quack_query({});",
        args.join(", ")
    )
}

fn normalize_quack_uri(input: &str) -> Result<String, Error> {
    if let Some(rest) = input.strip_prefix("quack://") {
        return Ok(format!("quack:{rest}"));
    }
    if input.starts_with("quack:") {
        return Ok(input.to_owned());
    }
    Err(Error::InvalidArgument(
        "Quack remote endpoints must start with quack:// or quack:".to_owned(),
    ))
}

fn validate_quack_server_binding(quack_uri: &str, allow_other_hostname: bool) -> Result<(), Error> {
    if allow_other_hostname {
        return Ok(());
    }
    let endpoint = quack_uri.strip_prefix("quack:").unwrap_or(quack_uri);
    let host = endpoint
        .rsplit_once('@')
        .map_or(endpoint, |(_, host_and_port)| host_and_port)
        .split_once(':')
        .map_or(endpoint, |(host, _)| host);
    if matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]") {
        Ok(())
    } else {
        Err(Error::InvalidArgument(
            "quack serve binds only localhost by default; pass --allow-other-hostname for supervised backend usage"
                .to_owned(),
        ))
    }
}

fn local_target_for_engine(engine: SqlEngine) -> Result<LocalTarget, Error> {
    match engine {
        SqlEngine::Duckdb => Ok(LocalTarget::Duckdb),
        SqlEngine::Ducklake => Ok(LocalTarget::Ducklake),
        SqlEngine::Datafusion => Err(Error::InvalidArgument(
            "DataFusion is not a DuckDB local target".to_owned(),
        )),
    }
}

fn exec_statement_input(
    file: Option<&PathBuf>,
    statement_parts: &[String],
) -> Result<(String, String, Option<PathBuf>), Error> {
    let has_inline_statement = !statement_parts.is_empty();
    match (file, has_inline_statement) {
        (Some(_), true) => Err(Error::InvalidArgument(
            "query accepts either --file or an inline SQL statement, not both".to_owned(),
        )),
        (None, false) => Err(Error::InvalidArgument(
            "query requires --file or an inline SQL statement".to_owned(),
        )),
        (Some(path), false) => {
            let statement = fs::read_to_string(path)
                .map_err(|error| Error::io_context("read SQL file", path, error))?;
            validate_statement(&statement)?;
            Ok((statement, "file".to_owned(), Some(path.clone())))
        }
        (None, true) => {
            let statement = statement_parts.join(" ");
            validate_statement(&statement)?;
            Ok((statement, "inline".to_owned(), None))
        }
    }
}

fn validate_statement(statement: &str) -> Result<(), Error> {
    if statement.trim().is_empty() {
        return Err(Error::InvalidArgument(
            "query SQL statement must not be empty".to_owned(),
        ));
    }
    Ok(())
}

fn exec_manifest(input: &ExecManifestInput<'_>) -> Result<Value, Error> {
    let mut manifest = json!({
        "kind": "exec_manifest",
        "schema_version": "1.0",
        "target": {
            "kind": engine_name(input.args.engine),
            "database": input.database.display().to_string(),
        },
        "mode": mode_name(input.args.mode),
        "remote": input.remote,
        "statement_source": input.statement_source,
        "statement_file": input.statement_file.map(|path| path.display().to_string()),
        "statement_sha256": input.statement_hash,
        "statement_bytes": input.statement.len(),
        "rows_returned": input.rows_returned,
        "local_only": input.args.mode == ExecutionMode::Local,
        "recorded_sql": input.args.record_sql,
        "generated_at_unix_seconds": unix_seconds()?,
    });
    if input.args.record_sql {
        manifest["statement"] = Value::String(input.statement.to_owned());
    }
    Ok(manifest)
}

fn statement_sha256(statement: &str) -> String {
    bytes_sha256(statement.as_bytes())
}

fn bytes_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

fn redact_sensitive_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => object
            .iter()
            .map(|(key, value)| {
                let redacted_value = if is_sensitive_json_key(key) {
                    Value::String("[redacted]".to_owned())
                } else {
                    redact_sensitive_json(value)
                };
                (key.clone(), redacted_value)
            })
            .collect::<Map<_, _>>()
            .into(),
        Value::Array(values) => Value::Array(values.iter().map(redact_sensitive_json).collect()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}

fn is_sensitive_json_key(key: &str) -> bool {
    matches!(
        key,
        "token"
            | "auth_token"
            | "access_token"
            | "refresh_token"
            | "id_token"
            | "password"
            | "secret"
            | "credential"
    ) || key.ends_with("_token")
}

fn redact_remote_error(error: Error, token: Option<&str>) -> Error {
    let secrets = token.into_iter().collect::<Vec<_>>();
    error.redact_secrets(&secrets)
}

fn duckdb_remote_check(
    remote: Option<&str>,
    quack_token: Option<&str>,
    disable_ssl: bool,
) -> Value {
    let Some(remote) = remote else {
        return json!({
            "ok": false,
            "error": "remote doctor requires --remote quack://host:port",
        });
    };
    let quack_uri = match normalize_quack_uri(remote) {
        Ok(uri) => uri,
        Err(error) => {
            return json!({
                "ok": false,
                "error": error.to_string(),
            });
        }
    };
    let statement = quack_query_sql(&quack_uri, "SELECT 1 AS ok", quack_token, disable_ssl);
    match query_json_on_database(":memory:", &statement, false) {
        Ok(rows) => json!({
            "ok": true,
            "protocol": "quack",
            "uri": quack_uri,
            "token_provided": quack_token.is_some(),
            "disable_ssl": disable_ssl,
            "transport": "stateless_quack_query",
            "rows": rows,
        }),
        Err(error) => json!({
            "ok": false,
            "protocol": "quack",
            "uri": quack_uri,
            "token_provided": quack_token.is_some(),
            "disable_ssl": disable_ssl,
            "transport": "stateless_quack_query",
            "error": redact_remote_error(error, quack_token).to_string(),
        }),
    }
}

fn doctor_checks_ok(checks: &Value) -> bool {
    let version_ok = checks
        .pointer("/duckdb_runtime/version/ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let extensions_ok = checks
        .pointer("/duckdb_runtime/extensions/ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let quack_functions_ok = checks
        .pointer("/duckdb_runtime/extensions/quack_functions/ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let remote_ok = checks.get("remote").is_none_or(|remote| {
        remote.is_null() || remote.get("ok").and_then(Value::as_bool).unwrap_or(false)
    });
    version_ok && extensions_ok && quack_functions_ok && remote_ok
}

fn doctor_recommendations(checks: &Value) -> Vec<&'static str> {
    let mut recommendations = Vec::new();
    if !checks
        .pointer("/duckdb_runtime/version/ok")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        recommendations.push("rebuild Altaika with the duckdb feature enabled");
    }
    if !checks
        .pointer("/duckdb_runtime/extensions/ok")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        recommendations
            .push("use DuckDB v1.5.2 or newer with DuckLake and Quack extensions available");
    }
    if !checks
        .pointer("/duckdb_runtime/extensions/quack_functions/ok")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        recommendations.push(
            "use an embedded or backend DuckDB runtime that exposes quack_serve, quack_query, and quack_stop",
        );
    }
    if checks
        .pointer("/remote/ok")
        .and_then(Value::as_bool)
        .is_some_and(|ok| !ok)
    {
        recommendations.push("start the Quack server or verify --remote, token, and TLS settings");
    }
    recommendations
}

fn runtime_warnings(mode: ExecutionMode) -> Vec<&'static str> {
    match mode {
        ExecutionMode::Local => Vec::new(),
        ExecutionMode::Remote => vec![
            "Quack is a beta DuckDB remote protocol; keep remote usage explicit",
            "Use TLS termination in front of production Quack and reserve --quack-disable-ssl for local testing",
        ],
    }
}

fn skill_target_dir(target_dir: Option<&Path>) -> Result<PathBuf, Error> {
    if let Some(target_dir) = target_dir {
        return Ok(target_dir.to_path_buf());
    }
    if let Ok(agents_home) = env::var("AGENTS_HOME") {
        return Ok(PathBuf::from(agents_home).join("skills"));
    }
    if let Ok(codex_home) = env::var("CODEX_HOME") {
        return Ok(PathBuf::from(codex_home).join("skills"));
    }
    env::var("HOME")
        .map(|home| PathBuf::from(home).join(".agents").join("skills"))
        .map_err(|error| {
            Error::InvalidArgument(format!(
                "cannot infer skills directory from AGENTS_HOME, CODEX_HOME, or HOME: {error}"
            ))
        })
}

const fn altaika_skill_content() -> &'static str {
    include_str!("../skills/altaika/SKILL.md")
}

fn source_sql(plan: &PackPlan) -> Result<SourceSql, Error> {
    let setup = source_setup_sql(&plan.source)?;
    let query = source_query(plan)?;
    Ok(SourceSql { setup, query })
}

fn source_query(plan: &PackPlan) -> Result<String, Error> {
    let columns = if plan.columns.is_empty() {
        "*".to_owned()
    } else {
        plan.columns
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Result<Vec<_>, _>>()?
            .join(", ")
    };
    let scanner = match plan.source.format {
        SourceFormat::Csv | SourceFormat::Parquet => file_scanner(&plan.source)?,
        SourceFormat::Duckdb | SourceFormat::Ducklake => attached_source_table(&plan.source)?,
    };
    let filter = plan
        .filter
        .as_ref()
        .map(|filter| format!(" WHERE {filter}"))
        .unwrap_or_default();
    Ok(format!(
        "SELECT {columns} FROM {scanner}{filter} LIMIT {}",
        plan.limit
    ))
}

fn file_scanner(source: &SourceSpec) -> Result<String, Error> {
    let path = sql_string_literal(&source.path.display().to_string());
    match source.format {
        SourceFormat::Csv => Ok(format!("read_csv_auto({path})")),
        SourceFormat::Parquet => Ok(format!("read_parquet({path})")),
        SourceFormat::Duckdb | SourceFormat::Ducklake => Err(Error::InvalidArgument(
            "DuckDB-family sources are attached tables, not file scanners".to_owned(),
        )),
    }
}

fn source_setup_sql(source: &SourceSpec) -> Result<String, Error> {
    match source.format {
        SourceFormat::Csv | SourceFormat::Parquet => Ok(String::new()),
        SourceFormat::Duckdb => Ok(format!(
            "ATTACH {} AS {} (READ_ONLY);",
            sql_string_literal(&source.path.display().to_string()),
            quote_identifier(SOURCE_ALIAS)?
        )),
        SourceFormat::Ducklake => Ok(format!(
            "INSTALL ducklake; LOAD ducklake; ATTACH {} AS {} (READ_ONLY);",
            sql_string_literal(&format!("ducklake:{}", source.path.display())),
            quote_identifier(SOURCE_ALIAS)?
        )),
    }
}

fn attached_source_table(source: &SourceSpec) -> Result<String, Error> {
    let table = source.table.as_ref().ok_or_else(|| {
        Error::InvalidArgument("DuckDB-family sources require a source table".to_owned())
    })?;
    attached_table_reference(SOURCE_ALIAS, table)
}

fn attached_table_reference(alias: &str, table: &str) -> Result<String, Error> {
    let parts = table.split('.').map(str::trim).collect::<Vec<_>>();
    if parts.iter().any(|part| part.is_empty()) {
        return Err(Error::InvalidArgument(
            "source table must not contain empty identifier segments".to_owned(),
        ));
    }
    match parts.as_slice() {
        [table] => Ok(format!(
            "{}.{}.{}",
            quote_identifier(alias)?,
            quote_identifier("main")?,
            quote_identifier(table)?
        )),
        [schema, table] => Ok(format!(
            "{}.{}.{}",
            quote_identifier(alias)?,
            quote_identifier(schema)?,
            quote_identifier(table)?
        )),
        _ => Err(Error::InvalidArgument(
            "source table must be table or schema.table".to_owned(),
        )),
    }
}

fn load_sql(
    target: LocalTarget,
    database: &Path,
    table: &str,
    source_setup: &str,
    query: &str,
) -> Result<String, Error> {
    let table = quote_qualified_identifier(table)?;
    let load = format!(
        "CREATE OR REPLACE TABLE {table} AS {query}; SELECT count(*) AS row_count FROM {table};"
    );
    match target {
        LocalTarget::Duckdb => Ok(prefix_sql(source_setup, &load)),
        LocalTarget::Ducklake => {
            let setup = prefix_sql(&attach_prefix(target, database), source_setup);
            Ok(prefix_sql(&setup, &load))
        }
    }
}

fn prefix_sql(prefix: &str, statement: &str) -> String {
    if prefix.is_empty() {
        statement.to_owned()
    } else {
        format!("{prefix} {statement}")
    }
}

fn attach_prefix(target: LocalTarget, database: &Path) -> String {
    attach_prefix_with_mode(target, database, false)
}

fn attach_prefix_with_mode(target: LocalTarget, database: &Path, read_only: bool) -> String {
    match target {
        LocalTarget::Duckdb => String::new(),
        LocalTarget::Ducklake => {
            let database = sql_string_literal(&format!("ducklake:{}", database.display()));
            let read_only = if read_only { " (READ_ONLY)" } else { "" };
            format!(
                "INSTALL ducklake; LOAD ducklake; ATTACH {database} AS lake{read_only}; USE lake;"
            )
        }
    }
}

fn database_arg(target: LocalTarget, database: &Path) -> String {
    match target {
        LocalTarget::Duckdb => database.display().to_string(),
        LocalTarget::Ducklake => ":memory:".to_owned(),
    }
}

fn parse_source_uri(input: &str) -> Result<SourceSpec, Error> {
    let Some((scheme, path)) = input.split_once("://") else {
        return Err(Error::InvalidArgument(format!(
            "source `{input}` must use csv://path or parquet://path"
        )));
    };
    let format = match scheme {
        "csv" => SourceFormat::Csv,
        "parquet" => SourceFormat::Parquet,
        "duckdb" => SourceFormat::Duckdb,
        "ducklake" => SourceFormat::Ducklake,
        "bigquery" | "snowflake" | "databricks" | "altertable" => {
            return Err(Error::InvalidArgument(format!(
                "remote source `{scheme}` is planned; export or connector support must be explicit via pull"
            )));
        }
        _ => {
            return Err(Error::InvalidArgument(format!(
                "unsupported source scheme `{scheme}`"
            )));
        }
    };
    let (path, table) = match format {
        SourceFormat::Csv | SourceFormat::Parquet => (PathBuf::from(path), None),
        SourceFormat::Duckdb | SourceFormat::Ducklake => parse_database_table_source(input, path)?,
    };
    Ok(SourceSpec {
        uri: input.to_owned(),
        path,
        format,
        table,
    })
}

fn parse_database_table_source(
    input: &str,
    path: &str,
) -> Result<(PathBuf, Option<String>), Error> {
    let Some((database, table)) = path.rsplit_once('/') else {
        return Err(Error::InvalidArgument(format!(
            "source `{input}` must include a database path and table, for example duckdb://warehouse.duckdb/events"
        )));
    };
    if database.is_empty() || table.is_empty() {
        return Err(Error::InvalidArgument(format!(
            "source `{input}` must include a non-empty database path and table"
        )));
    }
    Ok((PathBuf::from(database), Some(table.to_owned())))
}

fn manifest(plan: &PackPlan, source_sql: &SourceSql, row_count: u64) -> Result<Value, Error> {
    Ok(json!({
        "kind": "pull_manifest",
        "schema_version": "1.0",
        "source": {
            "uri": &plan.source.uri,
            "path": plan.source.path.display().to_string(),
            "format": plan.source.format,
            "table": plan.source.table,
            "setup": source_sql.setup,
            "query": source_sql.query,
        },
        "target": {
            "kind": target_name(plan.target),
            "database": plan.database.display().to_string(),
            "table": &plan.table,
        },
        "row_count": row_count,
        "limit": plan.limit,
        "generated_at_unix_seconds": unix_seconds()?,
    }))
}

fn workspace_database(workspace: &Path, target: LocalTarget) -> PathBuf {
    match target {
        LocalTarget::Duckdb => workspace.join("workspace.duckdb"),
        LocalTarget::Ducklake => workspace.join("lake.ducklake"),
    }
}

fn workspace_manifest(workspace: &Path, table: &str) -> PathBuf {
    workspace
        .join("manifests")
        .join(format!("{}.json", sanitize_path_fragment(table)))
}

fn workspace_run_manifest(workspace: &Path, name: Option<&str>) -> PathBuf {
    let name = name.map_or_else(|| "query".to_owned(), sanitize_path_fragment);
    let millis = unix_millis().unwrap_or(0);
    workspace.join("runs").join(format!("{millis}-{name}.json"))
}

fn write_json(path: &Path, value: &Value) -> Result<(), Error> {
    create_parent_dir(path)?;
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| Error::json_context("serialize JSON", path, error))?;
    fs::write(path, bytes).map_err(|error| Error::io_context("write", path, error))?;
    Ok(())
}

fn read_json(path: &Path) -> Result<Value, Error> {
    let bytes = fs::read(path).map_err(|error| Error::io_context("read", path, error))?;
    serde_json::from_slice(&bytes).map_err(|error| Error::json_context("parse JSON", path, error))
}

fn quote_qualified_identifier(input: &str) -> Result<String, Error> {
    input
        .split('.')
        .map(str::trim)
        .map(quote_identifier)
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("."))
}

fn quote_identifier(input: &str) -> Result<String, Error> {
    if input.is_empty() {
        return Err(Error::InvalidArgument(
            "identifier must not contain empty segments".to_owned(),
        ));
    }
    Ok(format!("\"{}\"", input.replace('"', "\"\"")))
}

fn sql_string_literal(input: &str) -> String {
    format!("'{}'", input.replace('\'', "''"))
}

fn sanitize_path_fragment(input: &str) -> String {
    input
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn create_parent_dir(path: &Path) -> Result<(), Error> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| Error::io_context("create directory", parent, error))?;
    }
    Ok(())
}

fn unix_seconds() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| Error::InvalidArgument(error.to_string()))
}

fn elapsed_ms(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn unix_millis() -> Result<u128, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|error| Error::InvalidArgument(error.to_string()))
}
