use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dialect {
    DuckDb,
    DuckLake,
    DataFusion,
    PostgreSql,
    Snowflake,
    Databricks,
}

impl Dialect {
    pub fn polyglot(self) -> polyglot_sql::DialectType {
        match self {
            // ponytail: DuckLake attaches through DuckDB. Split when its SQL rendering differs.
            Self::DuckDb | Self::DuckLake => polyglot_sql::DialectType::DuckDB,
            Self::DataFusion => polyglot_sql::DialectType::DataFusion,
            Self::PostgreSql => polyglot_sql::DialectType::PostgreSQL,
            Self::Snowflake => polyglot_sql::DialectType::Snowflake,
            Self::Databricks => polyglot_sql::DialectType::Databricks,
        }
    }
}
