use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Parser)]
#[command(
    name = "altaika",
    version,
    about = "AI CLI tool for local data workspaces"
)]
pub struct Cli {
    #[arg(
        long,
        env = "ALTAIKA_WORKSPACE",
        global = true,
        default_value = ".altaika",
        help = "Local workspace directory"
    )]
    pub workspace: PathBuf,
    #[arg(
        long,
        env = "ALTAIKA_PERMISSION",
        global = true,
        value_enum,
        default_value_t = PermissionMode::Permission,
        help = "Agent permission mode: permission=read-only, auto=local writes, allow=explicit remote approval"
    )]
    pub permission: PermissionMode,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Create a local DuckDB and DuckLake workspace")]
    Init(InitArgs),
    #[command(
        about = "Explain routing and safety reasoning for a source",
        aliases = ["plan", "route"]
    )]
    Explain(RouteArgs),
    #[command(about = "Pull bounded source data into the local workspace")]
    Pull(PullArgs),
    #[command(
        about = "Run SQL locally or against explicit remote DuckDB",
        alias = "run"
    )]
    Query(ExecArgs),
    #[command(about = "Explain the local or remote Altaika runtime setup")]
    Doctor(DoctorArgs),
    #[command(about = "List tables locally or through explicit remote DuckDB")]
    Ls(InspectArgs),
    #[command(about = "Describe a table locally or through explicit remote DuckDB")]
    Describe(TableInspectArgs),
    #[command(
        about = "Show one table or object locally or through explicit remote DuckDB",
        alias = "cat"
    )]
    Show(CatArgs),
    #[command(
        about = "Inspect raw workspace facts for agents",
        aliases = ["status"]
    )]
    Inspect,
    #[command(about = "Serve or query explicit remote DuckDB through Quack")]
    Quack(QuackArgs),
    #[command(about = "Install agent skills for this CLI")]
    Skills(SkillsArgs),
}

#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(long = "local-target", alias = "target", value_enum, default_value_t = LocalTarget::Ducklake, help = "Default local DuckDB or DuckLake target")]
    pub target: LocalTarget,
}

#[derive(Debug, Args)]
pub struct RouteArgs {
    #[arg(help = "Source URI, for example csv://events.csv or bigquery://project.dataset.table")]
    pub source: String,
    #[arg(long = "local-target", alias = "target", value_enum, default_value_t = LocalTarget::Ducklake, help = "Preferred local DuckDB or DuckLake target")]
    pub target: LocalTarget,
    #[arg(
        long,
        alias = "bytes",
        help = "Estimated bytes that the source plan would read or return"
    )]
    pub estimated_bytes: Option<u64>,
    #[arg(
        long,
        alias = "rows",
        help = "Estimated rows that the source plan would read or return"
    )]
    pub estimated_rows: Option<u64>,
    #[arg(
        long = "source-latency-ms",
        alias = "latency-ms",
        help = "Estimated source latency in milliseconds"
    )]
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Args)]
pub struct PullArgs {
    #[arg(
        help = "Source URI, for example csv://events.csv, parquet://events.parquet, duckdb://warehouse.duckdb/events, or ducklake://lake.ducklake/main.events"
    )]
    pub source: String,
    #[arg(long, help = "Target local table name")]
    pub table: String,
    #[arg(long = "local-target", alias = "target", value_enum, default_value_t = LocalTarget::Ducklake, help = "Local DuckDB or DuckLake target to load")]
    pub target: LocalTarget,
    #[arg(long, value_delimiter = ',', help = "Comma-separated columns to load")]
    pub columns: Vec<String>,
    #[arg(long, help = "SQL predicate without WHERE")]
    pub filter: Option<String>,
    #[arg(long, default_value_t = 500, help = "Maximum rows to pull")]
    pub limit: usize,
    #[arg(long, value_enum, help = "Optional remote source connector")]
    pub connector: Option<Connector>,
    #[arg(long, help = "Billing project for BigQuery connector reads")]
    pub billing_project: Option<String>,
    #[arg(long, help = "Maximum bytes billed for remote source reads")]
    pub max_bytes_billed: Option<u64>,
}

