mod args;
mod duckdb_runtime;
mod output;

use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use arrow::record_batch::RecordBatch;
use clap::{CommandFactory, Parser};
use clap_complete::{Shell, generate};
use datafusion::catalog::memory::MemorySchemaProvider;
use datafusion::prelude::{CsvReadOptions, ParquetReadOptions, SessionConfig, SessionContext};
use parquet::arrow::ArrowWriter;
use serde_json::{Value, json};

use altaika_engine::profile::ColumnProfile;
use altaika_engine::{Engine, SourceProfile};
use altaika_engine_altertable::{AltertableEngine, config::AltertableConfig};
use altaika_engine_datafusion::DatafusionEngine;
use altaika_foundation::error::AltaikaError;
use altaika_foundation::operation::{CatOp, DescribeOp, LsOp, Operation, SqlOp};
use altaika_foundation::path::{LakePath, PathError};
use altaika_sql::dialect::Dialect;
use altaika_sql::planner::{LogicalPlan, PlannedSql, plan_operation};
use args::{AgentCommand, Cli, Command, OutputFormat, PlanCommand, SkillsCommand};

enum Work {
    Operation(Operation),
    Snapshot {
        op: CatOp,
        out: PathBuf,
    },
    Auth {
        check: bool,
    },
    Plan {
        operation: Operation,
        target: Dialect,
    },
    AgentSchema {
        compact: bool,
    },
    AgentIssueTemplate,
    SkillsList,
}

struct SourceRegistration {
    table: String,
    path: String,
    format: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::try_parse().unwrap_or_else(|error| {
        if !error.use_stderr() {
            error.exit();
        }
        exit_with_error(AltaikaError::InvalidArgument(error.to_string()))
    });
    if let Command::Completions(args) = &cli.command {
        print_completions(args.shell);
        return;
    }
    let output_format = cli.format;
    match run(cli).await {
        Ok(value) => print_output(value, output_format),
        Err(error) => exit_with_error(error),
    }
}

fn print_completions(shell: Shell) {
    let mut command = Cli::command();
    let name = command.get_name().to_string();
    generate(shell, &mut command, name, &mut std::io::stdout());
}

fn print_output(value: Value, format: OutputFormat) {
    match format {
        OutputFormat::Json => println!("{value}"),
        OutputFormat::Ndjson => {
            if let Some(rows) = value.get("data").and_then(Value::as_array) {
                for row in rows {
                    println!("{row}");
                }
            } else {
                println!("{value}");
            }
        }
    }
}

