use thiserror::Error;

/// Errors reading or writing a result file.
#[derive(Debug, Error)]
pub enum FormatError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a result file")]
    NotAResultFile,
    #[error("unsupported file type {0:?}")]
    WrongFormat(String),
    #[error("the file was created by a newer version (schema {found}, supported up to {supported})")]
    NewerSchema { found: u32, supported: u32 },
    #[error("unsupported schema version {0}")]
    UnsupportedSchema(u32),
    #[error("invalid ZIP container: {0}")]
    Zip(String),
    #[error("invalid result data: {0}")]
    Invalid(String),
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
}

impl From<zip::result::ZipError> for FormatError {
    fn from(error: zip::result::ZipError) -> Self {
        match error {
            zip::result::ZipError::Io(io) => FormatError::Io(io),
            other => FormatError::Zip(other.to_string()),
        }
    }
}

impl From<serde_json::Error> for FormatError {
    fn from(error: serde_json::Error) -> Self {
        FormatError::Invalid(error.to_string())
    }
}
