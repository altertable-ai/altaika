use altaika_foundation::path::LakePath;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceProfile {
    pub path: LakePath,
    pub columns: Vec<ColumnProfile>,
    pub estimated_rows: Option<u64>,
    pub source_uri: Option<String>,
    pub source_format: Option<String>,
    pub source_size_bytes: Option<u64>,
    pub partition_keys: Vec<String>,
    pub clustering_keys: Vec<String>,
    pub sort_keys: Vec<String>,
    pub engine_hints: EngineHints,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnProfile {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EngineHints {
    pub notes: Vec<String>,
}
