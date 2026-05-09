//! Error types for AGD parse, serialize, and edit operations.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AgdError {
    #[error("parse error at line {line}, col {col}: {message}")]
    Parse {
        line: u32,
        col: u32,
        message: String,
    },

    #[error("duplicate block id `{id}` (first defined at line {first_line}, redefined at line {dup_line})")]
    DuplicateId {
        id: String,
        first_line: u32,
        dup_line: u32,
    },

    #[error("invalid block tag `{tag}` at line {line}: custom tags must be prefixed `x-`")]
    InvalidTag { tag: String, line: u32 },

    #[error("invalid identifier `{id}` at line {line}: must match [a-zA-Z_][a-zA-Z0-9_-]*")]
    InvalidId { id: String, line: u32 },

    #[error("invalid attribute at line {line}: {message}")]
    InvalidAttr { line: u32, message: String },

    #[error("unterminated fence opened at line {line}")]
    UnterminatedFence { line: u32 },

    #[error("dangling reference `#{target}` at line {line}: no block with this id")]
    DanglingRef { target: String, line: u32 },

    #[error("block id `{id}` not found")]
    IdNotFound { id: String },

    #[error("io: {0}")]
    Io(String),

    #[error("{0}")]
    Other(String),
}

impl From<std::io::Error> for AgdError {
    fn from(e: std::io::Error) -> Self {
        AgdError::Io(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AgdError>;
