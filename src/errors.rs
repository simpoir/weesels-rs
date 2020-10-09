use std::error::Error as ErrorTrait;
use std::fmt::{Debug, Display};

#[derive(Debug)]
pub enum ErrorKind {
    ConfigError,
}

#[derive(Debug)]
pub struct Error {
    source: Option<Box<dyn ErrorTrait + 'static>>,
    kind: ErrorKind,
}

impl Error {
    pub fn from<E: ErrorTrait + 'static>(kind: ErrorKind, source: E) -> Self {
        Self {
            kind,
            source: Some(Box::new(source)),
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match &self.kind {
            ErrorKind::ConfigError => "Could not read config",
        };
        match &self.source {
            Some(e) => write!(f, "{}: {}", msg, &e),
            None => write!(f, "{}.", msg),
        }
    }
}

impl ErrorTrait for Error {
    fn source(&self) -> Option<&(dyn ErrorTrait + 'static)> {
        match &self.source {
            Some(e) => Some(e.as_ref()),
            None => None,
        }
    }
}