#[derive(Debug, Args)]
pub struct ExecArgs {
    #[arg(long, value_enum, default_value_t = SqlEngine::Ducklake, help = "SQL engine")]
    pub engine: SqlEngine,
    #[arg(long, value_enum, default_value_t = ExecutionMode::Local, help = "Run locally or remotely")]
    pub mode: ExecutionMode,
    #[arg(
        long,
        help = "Remote endpoint. Quack endpoints may use quack://host:port or quack:host:port"
    )]
    pub remote: Option<String>,
    #[arg(
        long,
        env = "ALTAIKA_QUACK_TOKEN",
        hide_env_values = true,
        help = "Quack auth token"
    )]
    pub quack_token: Option<String>,
    #[arg(
        long = "quack-disable-ssl",
        alias = "disable-ssl",
        help = "Disable SSL verification for Quack"
    )]
    pub disable_ssl: bool,
    #[arg(long, help = "Read SQL from a local file")]
    pub file: Option<PathBuf>,
    #[arg(
        long = "manifest-out",
        alias = "manifest",
        help = "Manifest output path"
    )]
    pub manifest: Option<PathBuf>,
    #[arg(long, help = "Human-readable run name for the manifest path")]
    pub name: Option<String>,
    #[arg(long, help = "Include the full SQL statement in the run manifest")]
    pub record_sql: bool,
    #[arg(long, help = "Run EXPLAIN and skip execution manifests")]
    pub dry_run: bool,
    #[arg(
        long,
        default_value_t = 1000,
        help = "Maximum rows to return in JSON output"
    )]
    pub max_rows: usize,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0.., help = "SQL statement")]
    pub statement: Vec<String>,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    #[arg(long, value_enum, default_value_t = ExecutionMode::Local, help = "Check local setup or a remote Quack endpoint")]
    pub mode: ExecutionMode,
    #[arg(
        long,
        help = "Remote endpoint. Quack endpoints may use quack://host:port or quack:host:port"
    )]
    pub remote: Option<String>,
    #[arg(
        long,
        env = "ALTAIKA_QUACK_TOKEN",
        hide_env_values = true,
        help = "Quack auth token"
    )]
    pub quack_token: Option<String>,
    #[arg(
        long = "quack-disable-ssl",
        alias = "disable-ssl",
        help = "Disable SSL verification for Quack"
    )]
    pub disable_ssl: bool,
}

#[derive(Clone, Debug, Args)]
pub struct InspectArgs {
    #[arg(long, value_enum, default_value_t = SqlEngine::Ducklake, help = "SQL engine")]
    pub engine: SqlEngine,
    #[arg(long, value_enum, default_value_t = ExecutionMode::Local, help = "Inspect locally or remotely")]
    pub mode: ExecutionMode,
    #[arg(
        long,
        help = "Remote endpoint. Quack endpoints may use quack://host:port or quack:host:port"
    )]
    pub remote: Option<String>,
    #[arg(
        long,
        env = "ALTAIKA_QUACK_TOKEN",
        hide_env_values = true,
        help = "Quack auth token"
    )]
    pub quack_token: Option<String>,
    #[arg(
        long = "quack-disable-ssl",
        alias = "disable-ssl",
        help = "Disable SSL verification for Quack"
    )]
    pub disable_ssl: bool,
}

#[derive(Debug, Args)]
pub struct TableInspectArgs {
    #[command(flatten)]
    pub inspect: InspectArgs,
    #[arg(help = "Table name, optionally schema-qualified")]
    pub table: String,
}

#[derive(Debug, Args)]
pub struct CatArgs {
    #[command(flatten)]
    pub inspect: InspectArgs,
    #[arg(help = "Table name, optionally schema-qualified")]
    pub table: String,
    #[arg(long, default_value_t = 20, help = "Maximum rows to return")]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct QuackArgs {
    #[command(subcommand)]
    pub command: QuackCommand,
}

#[derive(Debug, Subcommand)]
pub enum QuackCommand {
    #[command(about = "Serve the local DuckDB or DuckLake workspace through Quack")]
    Serve(QuackServeArgs),
    #[command(about = "Run SQL against an explicit remote DuckDB through Quack")]
    Query(QuackQueryArgs),
}

#[derive(Debug, Args)]
pub struct QuackServeArgs {
    #[arg(long, value_enum, default_value_t = SqlEngine::Duckdb, help = "Local engine to expose through Quack")]
    pub engine: SqlEngine,
    #[arg(
        long,
        default_value = "quack:localhost:6544",
        help = "Quack server URI to bind, for example quack:localhost:6544"
    )]
    pub remote: String,
    #[arg(
        long,
        env = "ALTAIKA_QUACK_TOKEN",
        hide_env_values = true,
        help = "Token clients must provide"
    )]
    pub quack_token: Option<String>,
    #[arg(
        long,
        help = "Allow binding hostnames other than localhost. Use only behind trusted network controls"
    )]
    pub allow_other_hostname: bool,
    #[arg(
        long = "quack-disable-ssl",
        alias = "disable-ssl",
        help = "Disable SSL for local or controlled testing"
    )]
    pub disable_ssl: bool,
}