fn exit_with_error(error: AltaikaError) -> ! {
    let exit_code = error.exit_code();
    let envelope = error.envelope();
    match serde_json::to_string(&envelope) {
        Ok(json) => eprintln!("{json}"),
        Err(_) => eprintln!(r#"{{"error_code":"internal_error","exit_code":1}}"#),
    }
    std::process::exit(exit_code.into());
}

async fn run(cli: Cli) -> Result<Value, AltaikaError> {
    let Cli {
        engine,
        csv,
        parquet,
        format: _,
        command,
    } = cli;
    let (work, engine_name) = work_and_engine(command, engine)?;
    if let Work::Auth { check } = &work {
        return auth_status(&engine_name, *check).await;
    }
    if let Work::Plan { operation, target } = &work
        && engine_name != "datafusion"
    {
        return plan_status(&engine_name, operation, *target, None);
    }
    if let Work::AgentSchema { compact } = &work {
        return Ok(if *compact {
            compact_agent_schema()
        } else {
            agent_schema()
        });
    }
    if matches!(&work, Work::AgentIssueTemplate) {
        return Ok(agent_issue_template());
    }
    if matches!(&work, Work::SkillsList) {
        return Ok(skills_list());
    }

    match engine_name.as_str() {
        "datafusion" => {
            let ctx =
                SessionContext::new_with_config(SessionConfig::new().with_information_schema(true));
            let sources = register_sources(&ctx, csv, parquet).await?;
            let engine = sources
                .into_iter()
                .fold(DatafusionEngine::new(ctx), |engine, source| {
                    engine.with_source_path(source.table, source.path, source.format)
                });
            if let Work::Plan { operation, target } = work {
                let profile = source_profile_for_plan(&engine, &operation).await;
                return plan_status(&engine_name, &operation, target, profile);
            }
            execute_work(&engine, work).await
        }
        "altertable" => {
            if !csv.is_empty() || !parquet.is_empty() {
                return Err(AltaikaError::InvalidArgument(
                    "local --csv and --parquet sources are only supported with datafusion"
                        .to_string(),
                ));
            }
            let config = AltertableConfig::from_env().map_err(AltaikaError::InvalidArgument)?;
            let engine = AltertableEngine::connect(config)
                .await
                .map_err(|error| AltaikaError::Engine(error.to_string()))?;
            execute_work(&engine, work).await
        }
        "duckdb-beta" => {
            if !csv.is_empty() || !parquet.is_empty() {
                return Err(AltaikaError::InvalidArgument(
                    "local --csv and --parquet sources are only supported with datafusion"
                        .to_string(),
                ));
            }
            execute_duckdb_beta(work)
        }
        _ => Err(AltaikaError::UnsupportedOperation(format!(
            "engine `{engine_name}` is not implemented yet"
        ))),
    }
}

fn execute_duckdb_beta(work: Work) -> Result<Value, AltaikaError> {
    match work {
        Work::Operation(Operation::Sql(sql)) => duckdb_beta_sql(sql),
        _ => Err(AltaikaError::UnsupportedOperation(
            "duckdb-beta currently supports sql and auth only".to_string(),
        )),
    }
}

fn duckdb_beta_sql(sql: SqlOp) -> Result<Value, AltaikaError> {
    let limit = sql.limit;
    let result = duckdb_runtime::execute_sql(&sql.statement, limit)?;
    let row_count = result.rows.len();
    Ok(json!({
        "kind": "rows",
        "engine": "duckdb-beta",
        "schema_version": "1.0",
        "meta": {
            "row_count": row_count,
            "limit": limit,
            "elapsed_ms": result.elapsed_ms,
            "stats": result.stats,
        },
        "data": result.rows,
    }))
}

async fn execute_work(engine: &impl Engine, work: Work) -> Result<Value, AltaikaError> {
    match work {
        Work::Operation(operation) => execute_operation(engine, &operation).await,
        Work::Snapshot { op, out } => execute_snapshot(engine, op, out).await,
        Work::Auth { .. } => Err(AltaikaError::UnsupportedOperation(
            "auth diagnostics do not execute on engines".to_string(),
        )),
        Work::Plan { .. } => Err(AltaikaError::UnsupportedOperation(
            "plan diagnostics do not execute on engines".to_string(),
        )),
        Work::AgentSchema { .. } => Err(AltaikaError::UnsupportedOperation(
            "agent schema does not execute on engines".to_string(),
        )),
        Work::AgentIssueTemplate => Err(AltaikaError::UnsupportedOperation(
            "agent issue template does not execute on engines".to_string(),
        )),
        Work::SkillsList => Err(AltaikaError::UnsupportedOperation(
            "skills list does not execute on engines".to_string(),
        )),
    }
}

async fn execute_operation(
    engine: &impl Engine,
    operation: &Operation,
) -> Result<Value, AltaikaError> {
    if let Operation::Ls(op) = operation
        && op.long
    {
        return execute_long_ls(engine, op).await;
    }

    if let Operation::Describe(op) = operation {
        match engine.inspect(&op.path).await {
            Ok(profile) => {
                let profile_data = profile_data(&profile);
                return Ok(json!({
                    "kind": "schema",
                    "engine": engine.name(),
                    "schema_version": "1.0",
                    "profile": profile_data,
                    "data": columns_data(profile.columns),
                }));
            }
            Err(error) if should_describe_with_information_schema(engine, &error) => {
                return execute_describe_with_information_schema(engine, operation).await;
            }
            Err(error) => return Err(AltaikaError::Engine(error.to_string())),
        }
    }

    let planned = planned_sql(operation, engine.dialect())?;
    let batches = engine
        .execute(planned)
        .await
        .map_err(|error| AltaikaError::Engine(error.to_string()))?;
    let meta = execution_meta(operation, &batches);

    Ok(json!({
        "kind": output_kind(operation),
        "engine": engine.name(),
        "schema_version": "1.0",
        "meta": meta,
        "data": output::batches_to_json_rows(&batches),
    }))
}

fn should_describe_with_information_schema(
    engine: &impl Engine,
    error: &altaika_engine::EngineError,
) -> bool {
    engine.capabilities().information_schema
        && error
            .to_string()
            .to_ascii_lowercase()
            .contains("not implemented")
}

async fn execute_describe_with_information_schema(
    engine: &impl Engine,
    operation: &Operation,
) -> Result<Value, AltaikaError> {
    let planned = planned_sql(operation, engine.dialect())?;
    let batches = engine
        .execute(planned)
        .await
        .map_err(|error| AltaikaError::Engine(error.to_string()))?;
    let rows = output::batches_to_json_rows(&batches);

    Ok(json!({
        "kind": "schema",
        "engine": engine.name(),
        "schema_version": "1.0",
        "profile": Value::Null,
        "data": information_schema_columns_data(rows),
    }))
}

fn execution_meta(operation: &Operation, batches: &[RecordBatch]) -> Value {
    json!({
        "row_count": row_count(batches),
        "limit": operation_limit(operation),
        "columns": batch_columns(batches),
        "profile": Value::Null,
    })
}

fn operation_limit(operation: &Operation) -> Option<usize> {
    match operation {
        Operation::Cat(op) => Some(op.limit),
        Operation::Sql(op) => Some(op.limit),
        Operation::Ls(_) | Operation::Describe(_) => None,
    }
}

async fn execute_long_ls(engine: &impl Engine, op: &LsOp) -> Result<Value, AltaikaError> {
    let planned = planned_sql(&Operation::Ls(op.clone()), engine.dialect())?;
    let batches = engine
        .execute(planned)
        .await
        .map_err(|error| AltaikaError::Engine(error.to_string()))?;
    let rows = output::batches_to_json_rows(&batches);
    let mut data = Vec::with_capacity(rows.len());

    // ponytail: serial inspection keeps the engine contract small. Batch metadata when remote engines expose it.
    for mut row in rows {
        if let Some(table_name) = row.get("table_name").and_then(Value::as_str) {
            let path = long_ls_profile_path(op.path.as_ref(), table_name);
            let profile = engine
                .inspect(&path)
                .await
                .map_err(|error| AltaikaError::Engine(error.to_string()))?;
            if let Some(object) = row.as_object_mut() {
                object.insert("profile".to_string(), profile_with_columns_data(profile));
            }
        }
        data.push(row);
    }

    Ok(json!({
        "kind": "listing",
        "engine": engine.name(),
        "schema_version": "1.0",
        "meta": {
            "row_count": data.len(),
        },
        "data": data,
    }))
}

async fn execute_snapshot(
    engine: &impl Engine,
    op: CatOp,
    out: PathBuf,
) -> Result<Value, AltaikaError> {
    let source = op.path.display();
    let limit = op.limit;
    let source_profile = engine
        .inspect(&op.path)
        .await
        .map_err(|error| AltaikaError::Engine(error.to_string()))?;
    let operation = Operation::Cat(op);
    let planned = planned_sql(&operation, engine.dialect())?;
    let query = planned.rendered_sql.clone();
    let batches = engine
        .execute(planned)
        .await
        .map_err(|error| AltaikaError::Engine(error.to_string()))?;
    let row_count = row_count(&batches);

    // ponytail: Engine::execute already collects batches. Stream when the engine contract does.
    write_parquet(&out, &batches)?;
    let manifest_path = snapshot_manifest_path(&out);
    let manifest = json!({
        "kind": "snapshot_manifest",
        "schema_version": "1.0",
        "engine": engine.name(),
        "source": source,
        "query": query,
        "format": "parquet",
        "data_path": out.display().to_string(),
        "row_count": row_count,
        "limit": limit,
        "generated_at_unix_seconds": generated_at_unix_seconds()?,
        "source_profile": profile_with_columns_data(source_profile),
        "columns": batch_columns(&batches),
    });
    write_json(&manifest_path, &manifest)?;

    Ok(json!({
        "kind": "snapshot",
        "engine": engine.name(),
        "schema_version": "1.0",
        "data": {
            "path": out.display().to_string(),
            "manifest_path": manifest_path.display().to_string(),
            "row_count": row_count,
        },
    }))
}

fn work_and_engine(
    command: Command,
    global_engine: String,
) -> Result<(Work, String), AltaikaError> {
    match command {
        Command::Completions(_) => Err(AltaikaError::UnsupportedOperation(
            "completion generation bypasses engine execution".to_string(),
        )),
        Command::Ls(args) => {
            let path = args
                .path
                .map(|path| path.parse())
                .transpose()
                .map_err(path_error)?
                .map(normalize_ls_path);
            Ok((
                Work::Operation(Operation::Ls(LsOp {
                    path,
                    long: args.long,
                    all: args.all,
                })),
                global_engine,
            ))
        }
        Command::Describe(args) => Ok((
            Work::Operation(Operation::Describe(DescribeOp {
                path: args.path.parse().map_err(path_error)?,
            })),
            global_engine,
        )),
        Command::Cat(args) => Ok((
            Work::Operation(Operation::Cat(CatOp {
                path: args.path.parse().map_err(path_error)?,
                columns: args.columns,
                filters: args.filters,
                limit: args.limit,
            })),
            global_engine,
        )),
        Command::Snapshot(args) => Ok((
            Work::Snapshot {
                op: CatOp {
                    path: args.path.parse().map_err(path_error)?,
                    columns: args.columns,
                    filters: args.filters,
                    limit: args.limit,
                },
                out: args.out,
            },
            global_engine,
        )),
        Command::Auth(args) => Ok((Work::Auth { check: args.check }, global_engine)),
        Command::Plan(args) => plan_work(args.command, global_engine),
        Command::Agent(args) => agent_work(args.command, global_engine),
        Command::Skills(args) => skills_work(args.command, global_engine),
        Command::Sql(args) => Ok((
            Work::Operation(Operation::Sql(SqlOp {
                engine: args.engine.clone().unwrap_or_else(|| global_engine.clone()),
                statement: args.statement.join(" "),
                limit: args.limit,
            })),
            args.engine.unwrap_or(global_engine),
        )),
    }
}

fn agent_work(
    command: AgentCommand,
    global_engine: String,
) -> Result<(Work, String), AltaikaError> {
    match command {
        AgentCommand::Schema(args) => Ok((
            Work::AgentSchema {
                compact: args.compact,
            },
            global_engine,
        )),
        AgentCommand::IssueTemplate => Ok((Work::AgentIssueTemplate, global_engine)),
    }
}

fn skills_work(
    command: SkillsCommand,
    global_engine: String,
) -> Result<(Work, String), AltaikaError> {
    match command {
        SkillsCommand::List => Ok((Work::SkillsList, global_engine)),
    }
}

fn agent_schema() -> Value {
    // ponytail: static v0 schema. Generate from Clap when command churn becomes a problem.
    json!({
        "kind": "agent_schema",
        "schema_version": "1.0",
        "data": {
            "engines": ["datafusion", "altertable", "duckdb-beta"],
            "dialects": ["duckdb", "ducklake", "datafusion", "postgresql", "snowflake", "databricks"],
            "output_formats": ["json", "ndjson"],
            "path_shape": "source[/namespace...][/object]",
            "filter_syntax": [
                "column:=value",
                "column:>value",
                "column:>=value",
                "column:<value",
                "column:<=value"
            ],
            "profile_fields": [
                "path",
                "estimated_rows",
                "source_uri",
                "source_format",
                "source_size_bytes",
                "partition_keys",
                "clustering_keys",
                "sort_keys",
                "engine_hints",
                "columns"
            ],
            "row_meta_fields": [
                "row_count",
                "limit",
                "columns",
                "profile"
            ],
            "commands": [
                {
                    "name": "completions",
                    "purpose": "Generate shell completion scripts for humans and terminal agents.",
                    "output_kind": "completion_script",
                    "example": "altaika completions zsh",
                    "help": "altaika completions --help",
                    "skill": "docs/skills/commands/completions.md"
                },
                {
                    "name": "ls",
                    "purpose": "List catalogs, schemas, tables, or columns. Use --long to include table profile hints.",
                    "output_kind": "listing",
                    "example": "altaika --csv public.events=events.csv ls --long local/public",
                    "help": "altaika ls --help",
                    "skill": "docs/skills/commands/ls.md"
                },
                {
                    "name": "describe",
                    "purpose": "Return schema plus source profile hints without row data.",
                    "output_kind": "schema",
                    "example": "altaika --csv public.events=events.csv describe local/public/events",
                    "help": "altaika describe --help",
                    "skill": "docs/skills/commands/describe.md"
                },
                {
                    "name": "cat",
                    "purpose": "Read bounded rows through a typed operation.",
                    "output_kind": "rows",
                    "example": "altaika --csv public.events=events.csv cat local/events --columns id --limit 10",
                    "help": "altaika cat --help",
                    "skill": "docs/skills/commands/cat.md"
                },
                {
                    "name": "plan ls",
                    "purpose": "Render catalog listing SQL without execution.",
                    "output_kind": "plan",
                    "example": "altaika plan ls local/public --target snowflake",
                    "help": "altaika plan ls --help",
                    "skill": "docs/skills/commands/plan-ls.md"
                },
                {
                    "name": "plan describe",
                    "purpose": "Render schema inspection SQL without execution.",
                    "output_kind": "plan",
                    "example": "altaika plan describe local/public/events --target postgresql",
                    "help": "altaika plan describe --help",
                    "skill": "docs/skills/commands/plan-describe.md"
                },
                {
                    "name": "plan cat",
                    "purpose": "Render canonical and target SQL without execution.",
                    "output_kind": "plan",
                    "example": "altaika plan cat local/events --target snowflake --columns id,event_name --limit 10",
                    "help": "altaika plan cat --help",
                    "skill": "docs/skills/commands/plan-cat.md"
                },
                {
                    "name": "snapshot",
                    "purpose": "Write a bounded local Parquet copy and manifest.",
                    "output_kind": "snapshot",
                    "example": "altaika --csv public.events=events.csv snapshot local/events --out events.parquet --columns id --limit 100",
                    "help": "altaika snapshot --help",
                    "skill": "docs/skills/commands/snapshot.md"
                },
                {
                    "name": "sql",
                    "purpose": "Execute explicit SQL when the typed commands are not enough.",
                    "output_kind": "rows",
                    "example": "altaika sql \"SELECT 1 AS one\"",
                    "help": "altaika sql --help",
                    "skill": "docs/skills/commands/sql.md"
                },
                {
                    "name": "auth",
                    "purpose": "Report runtime credential readiness without secret values.",
                    "output_kind": "auth",
                    "example": "altaika --engine altertable auth",
                    "help": "altaika auth --help",
                    "skill": "docs/skills/commands/auth.md"
                },
                {
                    "name": "agent issue-template",
                    "purpose": "Return a structured GitHub issue template for CLI problems.",
                    "output_kind": "issue_template",
                    "example": "altaika agent issue-template",
                    "help": "altaika agent issue-template --help",
                    "skill": "docs/skills/commands/agent-issue-template.md"
                },
                {
                    "name": "skills list",
                    "purpose": "Return a compact index of embedded command skills.",
                    "output_kind": "skills",
                    "example": "altaika skills list",
                    "help": "altaika skills list --help",
                    "skill": "docs/skills/commands/skills-list.md"
                },
                {
                    "name": "duckdb-beta sql",
                    "purpose": "Run explicit SQL through the configured DuckDB CLI for beta DuckDB features.",
                    "output_kind": "rows",
                    "example": "ALTAIKA_DUCKDB_BIN=$HOME/.duckdb/cli/latest/duckdb altaika --engine duckdb-beta sql \"SELECT 1 AS one\"",
                    "help": "altaika --engine duckdb-beta sql --help",
                    "skill": "docs/skills/commands/sql.md"
                }
            ]
        }
    })
}

fn compact_agent_schema() -> Value {
    let schema = agent_schema();
    let data = &schema["data"];
    let commands = data["commands"]
        .as_array()
        .map(|commands| commands.iter().map(compact_command).collect::<Vec<_>>())
        .unwrap_or_default();

    json!({
        "kind": "agent_schema",
        "schema_version": "1.0",
        "compact": true,
        "data": {
            "engines": data["engines"].clone(),
            "dialects": data["dialects"].clone(),
            "output_formats": data["output_formats"].clone(),
            "commands": commands,
        }
    })
}

fn compact_command(command: &Value) -> Value {
    json!({
        "name": command["name"].clone(),
        "output_kind": command["output_kind"].clone(),
        "help": command["help"].clone(),
        "skill": command["skill"].clone(),
    })
}

fn skills_list() -> Value {
    let schema = agent_schema();
    let skills = schema["data"]["commands"]
        .as_array()
        .map(|commands| commands.iter().map(command_skill).collect::<Vec<_>>())
        .unwrap_or_default();

    json!({
        "kind": "skills",
        "schema_version": "1.0",
        "data": skills,
    })
}

fn command_skill(command: &Value) -> Value {
    json!({
        "name": command["name"].clone(),
        "kind": "command",
        "command": command["help"].clone(),
        "path": command["skill"].clone(),
        "output_kind": command["output_kind"].clone(),
    })
}

fn agent_issue_template() -> Value {
    json!({
        "kind": "issue_template",
        "schema_version": "1.0",
        "data": {
            "repository": "altertable-ai/altaika",
            "title": "CLI behavior needs review",
            "labels": ["cli", "agent-devex"],
            "body": "## Command\n\n```bash\n<command here>\n```\n\n## Engine\n\n`<engine>`\n\n## Expected JSON or behavior\n\n<what should happen>\n\n## Actual JSON or behavior\n\n<what happened>\n\n## Smallest reproducer\n\n<fixture, env redactions, or exact local file shape>\n\n## Notes\n\n- Do not include secrets.\n- Include `altaika --version` when available.\n"
        }
    })
}

fn plan_work(command: PlanCommand, global_engine: String) -> Result<(Work, String), AltaikaError> {
    match command {
        PlanCommand::Ls(args) => {
            let path = args
                .path
                .map(|path| path.parse())
                .transpose()
                .map_err(path_error)?
                .map(normalize_ls_path);
            Ok((
                Work::Plan {
                    operation: Operation::Ls(LsOp {
                        path,
                        long: false,
                        all: false,
                    }),
                    target: parse_dialect(&args.target)?,
                },
                global_engine,
            ))
        }
        PlanCommand::Describe(args) => Ok((
            Work::Plan {
                operation: Operation::Describe(DescribeOp {
                    path: args.path.parse().map_err(path_error)?,
                }),
                target: parse_dialect(&args.target)?,
            },
            global_engine,
        )),
        PlanCommand::Cat(args) => Ok((
            Work::Plan {
                operation: Operation::Cat(CatOp {
                    path: args.path.parse().map_err(path_error)?,
                    columns: args.columns,
                    filters: args.filters,
                    limit: args.limit,
                }),
                target: parse_dialect(&args.target)?,
            },
            global_engine,
        )),
    }
}

fn plan_status(
    engine_name: &str,
    operation: &Operation,
    target: Dialect,
    profile: Option<SourceProfile>,
) -> Result<Value, AltaikaError> {
    let planned = plan_operation(operation, Dialect::DuckDb, target)
        .map_err(|error| AltaikaError::UnsupportedOperation(error.to_string()))?;
    let profile = profile.map(profile_with_columns_data);
    Ok(json!({
        "kind": "plan",
        "engine": engine_name,
        "schema_version": "1.0",
        "data": {
            "source_dialect": dialect_name(planned.source_dialect),
            "target_dialect": dialect_name(planned.target_dialect),
            "canonical_sql": planned.canonical_sql,
            "rendered_sql": planned.rendered_sql,
            "warnings": planned.warnings.into_iter().map(|warning| warning.message).collect::<Vec<_>>(),
            "profile": profile,
        },
    }))
}

async fn source_profile_for_plan(
    engine: &impl Engine,
    operation: &Operation,
) -> Option<SourceProfile> {
    let path = match operation {
        Operation::Cat(op) => &op.path,
        Operation::Describe(op) => &op.path,
        Operation::Ls(op) => op.path.as_ref().filter(|path| path.object.is_some())?,
        Operation::Sql(_) => return None,
    };
    engine.inspect(path).await.ok()
}

fn parse_dialect(input: &str) -> Result<Dialect, AltaikaError> {
    match input {
        "duckdb" => Ok(Dialect::DuckDb),
        "ducklake" => Ok(Dialect::DuckLake),
        "datafusion" => Ok(Dialect::DataFusion),
        "postgres" | "postgresql" => Ok(Dialect::PostgreSql),
        "snowflake" => Ok(Dialect::Snowflake),
        "databricks" => Ok(Dialect::Databricks),
        _ => Err(AltaikaError::InvalidArgument(format!(
            "unsupported target dialect `{input}`"
        ))),
    }
}

fn dialect_name(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::DuckDb => "duckdb",
        Dialect::DuckLake => "ducklake",
        Dialect::DataFusion => "datafusion",
        Dialect::PostgreSql => "postgresql",
        Dialect::Snowflake => "snowflake",
        Dialect::Databricks => "databricks",
    }
}

