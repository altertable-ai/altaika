use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use async_trait::async_trait;
use datafusion::prelude::SessionContext;
use serde_json::Value;

use altaika_engine::profile::{ColumnProfile, EngineHints};
use altaika_engine::{Capabilities, Engine, EngineError, RecordStream, SourceProfile};
use altaika_foundation::path::LakePath;
use altaika_sql::dialect::Dialect;
use altaika_sql::planner::PlannedSql;

pub struct DatafusionEngine {
    ctx: SessionContext,
    source_paths: HashMap<String, SourcePath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourcePath {
    uri: String,
    format: String,
}

impl DatafusionEngine {
    pub fn new(ctx: SessionContext) -> Self {
        Self {
            ctx,
            source_paths: HashMap::new(),
        }
    }

    pub fn with_source_path(
        mut self,
        table: impl Into<String>,
        path: impl Into<String>,
        format: impl Into<String>,
    ) -> Self {
        let table = table.into();
        let path = path.into();
        let source_path = SourcePath {
            format: format.into(),
            uri: path,
        };
        if let Some(default_schema_table) = table.strip_prefix("public.") {
            self.source_paths
                .insert(default_schema_table.to_string(), source_path.clone());
        }
        self.source_paths.insert(table, source_path);
        self
    }
}

#[async_trait]
impl Engine for DatafusionEngine {
    fn name(&self) -> &'static str {
        "datafusion"
    }

    fn dialect(&self) -> Dialect {
        Dialect::DataFusion
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            metadata_api: false,
            information_schema: true,
            limit_pushdown: true,
        }
    }

    async fn inspect(&self, path: &LakePath) -> Result<SourceProfile, EngineError> {
        let table_ref = table_ref(path)?;
        let provider = self
            .ctx
            .table_provider(table_ref.as_str())
            .await
            .map_err(|error| EngineError::Execution(error.to_string()))?;
        let schema = provider.schema();
        let columns = schema
            .fields()
            .iter()
            .map(|field| ColumnProfile {
                name: field.name().clone(),
                data_type: field.data_type().to_string(),
                nullable: field.is_nullable(),
            })
            .collect();
        let estimated_rows = provider
            .statistics()
            .and_then(|statistics| statistics.num_rows.get_value().copied())
            .map(|rows| rows as u64)
            .or_else(|| manifest_estimated_rows(self.source_paths.get(&table_ref)));
        let source_path = self.source_paths.get(&table_ref);

        Ok(SourceProfile {
            path: path.clone(),
            columns,
            estimated_rows,
            source_uri: source_path.map(|source_path| source_path.uri.clone()),
            source_format: source_path.map(|source_path| source_path.format.clone()),
            source_size_bytes: source_path.and_then(source_size_bytes),
            partition_keys: Vec::new(),
            clustering_keys: Vec::new(),
            sort_keys: Vec::new(),
            engine_hints: EngineHints::default(),
        })
    }

    async fn execute(&self, sql: PlannedSql) -> Result<RecordStream, EngineError> {
        let dataframe = self
            .ctx
            .sql(&sql.rendered_sql)
            .await
            .map_err(|error| EngineError::Execution(error.to_string()))?;
        dataframe
            .collect()
            .await
            .map_err(|error| EngineError::Execution(error.to_string()))
    }
}

fn manifest_estimated_rows(path: Option<&SourcePath>) -> Option<u64> {
    let path = &path?.uri;
    // ponytail: local snapshot sidecar only. Prefer provider stats when engines expose them.
    let manifest_path = PathBuf::from(format!("{path}.manifest.json"));
    let manifest = fs::read(manifest_path).ok()?;
    let manifest: Value = serde_json::from_slice(&manifest).ok()?;
    manifest.get("row_count")?.as_u64()
}

fn source_size_bytes(path: &SourcePath) -> Option<u64> {
    fs::metadata(&path.uri).ok().map(|metadata| metadata.len())
}

fn table_ref(path: &LakePath) -> Result<String, EngineError> {
    let object = path
        .object
        .as_deref()
        .ok_or_else(|| EngineError::Config("table path must include an object".to_string()))?;
    let mut parts = path.namespace.clone();
    parts.push(object.to_string());
    Ok(parts.join("."))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use altaika_engine::Engine;
    use altaika_foundation::operation::{CatOp, Operation};
    use altaika_sql::dialect::Dialect;
    use altaika_sql::planner::plan_operation;
    use arrow::array::{Int64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use datafusion::prelude::SessionContext;

    use super::*;

    #[tokio::test]
    async fn executes_planned_cat() {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("event_name", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec!["signup", "login"])),
            ],
        )
        .unwrap();
        ctx.register_batch("public.events", batch).unwrap();

        let operation = Operation::Cat(CatOp {
            path: "local/public/events".parse().unwrap(),
            columns: vec!["id".to_string()],
            filters: vec!["event_name:=signup".to_string()],
            limit: 10,
        });
        let planned = plan_operation(&operation, Dialect::DataFusion, Dialect::DataFusion).unwrap();

        let engine = DatafusionEngine::new(ctx);
        let batches = engine.execute(planned).await.unwrap();
        assert_eq!(
            batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
            1
        );
    }

    #[tokio::test]
    async fn inspects_registered_table_columns() {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("event_name", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::new_empty(schema);
        ctx.register_batch("public.events", batch).unwrap();

        let engine = DatafusionEngine::new(ctx);
        let profile = engine
            .inspect(&"local/public/events".parse().unwrap())
            .await
            .unwrap();

        assert_eq!(profile.columns.len(), 2);
        assert_eq!(profile.columns[0].name, "id");
        assert_eq!(profile.columns[0].data_type, "Int64");
        assert!(!profile.columns[0].nullable);
        assert_eq!(profile.columns[1].name, "event_name");
        assert_eq!(profile.columns[1].data_type, "Utf8");
        assert!(profile.columns[1].nullable);
    }
}
