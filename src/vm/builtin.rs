use crate::vm::symbols::{ContextType, DefineType};
use crate::vm::types::Type;

#[derive(Debug, Copy, Clone)]
#[repr(u8)]
pub enum Builtin {
    Print,
    Type,
    Bool,
    Float,
    Int,
    String,
    Length,
    Byte,
    Rune,
    Println,
    Int64,
    Make,
    Cap,
    Append,
    Copy,
    Delete,
    Clear,
    GcCollect,
    Sprintf,
    Panic,
    Recover,
    Close,
}

impl Builtin {
    pub fn is_void(&self) -> bool {
        match &self {
            Self::Print
            | Self::Println
            | Self::Copy
            | Self::Delete
            | Self::Clear
            | Self::GcCollect
            | Self::Panic
            | Self::Close => true,
            _ => false,
        }
    }
}

pub(crate) fn resolve(name: &str) -> Option<Builtin> {
    match name {
        "print" => Some(Builtin::Print),
        "type" => Some(Builtin::Type),
        "int" => Some(Builtin::Int),
        "int64" => Some(Builtin::Int64),
        "float" => Some(Builtin::Float),
        "bool" => Some(Builtin::Bool),
        "string" => Some(Builtin::String),
        "byte" => Some(Builtin::Byte),
        "len" => Some(Builtin::Length),
        "rune" => Some(Builtin::Rune),
        "println" => Some(Builtin::Println),
        "make" => Some(Builtin::Make),
        "cap" => Some(Builtin::Cap),
        "append" => Some(Builtin::Append),
        "copy" => Some(Builtin::Copy),
        "delete" => Some(Builtin::Delete),
        "clear" => Some(Builtin::Clear),
        "gccollect" => Some(Builtin::GcCollect),
        "sprintf" => Some(Builtin::Sprintf),
        "panic" => Some(Builtin::Panic),
        "recover" => Some(Builtin::Recover),
        "close" => Some(Builtin::Close),
        _ => None,
    }
}

pub fn signature_from_t(t: DefineType) -> Option<DefineType> {
    if !t.is_type() {
        return None;
    }

    let (_, inner_t) = t.as_type();

    match inner_t {
        Type::String => Some(DefineType::Func {
            name: "string".to_string(),
            args: vec![ContextType::Named("s".to_string(), DefineType::String)],
            recv: None,
            rt: Box::new(DefineType::String),
        }),
        Type::I64 => Some(DefineType::Func {
            name: "int64".to_string(),
            args: vec![ContextType::Named("i".to_string(), DefineType::Int)],
            recv: None,
            rt: Box::new(DefineType::String),
        }),
        _ => None,
    }
}