async fn auth_status(engine_name: &str, check: bool) -> Result<Value, AltaikaError> {
    match engine_name {
        "datafusion" => Ok(json!({
            "kind": "auth",
            "engine": "datafusion",
            "schema_version": "1.0",
            "data": {
                "ready": true,
                "checked": check,
                "connection": if check { "ok" } else { "not_checked" },
                "runtime_only": true,
                "missing": [],
                "present": {},
            },
        })),
        "altertable" => {
            let missing = ["ALTERTABLE_USER", "ALTERTABLE_PASSWORD"]
                .into_iter()
                .filter(|key| !env_present(key))
                .collect::<Vec<_>>();
            let ready_to_check = missing.is_empty();
            let check_result = altertable_check_result(check, ready_to_check).await;
            let check_ok = check_result
                .as_ref()
                .map(|result| result.is_ok())
                .unwrap_or(true);
            Ok(json!({
                "kind": "auth",
                "engine": "altertable",
                "schema_version": "1.0",
                "data": {
                    "ready": ready_to_check && check_ok,
                    "checked": check_result.is_some(),
                    "connection": auth_connection_status(check, ready_to_check, check_result.as_ref()),
                    "check_error": check_result.as_ref().and_then(|result| result.as_ref().err()).map(String::as_str),
                    "runtime_only": true,
                    "missing": missing,
                    "present": {
                        "ALTAIKA_ALTERTABLE_APP_URL": env_present("ALTAIKA_ALTERTABLE_APP_URL"),
                        "ALTAIKA_ALTERTABLE_ENVIRONMENT_ID": env_present("ALTAIKA_ALTERTABLE_ENVIRONMENT_ID"),
                        "ALTAIKA_ALTERTABLE_FLIGHT_HOST": env_present("ALTAIKA_ALTERTABLE_FLIGHT_HOST"),
                        "ALTAIKA_ALTERTABLE_INSECURE": env_present("ALTAIKA_ALTERTABLE_INSECURE"),
                        "ALTERTABLE_PASSWORD": env_present("ALTERTABLE_PASSWORD"),
                        "ALTERTABLE_USER": env_present("ALTERTABLE_USER"),
                    },
                },
            }))
        }
        "duckdb-beta" => Ok(duckdb_runtime::auth_status(check)),
        _ => Err(AltaikaError::UnsupportedOperation(format!(
            "engine `{engine_name}` is not implemented yet"
        ))),
    }
}

