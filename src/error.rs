use std::fmt;

#[derive(Debug)]
pub struct AppExit {
    pub code: i32,
    message: String,
}

impl AppExit {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for AppExit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppExit {}

#[allow(dead_code)]
#[derive(Debug)]
pub enum NbError {
    FileNotFound(String),
    InvalidFormat(String),
    IndexOutOfRange(usize, usize),
    UsageError(String),
}

impl fmt::Display for NbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NbError::FileNotFound(p) => write!(f, "File not found: {p}"),
            NbError::InvalidFormat(msg) => write!(f, "Invalid notebook format: {msg}"),
            NbError::IndexOutOfRange(idx, total) => {
                write!(
                    f,
                    "Cell index {idx} is out of range (notebook has {total} cells)"
                )
            }
            NbError::UsageError(msg) => write!(f, "Usage error: {msg}"),
        }
    }
}

impl std::error::Error for NbError {}
