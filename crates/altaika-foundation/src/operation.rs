use serde::{Deserialize, Serialize};

use crate::path::LakePath;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Ls(LsOp),
    Describe(DescribeOp),
    Cat(CatOp),
    Sql(SqlOp),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LsOp {
    pub path: Option<LakePath>,
    pub long: bool,
    pub all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DescribeOp {
    pub path: LakePath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatOp {
    pub path: LakePath,
    pub columns: Vec<String>,
    pub filters: Vec<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlOp {
    pub engine: String,
    pub statement: String,
    pub limit: usize,
}