async fn altertable_check_result(check: bool, ready_to_check: bool) -> Option<Result<(), String>> {
    if !check || !ready_to_check {
        return None;
    }
    Some(altertable_live_check().await)
}

async fn altertable_live_check() -> Result<(), String> {
    let config = AltertableConfig::from_env()?;
    let engine = AltertableEngine::connect(config)
        .await
        .map_err(|error| error.to_string())?;
    let planned = planned_sql(
        &Operation::Sql(SqlOp {
            engine: "altertable".to_string(),
            statement: "SELECT 1 AS one".to_string(),
            limit: 1,
        }),
        engine.dialect(),
    )
    .map_err(|error| error.to_string())?;
    engine
        .execute(planned)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn auth_connection_status(
    check: bool,
    ready_to_check: bool,
    check_result: Option<&Result<(), String>>,
) -> &'static str {
    match (check, ready_to_check, check_result) {
        (false, _, _) => "not_checked",
        (true, false, _) => "skipped",
        (true, true, Some(Ok(()))) => "ok",
        (true, true, Some(Err(_))) => "failed",
        (true, true, None) => "skipped",
    }
}

fn env_present(key: &str) -> bool {
    env::var(key).is_ok_and(|value| !value.trim().is_empty())
}

fn normalize_ls_path(mut path: LakePath) -> LakePath {
    if path.namespace.is_empty()
        && let Some(object) = path.object.take()
    {
        path.namespace.push(object);
    }
    path
}

