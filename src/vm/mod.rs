pub mod builtin;
pub mod compiler;
pub mod module;
pub mod symbols;
pub mod types;

#[derive(Debug)]
pub enum Error {
    TypeError(String),
    SyntaxError(String),
    ReferenceError(String),
    IndexError(String),
    ArgumentError(String),
    InternalError(String),
    RuntimeError(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TypeError(s) => write!(f, "TypeError: {}", s),
            Error::SyntaxError(s) => write!(f, "SyntaxError: {}", s),
            Error::ReferenceError(s) => write!(f, "ReferenceError: {}", s),
            Error::IndexError(s) => write!(f, "IndexError: {}", s),
            Error::ArgumentError(s) => write!(f, "ArgumentError: {}", s),
            Error::InternalError(s) => write!(f, "InternalError: {}", s),
            Error::RuntimeError(s) => write!(f, "RuntimeError: {}", s),
        }
    }
}
