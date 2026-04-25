#[derive(Debug, PartialEq, Copy, Clone, PartialOrd, Ord, Eq, Hash)]
#[repr(u8)]
pub enum Type {
    Null = 0,
    Bool,
    Function,
    Int,
    Byte,
    I8,
    I16,
    I32,
    I64,
    UI,
    UI8,
    UI16,
    UI32,
    UI64,
    Float32,
    Float64,
    Complex64,
    Complex128,
    String,
    Rune,
    Array,
    Map,
    Iter,
    Struct,
    Ref,
    Closure,
    Interface,
    Type,
    Slice,
    Variadic,
    Alias,
    Channel,
    SmallInt,
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            Type::Null => "nil",
            Type::Bool => "bool",
            Type::Float32 => "float32",
            Type::Float64 => "float64",
            Type::Byte => "byte",
            Type::Int => "int",
            Type::UI => "uint",
            Type::I8 => "int8",
            Type::I16 => "int16",
            Type::I32 => "int32",
            Type::I64 => "int64",
            Type::UI8 => "uint8",
            Type::UI16 => "uint16",
            Type::UI32 => "uint32",
            Type::UI64 => "uint64",
            Type::String => "string",
            Type::Array => "array",
            Type::Function => "func",
            Type::Map => "map",
            Type::Iter => "iter",
            Type::Ref => "&",
            Type::Struct => "struct",
            Type::Rune => "rune",
            Type::Closure => "closure",
            Type::Complex64 => "complex64",
            Type::Complex128 => "complex128",
            Type::Interface => "interface",
            Type::Type => "type",
            Type::Slice => "slice",
            Type::Variadic => "variadic",
            Type::Alias => "alias",
            Type::Channel => "chan",
            Type::SmallInt => "int",
        };
        f.write_str(str)
    }
}

impl TryFrom<&str> for Type {
    type Error = std::string::String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Ok(match value {
            "nil" => Self::Null,
            "int" => Self::Int,
            "bool" => Self::Bool,
            "func" => Self::Function,
            "string" => Self::String,
            _ => return Err(value.to_string()),
        })
    }
}