async fn register_sources(
    ctx: &SessionContext,
    csv: Vec<String>,
    parquet: Vec<String>,
) -> Result<Vec<SourceRegistration>, AltaikaError> {
    let mut sources = Vec::with_capacity(csv.len() + parquet.len());
    for registration in csv {
        let (table, path) = parse_registration(&registration)?;
        ensure_registration_schema(ctx, table)?;
        ctx.register_csv(
            table,
            path,
            CsvReadOptions::new().file_extension(file_extension(path)),
        )
        .await
        .map_err(|error| AltaikaError::Engine(error.to_string()))?;
        sources.push(SourceRegistration {
            table: table.to_string(),
            path: path.to_string(),
            format: "csv".to_string(),
        });
    }

    for registration in parquet {
        let (table, path) = parse_registration(&registration)?;
        ensure_registration_schema(ctx, table)?;
        ctx.register_parquet(
            table,
            path,
            ParquetReadOptions::default().file_extension(file_extension(path)),
        )
        .await
        .map_err(|error| AltaikaError::Engine(error.to_string()))?;
        sources.push(SourceRegistration {
            table: table.to_string(),
            path: path.to_string(),
            format: "parquet".to_string(),
        });
    }

    Ok(sources)
}

fn ensure_registration_schema(ctx: &SessionContext, table: &str) -> Result<(), AltaikaError> {
    let parts = table.split('.').map(str::trim).collect::<Vec<_>>();
    if parts.iter().any(|part| part.is_empty()) {
        return Err(AltaikaError::InvalidArgument(format!(
            "source table `{table}` must not contain empty name parts"
        )));
    }

    match parts.as_slice() {
        [_table] => Ok(()),
        [schema, _table] => ensure_catalog_schema(ctx, "datafusion", schema),
        [catalog, schema, _table] => ensure_catalog_schema(ctx, catalog, schema),
        _ => Err(AltaikaError::InvalidArgument(format!(
            "source table `{table}` must be table, schema.table, or catalog.schema.table"
        ))),
    }
}

