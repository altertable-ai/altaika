use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

#[derive(Debug, Parser)]
#[command(
    name = "altaika",
    version,
    about = "Agent data CLI",
    after_long_help = include_str!("../../../docs/skills/commands/root.md")
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        default_value = "datafusion",
        help = "Data engine to use"
    )]
    pub engine: String,
    #[arg(long = "csv", global = true, help = "Register CSV as table=path")]
    pub csv: Vec<String>,
    #[arg(
        long = "parquet",
        global = true,
        help = "Register Parquet as table=path"
    )]
    pub parquet: Vec<String>,
    #[arg(long, global = true, default_value = "json", help = "Output format")]
    pub format: OutputFormat,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Json,
    Ndjson,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(
        about = "Generate shell completions",
        after_long_help = include_str!("../../../docs/skills/commands/completions.md")
    )]
    Completions(CompletionsArgs),
    #[command(
        about = "List catalogs, schemas, tables, or columns",
        after_long_help = include_str!("../../../docs/skills/commands/ls.md")
    )]
    Ls(LsArgs),
    #[command(
        about = "Describe table columns and profile hints",
        after_long_help = include_str!("../../../docs/skills/commands/describe.md")
    )]
    Describe(PathArgs),
    #[command(
        about = "Read bounded rows from a table",
        after_long_help = include_str!("../../../docs/skills/commands/cat.md")
    )]
    Cat(CatArgs),
    #[command(
        about = "Write bounded rows to Parquet with a manifest",
        after_long_help = include_str!("../../../docs/skills/commands/snapshot.md")
    )]
    Snapshot(SnapshotArgs),
    #[command(
        about = "Check runtime credential readiness",
        after_long_help = include_str!("../../../docs/skills/commands/auth.md")
    )]
    Auth(AuthArgs),
    #[command(
        about = "Render SQL without executing it",
        after_long_help = include_str!("../../../docs/skills/commands/plan.md")
    )]
    Plan(PlanArgs),
    #[command(
        about = "Show machine-readable agent affordances",
        after_long_help = include_str!("../../../docs/skills/commands/agent.md")
    )]
    Agent(AgentArgs),
    #[command(
        about = "List embedded agent skills",
        after_long_help = include_str!("../../../docs/skills/commands/skills.md")
    )]
    Skills(SkillsArgs),
    #[command(
        about = "Execute explicit SQL",
        after_long_help = include_str!("../../../docs/skills/commands/sql.md")
    )]
    Sql(SqlArgs),
}

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    #[arg(value_enum, help = "Shell to generate completions for")]
    pub shell: Shell,
}

#[derive(Debug, Args)]
pub struct LsArgs {
    #[arg(help = "Optional path as source/schema/table")]
    pub path: Option<String>,
    #[arg(long, help = "Include profile hints where supported")]
    pub long: bool,
    #[arg(long, help = "Include hidden or internal objects where supported")]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct PathArgs {
    #[arg(help = "Table path as source/schema/table")]
    pub path: String,
}

#[derive(Debug, Args)]
pub struct CatArgs {
    #[arg(help = "Table path as source/schema/table")]
    pub path: String,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated columns to return"
    )]
    pub columns: Vec<String>,
    #[arg(long = "filter", help = "Filter predicate like column:=value")]
    pub filters: Vec<String>,
    #[arg(long, default_value_t = 500, help = "Maximum rows to return")]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct SnapshotArgs {
    #[arg(help = "Table path as source/schema/table")]
    pub path: String,
    #[arg(long, help = "Parquet output path")]
    pub out: PathBuf,
    #[arg(long, value_delimiter = ',', help = "Comma-separated columns to write")]
    pub columns: Vec<String>,
    #[arg(long = "filter", help = "Filter predicate like column:=value")]
    pub filters: Vec<String>,
    #[arg(long, default_value_t = 500, help = "Maximum rows to write")]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct AuthArgs {
    #[arg(
        long,
        help = "Run a live connectivity smoke when credentials are present"
    )]
    pub check: bool,
}

#[derive(Debug, Args)]
pub struct PlanArgs {
    #[command(subcommand)]
    pub command: PlanCommand,
}

#[derive(Debug, Subcommand)]
pub enum PlanCommand {
    #[command(
        about = "Render listing SQL",
        after_long_help = include_str!("../../../docs/skills/commands/plan-ls.md")
    )]
    Ls(PlanLsArgs),
    #[command(
        about = "Render schema inspection SQL",
        after_long_help = include_str!("../../../docs/skills/commands/plan-describe.md")
    )]
    Describe(PlanDescribeArgs),
    #[command(
        about = "Render bounded read SQL",
        after_long_help = include_str!("../../../docs/skills/commands/plan-cat.md")
    )]
    Cat(PlanCatArgs),
}

#[derive(Debug, Args)]
pub struct PlanLsArgs {
    #[arg(help = "Optional path as source/schema/table")]
    pub path: Option<String>,
    #[arg(long, help = "Target dialect")]
    pub target: String,
}

#[derive(Debug, Args)]
pub struct PlanDescribeArgs {
    #[arg(help = "Table path as source/schema/table")]
    pub path: String,
    #[arg(long, help = "Target dialect")]
    pub target: String,
}

#[derive(Debug, Args)]
pub struct PlanCatArgs {
    #[arg(help = "Table path as source/schema/table")]
    pub path: String,
    #[arg(long, help = "Target dialect")]
    pub target: String,
    #[arg(long, value_delimiter = ',', help = "Comma-separated columns to read")]
    pub columns: Vec<String>,
    #[arg(long = "filter", help = "Filter predicate like column:=value")]
    pub filters: Vec<String>,
    #[arg(long, default_value_t = 500, help = "Maximum rows to read")]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub command: AgentCommand,
}

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    #[command(
        about = "Print machine-readable command schema",
        after_long_help = include_str!("../../../docs/skills/commands/agent-schema.md")
    )]
    Schema(AgentSchemaArgs),
    #[command(
        about = "Print a GitHub issue template for CLI problems",
        after_long_help = include_str!("../../../docs/skills/commands/agent-issue-template.md")
    )]
    IssueTemplate,
}

#[derive(Debug, Args)]
pub struct AgentSchemaArgs {
    #[arg(long, help = "Print the smallest command discovery schema")]
    pub compact: bool,
}

#[derive(Debug, Args)]
pub struct SkillsArgs {
    #[command(subcommand)]
    pub command: SkillsCommand,
}

#[derive(Debug, Subcommand)]
pub enum SkillsCommand {
    #[command(
        about = "Print embedded command skills",
        after_long_help = include_str!("../../../docs/skills/commands/skills-list.md")
    )]
    List,
}

#[derive(Debug, Args)]
pub struct SqlArgs {
    #[arg(long, help = "Engine override for this SQL statement")]
    pub engine: Option<String>,
    #[arg(long, default_value_t = 500, help = "Maximum rows to return")]
    pub limit: usize,
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        num_args = 1..,
        help = "SQL statement"
    )]
    pub statement: Vec<String>,
}
