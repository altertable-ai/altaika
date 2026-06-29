pub mod app_context;
pub mod config;
pub mod flight;

use async_trait::async_trait;
use tokio::sync::Mutex;

use altaika_engine::{Capabilities, Engine, EngineError, RecordStream, SourceProfile};
use altaika_foundation::path::LakePath;
use altaika_sql::dialect::Dialect;
use altaika_sql::planner::PlannedSql;
use config::AltertableConfig;
use flight::FlightClient;

pub struct AltertableEngine {
    // ponytail: Option only supports unit tests without live Flight credentials.
    flight: Mutex<Option<FlightClient>>,
}

impl AltertableEngine {
    pub async fn connect(config: AltertableConfig) -> Result<Self, EngineError> {
        let flight = FlightClient::connect(&config)
            .await
            .map_err(|error| EngineError::Config(error.to_string()))?;
        Ok(Self {
            flight: Mutex::new(Some(flight)),
        })
    }

    #[cfg(test)]
    fn new_for_test() -> Self {
        Self {
            flight: Mutex::new(None),
        }
    }
}

#[async_trait]
impl Engine for AltertableEngine {
    fn name(&self) -> &'static str {
        "altertable"
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

    async fn inspect(&self, path: &LakePath) -> Result<SourceProfile, EngineError> {
        let _ = path;
        Err(EngineError::Config(
            "Altertable metadata inspection is not implemented yet".to_string(),
        ))
    }

    async fn execute(&self, sql: PlannedSql) -> Result<RecordStream, EngineError> {
        let mut flight = self.flight.lock().await;
        let flight = flight.as_mut().ok_or_else(|| {
            EngineError::Config("Altertable Flight SQL client is not connected".to_string())
        })?;
        flight
            .execute_sql(&sql.rendered_sql)
            .await
            .map_err(|error| EngineError::Execution(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use altaika_engine::Engine;

    use super::*;

    #[test]
    fn altertable_engine_declares_product_native_capabilities() {
        let engine = AltertableEngine::new_for_test();

        assert_eq!(engine.name(), "altertable");
        assert_eq!(engine.dialect(), altaika_sql::dialect::Dialect::DuckDb);
        assert!(!engine.capabilities().metadata_api);
        assert!(engine.capabilities().limit_pushdown);
    }

    #[tokio::test]
    async fn altertable_inspect_does_not_return_empty_metadata() {
        let engine = AltertableEngine::new_for_test();

        let error = engine
            .inspect(&"altertable/public/events".parse().unwrap())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("not implemented"));
    }

    #[tokio::test]
    #[ignore = "requires live Altertable Flight SQL credentials"]
    async fn altertable_live_select_one() {
        let config = config::AltertableConfig::from_env().unwrap();
        let engine = AltertableEngine::connect(config).await.unwrap();
        let planned = altaika_sql::planner::PlannedSql {
            logical_plan: altaika_sql::planner::LogicalPlan::Sql {
                statement: "SELECT 1 AS one".to_string(),
            },
            canonical_sql: "SELECT 1 AS one".to_string(),
            source_dialect: altaika_sql::dialect::Dialect::DuckDb,
            target_dialect: altaika_sql::dialect::Dialect::DuckDb,
            rendered_sql: "SELECT 1 AS one".to_string(),
            warnings: Vec::new(),
        };

        let batches = engine.execute(planned).await.unwrap();

        assert_eq!(
            batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
            1
        );
    }
}
