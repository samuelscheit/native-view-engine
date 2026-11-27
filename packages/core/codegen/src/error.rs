use std::fmt;

/// Error returned by the bi-directional RN <-> NVE code generator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodegenError {
    /// Lexical or syntactic error while parsing a React Native TypeScript/Flow spec.
    Parse {
        line: usize,
        col: usize,
        message: String,
    },
    /// Semantic or validation error in a parsed RN spec or `NveExportSpec`.
    Validation(String),
    /// Filesystem I/O failure when reading specs or writing generated outputs.
    Io { path: String, message: String },
}

impl CodegenError {
    pub fn parse(line: usize, col: usize, message: impl Into<String>) -> Self {
        Self::Parse {
            line,
            col,
            message: message.into(),
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    pub fn io(path: impl Into<String>, err: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: err.to_string(),
        }
    }
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { line, col, message } => {
                write!(f, "spec parse error at {line}:{col}: {message}")
            }
            Self::Validation(msg) => write!(f, "spec validation error: {msg}"),
            Self::Io { path, message } => write!(f, "I/O error for '{path}': {message}"),
        }
    }
}

impl std::error::Error for CodegenError {}
