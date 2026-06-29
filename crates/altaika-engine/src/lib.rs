pub mod capabilities;
pub mod profile;
pub mod record_stream;

use async_trait::async_trait;
use thiserror::Error;

use altaika_foundation::path::LakePath;
use altaika_sql::dialect::Dialect;
use altaika_sql::planner::PlannedSql;

pub use capabilities::Capabilities;
pub use profile::{ColumnProfile, EngineHints, SourceProfile};
pub use record_stream::RecordStream;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("engine configuration error: {0}")]
    Config(String),
    #[error("engine execution error: {0}")]
    Execution(String),
}

#[async_trait]
pub trait Engine: Send + Sync {
    fn name(&self) -> &'static str;
    fn dialect(&self) -> Dialect;
    fn capabilities(&self) -> Capabilities;
    async fn inspect(&self, path: &LakePath) -> Result<SourceProfile, EngineError>;
    async fn execute(&self, sql: PlannedSql) -> Result<RecordStream, EngineError>;
}