#[derive(Debug, Args)]
pub struct QuackQueryArgs {
    #[arg(
        long,
        help = "Remote endpoint. Quack endpoints may use quack://host:port or quack:host:port"
    )]
    pub remote: String,
    #[arg(
        long,
        env = "ALTAIKA_QUACK_TOKEN",
        hide_env_values = true,
        help = "Quack auth token"
    )]
    pub quack_token: Option<String>,
    #[arg(
        long = "quack-disable-ssl",
        alias = "disable-ssl",
        help = "Disable SSL verification for local or controlled testing"
    )]
    pub disable_ssl: bool,
    #[arg(long, help = "Read SQL from a local file")]
    pub file: Option<PathBuf>,
    #[arg(
        long,
        default_value_t = 1000,
        help = "Maximum rows to return in JSON output"
    )]
    pub max_rows: usize,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0.., help = "SQL statement")]
    pub statement: Vec<String>,
}

#[derive(Debug, Args)]
pub struct SkillsArgs {
    #[command(subcommand)]
    pub command: SkillsCommand,
}

#[derive(Debug, Subcommand)]
pub enum SkillsCommand {
    #[command(about = "Install the Altaika agent skill into a local skills directory")]
    Install(SkillsInstallArgs),
}

#[derive(Debug, Args)]
pub struct SkillsInstallArgs {
    #[arg(
        long,
        env = "ALTAIKA_SKILLS_DIR",
        help = "Skills root directory. Defaults to AGENTS_HOME/skills, CODEX_HOME/skills, or ~/.agents/skills"
    )]
    pub target_dir: Option<PathBuf>,
    #[arg(
        long,
        help = "Show the target path and content hash without writing files"
    )]
    pub dry_run: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum LocalTarget {
    Duckdb,
    Ducklake,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    Csv,
    Parquet,
    Duckdb,
    Ducklake,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum SqlEngine {
    Duckdb,
    Ducklake,
    Datafusion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Connector {
    Adbc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Local,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Permission,
    Auto,
    Allow,
}

#[derive(Clone, Debug)]
pub struct SourceSpec {
    pub uri: String,
    pub path: PathBuf,
    pub format: SourceFormat,
    pub table: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PackPlan {
    pub source: SourceSpec,
    pub target: LocalTarget,
    pub database: PathBuf,
    pub table: String,
    pub manifest: PathBuf,
    pub columns: Vec<String>,
    pub filter: Option<String>,
    pub limit: usize,
}

pub struct SourceSql {
    pub setup: String,
    pub query: String,
}

#[derive(Clone, Copy, Debug)]
pub struct RouteSignals {
    pub estimated_bytes: Option<u64>,
    pub estimated_rows: Option<u64>,
    pub latency_ms: Option<u64>,
    pub observed_file_size_bytes: Option<u64>,
}

pub struct SqlExecution {
    pub rows: Value,
    pub rows_returned: usize,
    pub rows_truncated: bool,
    pub database: PathBuf,
    pub remote: Value,
    pub remote_calls: u64,
    pub skill: &'static str,
}

pub struct ExecManifestInput<'a> {
    pub args: &'a ExecArgs,
    pub database: &'a Path,
    pub statement: &'a str,
    pub statement_source: &'a str,
    pub statement_file: Option<&'a Path>,
    pub rows_returned: usize,
    pub statement_hash: &'a str,
    pub remote: &'a Value,
}

#[must_use]
pub const fn target_name(target: LocalTarget) -> &'static str {
    match target {
        LocalTarget::Duckdb => "duckdb",
        LocalTarget::Ducklake => "ducklake",
    }
}

#[must_use]
pub const fn source_format_name(format: SourceFormat) -> &'static str {
    match format {
        SourceFormat::Csv => "csv",
        SourceFormat::Parquet => "parquet",
        SourceFormat::Duckdb => "duckdb",
        SourceFormat::Ducklake => "ducklake",
    }
}

#[must_use]
pub const fn engine_name(engine: SqlEngine) -> &'static str {
    match engine {
        SqlEngine::Duckdb => "duckdb",
        SqlEngine::Ducklake => "ducklake",
        SqlEngine::Datafusion => "datafusion",
    }
}

#[must_use]
pub const fn connector_name(connector: Connector) -> &'static str {
    match connector {
        Connector::Adbc => "adbc",
    }
}

#[must_use]
pub const fn mode_name(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::Local => "local",
        ExecutionMode::Remote => "remote",
    }
}

#[must_use]
pub const fn permission_mode_name(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Permission => "permission",
        PermissionMode::Auto => "auto",
        PermissionMode::Allow => "allow",
    }
}
