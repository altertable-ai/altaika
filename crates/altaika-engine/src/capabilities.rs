#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub metadata_api: bool,
    pub information_schema: bool,
    pub limit_pushdown: bool,
}