fn ensure_catalog_schema(
    ctx: &SessionContext,
    catalog_name: &str,
    schema_name: &str,
) -> Result<(), AltaikaError> {
    let catalog = ctx.catalog(catalog_name).ok_or_else(|| {
        AltaikaError::Engine(format!(
            "datafusion catalog `{catalog_name}` is not registered"
        ))
    })?;
    if catalog.schema(schema_name).is_none() {
        catalog
            .register_schema(schema_name, Arc::new(MemorySchemaProvider::new()))
            .map_err(|error| AltaikaError::Engine(error.to_string()))?;
    }
    Ok(())
}

fn file_extension(path: &str) -> &str {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
}

fn parse_registration(input: &str) -> Result<(&str, &str), AltaikaError> {
    let Some((table, path)) = input.split_once('=') else {
        return Err(AltaikaError::InvalidArgument(format!(
            "source registration `{input}` must use table=path"
        )));
    };
    if table.trim().is_empty() || path.trim().is_empty() {
        return Err(AltaikaError::InvalidArgument(format!(
            "source registration `{input}` must use non-empty table and path"
        )));
    }
    Ok((table.trim(), path.trim()))
}

fn planned_sql(operation: &Operation, dialect: Dialect) -> Result<PlannedSql, AltaikaError> {
    match operation {
        Operation::Sql(sql) if sql.limit > 0 => {
            let statement = bounded_sql_statement(&sql.statement, sql.limit);
            Ok(PlannedSql {
                logical_plan: LogicalPlan::Sql {
                    statement: statement.clone(),
                },
                canonical_sql: statement.clone(),
                source_dialect: dialect,
                target_dialect: dialect,
                rendered_sql: statement,
                warnings: Vec::new(),
            })
        }
        Operation::Sql(sql) => Ok(PlannedSql {
            logical_plan: LogicalPlan::Sql {
                statement: sql.statement.clone(),
            },
            canonical_sql: sql.statement.clone(),
            source_dialect: dialect,
            target_dialect: dialect,
            rendered_sql: sql.statement.clone(),
            warnings: Vec::new(),
        }),
        _ => plan_operation(operation, dialect, dialect)
            .map_err(|error| AltaikaError::UnsupportedOperation(error.to_string())),
    }
}

