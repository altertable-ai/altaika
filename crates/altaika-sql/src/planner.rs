use altaika_foundation::operation::{CatOp, Operation};
use altaika_foundation::path::LakePath;
use thiserror::Error;

use crate::dialect::Dialect;
use crate::filter::{FilterError, Predicate, parse_filter, render_identifier, render_predicate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSql {
    pub logical_plan: LogicalPlan,
    pub canonical_sql: String,
    pub source_dialect: Dialect,
    pub target_dialect: Dialect,
    pub rendered_sql: String,
    pub warnings: Vec<PlanWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogicalPlan {
    List { path: Option<LakePath> },
    Describe { path: LakePath },
    Read(ReadPlan),
    Sql { statement: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPlan {
    pub relation: Relation,
    pub columns: Vec<String>,
    pub predicates: Vec<Predicate>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub namespace: Vec<String>,
    pub object: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanWarning {
    pub message: String,
}

#[derive(Debug, Error)]
pub enum PlanError {
    #[error(transparent)]
    Filter(#[from] FilterError),
    #[error("unsupported typed operation for SQL planning")]
    Unsupported,
    #[error("dialect transpilation failed: {0}")]
    Transpile(String),
}

pub fn plan_operation(
    operation: &Operation,
    source_dialect: Dialect,
    target_dialect: Dialect,
) -> Result<PlannedSql, PlanError> {
    let logical_plan = logical_plan(operation)?;
    let canonical_sql = render_logical_plan(&logical_plan)?;

    let rendered_sql = if source_dialect == target_dialect {
        canonical_sql.clone()
    } else {
        render_with_polyglot(&canonical_sql, source_dialect, target_dialect)?
    };

    Ok(PlannedSql {
        logical_plan,
        canonical_sql,
        source_dialect,
        target_dialect,
        rendered_sql,
        warnings: Vec::new(),
    })
}

fn logical_plan(operation: &Operation) -> Result<LogicalPlan, PlanError> {
    match operation {
        Operation::Ls(op) => Ok(LogicalPlan::List {
            path: op.path.clone(),
        }),
        Operation::Describe(op) => Ok(LogicalPlan::Describe {
            path: op.path.clone(),
        }),
        Operation::Cat(op) => plan_cat(op).map(LogicalPlan::Read),
        Operation::Sql(_) => Err(PlanError::Unsupported),
    }
}

fn render_with_polyglot(
    canonical_sql: &str,
    source_dialect: Dialect,
    target_dialect: Dialect,
) -> Result<String, PlanError> {
    let statements = polyglot_sql::transpile(
        canonical_sql,
        source_dialect.polyglot(),
        target_dialect.polyglot(),
    )
    .map_err(|error| PlanError::Transpile(error.to_string()))?;

    if statements.is_empty() {
        return Err(PlanError::Transpile(
            "polyglot-sql returned no statements".to_string(),
        ));
    }

    Ok(statements.join(";\n"))
}

fn render_logical_plan(plan: &LogicalPlan) -> Result<String, PlanError> {
    match plan {
        LogicalPlan::List { path } => render_ls(path.as_ref()),
        LogicalPlan::Describe { path } => render_describe(path),
        LogicalPlan::Read(plan) => render_read(plan),
        LogicalPlan::Sql { statement } => Ok(statement.clone()),
    }
}

fn render_ls(path: Option<&LakePath>) -> Result<String, PlanError> {
    match path {
        None => Ok("SELECT catalog_name FROM information_schema.schemata".to_string()),
        Some(path) if path.object.is_none() && path.namespace.is_empty() => {
            let filters = catalog_predicate(path, "catalog_name");
            if filters.is_empty() {
                Ok("SELECT schema_name FROM information_schema.schemata".to_string())
            } else {
                Ok(format!(
                    "SELECT schema_name FROM information_schema.schemata WHERE {}",
                    filters.join(" AND ")
                ))
            }
        }
        Some(path) if path.object.is_none() => {
            let filters = table_predicates(path, None, SchemaPredicate::Like);
            Ok(format!(
                "SELECT table_name FROM information_schema.tables WHERE {}",
                filters.join(" AND ")
            ))
        }
        Some(path) => {
            let table = path.object.as_deref().unwrap_or("%");
            let filters = column_predicates(path, table);
            Ok(format!(
                "SELECT column_name, data_type FROM information_schema.columns WHERE {}",
                filters.join(" AND ")
            ))
        }
    }
}

fn render_describe(path: &LakePath) -> Result<String, PlanError> {
    let table = path.object.as_deref().unwrap_or("%");
    let filters = column_predicates(path, table);
    Ok(format!(
        "SELECT column_name, data_type, is_nullable FROM information_schema.columns WHERE {}",
        filters.join(" AND ")
    ))
}

fn plan_cat(op: &CatOp) -> Result<ReadPlan, PlanError> {
    let table = op.path.object.as_deref().ok_or(PlanError::Unsupported)?;
    let predicates = op
        .filters
        .iter()
        .map(|filter| parse_filter(filter))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ReadPlan {
        relation: Relation {
            namespace: relation_namespace(&op.path),
            object: table.to_string(),
        },
        columns: op.columns.clone(),
        predicates,
        limit: op.limit,
    })
}

#[derive(Clone, Copy)]
enum SchemaPredicate {
    Equal,
    Like,
}

fn relation_namespace(path: &LakePath) -> Vec<String> {
    let mut namespace = Vec::new();
    if let Some(catalog) = catalog_name(path) {
        namespace.push(catalog.to_string());
    }
    if path.namespace.is_empty() {
        namespace.push(default_schema(path).to_string());
    } else {
        namespace.extend(path.namespace.clone());
    }
    namespace
}

fn table_predicates(
    path: &LakePath,
    table: Option<&str>,
    schema_predicate: SchemaPredicate,
) -> Vec<String> {
    let mut predicates = catalog_predicate(path, "table_catalog");
    let schema = path
        .namespace
        .last()
        .map_or_else(|| default_schema(path), String::as_str);
    predicates.push(schema_filter("table_schema", schema, schema_predicate));
    if let Some(table) = table {
        predicates.push(format!("table_name = '{}'", escape_literal(table)));
    }
    predicates
}

fn column_predicates(path: &LakePath, table: &str) -> Vec<String> {
    table_predicates(path, Some(table), SchemaPredicate::Equal)
}

fn catalog_predicate(path: &LakePath, column: &str) -> Vec<String> {
    catalog_name(path)
        .map(|catalog| vec![format!("{column} = '{}'", escape_literal(catalog))])
        .unwrap_or_default()
}

fn catalog_name(path: &LakePath) -> Option<&str> {
    path.source.as_deref().filter(|source| *source != "local")
}

fn schema_filter(column: &str, schema: &str, predicate: SchemaPredicate) -> String {
    let schema = escape_literal(schema);
    match predicate {
        SchemaPredicate::Equal => format!("{column} = '{schema}'"),
        SchemaPredicate::Like => format!("{column} LIKE '{schema}'"),
    }
}

fn default_schema(path: &LakePath) -> &'static str {
    if path.source.as_deref() == Some("local") {
        "public"
    } else {
        "main"
    }
}

fn render_read(plan: &ReadPlan) -> Result<String, PlanError> {
    let relation = render_relation(&plan.relation)?;
    let projection = if plan.columns.is_empty() {
        "*".to_string()
    } else {
        plan.columns
            .iter()
            .map(|column| render_identifier(column))
            .collect::<Result<Vec<_>, _>>()?
            .join(", ")
    };
    let filters = plan
        .predicates
        .iter()
        .map(render_predicate)
        .collect::<Result<Vec<_>, _>>()?;
    let where_clause = if filters.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", filters.join(" AND "))
    };

    Ok(format!(
        "SELECT {projection} FROM {relation}{where_clause} LIMIT {}",
        plan.limit
    ))
}

fn render_relation(relation: &Relation) -> Result<String, PlanError> {
    let mut parts = relation
        .namespace
        .iter()
        .map(|part| render_identifier(part))
        .collect::<Result<Vec<_>, _>>()?;
    parts.push(render_identifier(&relation.object)?);
    Ok(parts.join("."))
}

fn escape_literal(input: &str) -> String {
    input.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use altaika_foundation::operation::{CatOp, Operation};
    use altaika_foundation::path::LakePath;

    #[test]
    fn plans_cat_with_projection_filter_and_limit() {
        let op = Operation::Cat(CatOp {
            path: "local/analytics/events".parse::<LakePath>().unwrap(),
            columns: vec!["id".into(), "event_name".into()],
            filters: vec!["event_name:=signup".into()],
            limit: 10,
        });

        let planned = plan_operation(&op, Dialect::DuckDb, Dialect::DataFusion).unwrap();
        assert_eq!(
            planned.canonical_sql,
            "SELECT id, event_name FROM analytics.events WHERE event_name = 'signup' LIMIT 10"
        );
        assert_eq!(planned.target_dialect, Dialect::DataFusion);
        assert!(matches!(planned.logical_plan, LogicalPlan::Read(_)));
    }

    #[test]
    fn plans_cat_with_non_local_source_as_catalog() {
        let op = Operation::Cat(CatOp {
            path: "altertable/main/agent_events".parse::<LakePath>().unwrap(),
            columns: vec!["event".into(), "tool_name".into()],
            filters: Vec::new(),
            limit: 3,
        });

        let planned = plan_operation(&op, Dialect::DuckDb, Dialect::DuckDb).unwrap();

        assert_eq!(
            planned.canonical_sql,
            "SELECT event, tool_name FROM altertable.main.agent_events LIMIT 3"
        );
    }

    #[test]
    fn plans_ls_with_catalog_and_schema_filters() {
        let op = Operation::Ls(altaika_foundation::operation::LsOp {
            path: Some(LakePath {
                source: Some("altertable".to_string()),
                namespace: vec!["main".to_string()],
                object: None,
            }),
            long: false,
            all: false,
        });

        let planned = plan_operation(&op, Dialect::DuckDb, Dialect::DuckDb).unwrap();

        assert_eq!(
            planned.canonical_sql,
            "SELECT table_name FROM information_schema.tables WHERE table_catalog = 'altertable' AND table_schema LIKE 'main'"
        );
    }

    #[test]
    fn plans_describe_with_catalog_schema_and_table_filters() {
        let op = Operation::Describe(altaika_foundation::operation::DescribeOp {
            path: "altertable/main/agent_events".parse::<LakePath>().unwrap(),
        });

        let planned = plan_operation(&op, Dialect::DuckDb, Dialect::DuckDb).unwrap();

        assert_eq!(
            planned.canonical_sql,
            "SELECT column_name, data_type, is_nullable FROM information_schema.columns WHERE table_catalog = 'altertable' AND table_schema = 'main' AND table_name = 'agent_events'"
        );
    }

    #[test]
    fn transpiles_cat_plan_to_named_platform_dialects() {
        let op = Operation::Cat(CatOp {
            path: "local/analytics/events".parse::<LakePath>().unwrap(),
            columns: vec!["id".into()],
            filters: vec!["event_name:=signup".into()],
            limit: 10,
        });

        for target in [
            Dialect::DuckLake,
            Dialect::DataFusion,
            Dialect::PostgreSql,
            Dialect::Snowflake,
            Dialect::Databricks,
        ] {
            let planned = plan_operation(&op, Dialect::DuckDb, target).unwrap();

            assert_eq!(planned.target_dialect, target);
            assert!(!planned.rendered_sql.is_empty());
            assert!(planned.rendered_sql.contains("event_name"));
            polyglot_sql::parse_one(&planned.rendered_sql, target.polyglot()).unwrap();
        }
    }
}
