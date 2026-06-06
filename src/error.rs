use std::fmt;

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
                write!(f, "Cell index {idx} is out of range (notebook has {total} cells)")
            }
            NbError::UsageError(msg) => write!(f, "Usage error: {msg}"),
        }
    }
}

impl std::error::Error for NbError {}