fn bounded_sql_statement(statement: &str, limit: usize) -> String {
    let statement = statement.trim().trim_end_matches(';').trim();
    format!("SELECT * FROM ({statement}) AS altaika_sql LIMIT {limit}")
}

fn output_kind(operation: &Operation) -> &'static str {
    match operation {
        Operation::Ls(_) => "listing",
        Operation::Describe(_) => "schema",
        Operation::Cat(_) | Operation::Sql(_) => "rows",
    }
}

fn columns_data(columns: Vec<ColumnProfile>) -> Vec<Value> {
    columns
        .into_iter()
        .map(|column| {
            json!({
                "name": column.name,
                "data_type": column.data_type,
                "nullable": column.nullable,
            })
        })
        .collect()
}

fn information_schema_columns_data(rows: Vec<Value>) -> Vec<Value> {
    rows.into_iter()
        .map(|row| {
            json!({
                "name": row.get("column_name").and_then(Value::as_str).unwrap_or_default(),
                "data_type": row.get("data_type").and_then(Value::as_str).unwrap_or_default(),
                "nullable": row.get("is_nullable").and_then(nullable_value).unwrap_or(false),
            })
        })
        .collect()
}

fn nullable_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(value) if value.eq_ignore_ascii_case("YES") => Some(true),
        Value::String(value) if value.eq_ignore_ascii_case("NO") => Some(false),
        _ => None,
    }
}

fn profile_data(profile: &SourceProfile) -> Value {
    json!({
        "path": profile.path.display(),
        "estimated_rows": profile.estimated_rows,
        "source_uri": profile.source_uri.as_deref(),
        "source_format": profile.source_format.as_deref(),
        "source_size_bytes": profile.source_size_bytes,
        "partition_keys": profile.partition_keys,
        "clustering_keys": profile.clustering_keys,
        "sort_keys": profile.sort_keys,
        "engine_hints": profile.engine_hints.notes,
    })
}

fn profile_with_columns_data(profile: SourceProfile) -> Value {
    json!({
        "path": profile.path.display(),
        "estimated_rows": profile.estimated_rows,
        "source_uri": profile.source_uri,
        "source_format": profile.source_format,
        "source_size_bytes": profile.source_size_bytes,
        "partition_keys": profile.partition_keys,
        "clustering_keys": profile.clustering_keys,
        "sort_keys": profile.sort_keys,
        "engine_hints": profile.engine_hints.notes,
        "columns": columns_data(profile.columns),
    })
}

fn long_ls_profile_path(base: Option<&LakePath>, table_name: &str) -> LakePath {
    let mut path = base.cloned().unwrap_or(LakePath {
        source: None,
        namespace: Vec::new(),
        object: None,
    });
    path.object = Some(table_name.to_string());
    path
}

fn write_parquet(path: &Path, batches: &[RecordBatch]) -> Result<(), AltaikaError> {
    let Some(first_batch) = batches.first() else {
        return Err(AltaikaError::Engine(
            "snapshot query returned no schema".to_string(),
        ));
    };
    create_parent_dir(path)?;
    let file = File::create(path).map_err(|error| AltaikaError::Engine(error.to_string()))?;
    let mut writer = ArrowWriter::try_new(file, first_batch.schema(), None)
        .map_err(|error| AltaikaError::Engine(error.to_string()))?;
    for batch in batches {
        writer
            .write(batch)
            .map_err(|error| AltaikaError::Engine(error.to_string()))?;
    }
    writer
        .close()
        .map_err(|error| AltaikaError::Engine(error.to_string()))?;
    Ok(())
}

fn write_json(path: &Path, value: &Value) -> Result<(), AltaikaError> {
    create_parent_dir(path)?;
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| AltaikaError::Engine(error.to_string()))?;
    fs::write(path, bytes).map_err(|error| AltaikaError::Engine(error.to_string()))
}

fn create_parent_dir(path: &Path) -> Result<(), AltaikaError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| AltaikaError::Engine(error.to_string()))?;
    }
    Ok(())
}

fn snapshot_manifest_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.manifest.json", path.display()))
}

fn row_count(batches: &[RecordBatch]) -> usize {
    batches.iter().map(RecordBatch::num_rows).sum()
}

fn batch_columns(batches: &[RecordBatch]) -> Vec<Value> {
    let Some(first_batch) = batches.first() else {
        return Vec::new();
    };
    first_batch
        .schema()
        .fields()
        .iter()
        .map(|field| {
            json!({
                "name": field.name(),
                "data_type": format!("{:?}", field.data_type()),
                "nullable": field.is_nullable(),
            })
        })
        .collect()
}

fn generated_at_unix_seconds() -> Result<u64, AltaikaError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| AltaikaError::Engine(error.to_string()))
}

fn path_error(error: PathError) -> AltaikaError {
    AltaikaError::InvalidArgument(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use altaika_engine::{Capabilities, Engine, EngineError, RecordStream, SourceProfile};
    use altaika_sql::dialect::Dialect;
    use altaika_sql::planner::PlannedSql;
    use arrow::array::StringArray;
    use arrow::datatypes::{DataType, Field, Schema};
    use async_trait::async_trait;

    use super::*;

    #[test]
    fn parses_source_registration() {
        assert_eq!(
            parse_registration("public.events=/tmp/events.csv").unwrap(),
            ("public.events", "/tmp/events.csv")
        );
    }

    #[test]
    fn rejects_source_registration_without_table() {
        let error = parse_registration("/tmp/events.csv").unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn normalizes_two_segment_ls_path_as_namespace() {
        let path = normalize_ls_path("local/public".parse().unwrap());
        assert_eq!(path.source.as_deref(), Some("local"));
        assert_eq!(path.namespace, vec!["public"]);
        assert!(path.object.is_none());
    }

    struct InformationSchemaOnlyEngine;

    #[async_trait]
    impl Engine for InformationSchemaOnlyEngine {
        fn name(&self) -> &'static str {
            "information_schema_only"
        }

        fn dialect(&self) -> Dialect {
            Dialect::DuckDb
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities {
                metadata_api: false,
                information_schema: true,
                limit_pushdown: true,
            }
        }

        async fn inspect(&self, _path: &LakePath) -> Result<SourceProfile, EngineError> {
            Err(EngineError::Config(
                "metadata inspection is not implemented".to_string(),
            ))
        }

        async fn execute(&self, sql: PlannedSql) -> Result<RecordStream, EngineError> {
            assert_eq!(
                sql.rendered_sql,
                "SELECT column_name, data_type, is_nullable FROM information_schema.columns WHERE table_catalog = 'altertable' AND table_schema = 'main' AND table_name = 'agent_events'"
            );
            let schema = Arc::new(Schema::new(vec![
                Field::new("column_name", DataType::Utf8, false),
                Field::new("data_type", DataType::Utf8, false),
                Field::new("is_nullable", DataType::Utf8, false),
            ]));
            let batch = RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(StringArray::from(vec!["event", "duration_ms"])),
                    Arc::new(StringArray::from(vec!["VARCHAR", "UINTEGER"])),
                    Arc::new(StringArray::from(vec!["NO", "YES"])),
                ],
            )
            .unwrap();
            Ok(vec![batch])
        }
    }

    #[tokio::test]
    async fn describe_falls_back_to_information_schema_when_inspect_is_unimplemented() {
        let operation = Operation::Describe(DescribeOp {
            path: "altertable/main/agent_events".parse().unwrap(),
        });

        let output = execute_operation(&InformationSchemaOnlyEngine, &operation)
            .await
            .unwrap();

        assert_eq!(output["kind"], "schema");
        assert_eq!(output["engine"], "information_schema_only");
        assert_eq!(output["profile"], Value::Null);
        assert_eq!(
            output["data"],
            json!([
                {"name": "event", "data_type": "VARCHAR", "nullable": false},
                {"name": "duration_ms", "data_type": "UINTEGER", "nullable": true}
            ])
        );
    }
}
