use super::{Error, Object};
use crate::vm::gc::GC;
use crate::vm::object::int::{Byte, Int64};
use crate::vm::object::rune::Rune;
use crate::vm::object::Type;
use crate::vm::symbols::{ContextType, DefineType};

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
}

impl Builtin {
    pub fn is_void(&self) -> bool {
        match &self {
            Self::Print | Self::Println => true,
            _ => false,
        }
    }
}

impl From<u8> for Builtin {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Print,
            1 => Self::Type,
            2 => Self::Bool,
            3 => Self::Float,
            4 => Self::Int,
            5 => Self::String,
            6 => Self::Length,
            7 => Self::Byte,
            8 => Self::Rune,
            9 => Self::Println,
            _ => panic!("Builtin::from: invalid byte"),
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
        _ => None,
    }
}

pub fn signature_from_t(t: DefineType) -> Option<DefineType> {
    if !t.is_type() {
        return None;
    }

    let (_, inner_t) = t.as_type();

    //args define type need to be flexible
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

#[inline]
pub fn call(builtin: Builtin, args: &[Object], _gc: &mut GC) -> Result<Object, Error> {
    match builtin {
        Builtin::Print => call_print(args),
        //Builtin::Type => call_type(args, gc),
        //Builtin::String => call_string(args, gc),
        // Builtin::Bool => call_bool(args),
        // Builtin::Float => call_float(args, gc),
        // Builtin::Int => call_int(args),
        Builtin::Byte => call_byte(args),
        Builtin::Length => call_length(args),
        Builtin::Rune => call_rune(args),
        Builtin::Println => call_println(args),
        Builtin::Int64 => call_int64(args),
        _ => unimplemented!("{:#?}", builtin),
    }
}

fn call_int64(args: &[Object]) -> Result<Object, Error> {
    let num = args[0];

    let i = match num.tag() {
        Type::Int => num.as_isize() as i64,
        Type::I64 => return Ok(num),
        Type::UI => num.as_uint().value as i64,
        _ => unimplemented!(),
    };

    Ok(Int64::from_i64(i))
}

fn call_println(args: &[Object]) -> Result<Object, Error> {
    if !args.is_empty() {
        let mut args = args.iter();

        let mut output = Vec::with_capacity(args.len());
        for arg in args {
            output.push(format!("{:#?}({:#?})", arg.tag(), arg));
        }

        print!("{:#?}", output);
    }
    println!();
    Ok(Object::null())
}

/// Prints all the given arguments using a very simple format scheme
/// Example:
///     print("hello {}!", "world") => prints "hello world" to stdout
fn call_print(args: &[Object]) -> Result<Object, Error> {
    if !args.is_empty() {
        let mut args = args.iter();

        let mut output = Vec::with_capacity(args.len());
        for arg in args {
            output.push(format!("{:#?}", arg));
        }

        print!("{:#?}", output);
    }

    Ok(Object::null())
}

fn call_length(args: &[Object]) -> Result<Object, Error> {
    if args.len() != 1 {
        return Err(Error::ArgumentError(format!(
            "len expects 1 argument given {}",
            args.len()
        )));
    }

    let obj = match args[0].tag() {
        Type::Ref => args[0].as_ref().value,
        _ => args[0],
    };

    let length = match obj.tag() {
        Type::String => obj.as_str().chars().count() - 2,
        Type::Array => obj.as_vec().len(),
        Type::Map => obj.as_map().len(),
        _ => {
            return Err(Error::TypeError(format!(
                "type doesn't support len {}",
                obj.tag()
            )))
        }
    };
    Ok(Object::int(length as isize))
}

fn call_byte(args: &[Object]) -> Result<Object, Error> {
    if args.len() != 1 {
        return Err(Error::ArgumentError(format!(
            "byte expects 1 argument given {}",
            args.len()
        )));
    }

    let tag = args[0].tag();
    match tag {
        Type::Rune => {
            let r = args[0].as_rune().value;
            let byte = r as u8;
            Ok(Byte::from_u8(byte))
        }
        Type::Int => {
            let i = args[0].as_int().value;
            Ok(Byte::from_u8(i as u8))
        }
        Type::I8 => {
            let i = args[0].as_int8().value;
            Ok(Byte::from_u8(i as u8))
        }
        Type::I16 => {
            let i = args[0].as_int16().value;
            Ok(Byte::from_u8(i as u8))
        }
        Type::I32 => {
            let i = args[0].as_int32().value;
            Ok(Byte::from_u8(i as u8))
        }
        Type::I64 => {
            let i = args[0].as_int64().value;
            Ok(Byte::from_u8(i as u8))
        }
        Type::UI => {
            let i = args[0].as_uint().value;
            Ok(Byte::from_u8(i as u8))
        }
        Type::UI8 => {
            let i = args[0].as_uint8().value;
            Ok(Byte::from_u8(i))
        }
        Type::UI16 => {
            let i = args[0].as_uint16().value;
            Ok(Byte::from_u8(i as u8))
        }
        Type::UI32 => {
            let i = args[0].as_uint32().value;
            Ok(Byte::from_u8(i as u8))
        }
        Type::UI64 => {
            let i = args[0].as_uint64().value;
            Ok(Byte::from_u8(i as u8))
        }
        _ => Err(Error::ArgumentError(format!(
            "invalid argument: {:#?}",
            args[0]
        ))),
    }
}

fn call_rune(args: &[Object]) -> Result<Object, Error> {
    if args.len() != 1 {
        return Err(Error::ArgumentError(format!(
            "rune expects 1 argument given {}",
            args.len()
        )));
    }

    let tag = args[0].tag();
    match tag {
        Type::Rune => {
            let r = args[0].as_rune().value;
            Ok(Rune::from_char(r))
        }
        Type::Int => {
            let i = args[0].as_int().value;
            Ok(Rune::from_char(char::from_u32(i as u32).unwrap()))
        }
        Type::I8 => {
            let i = args[0].as_int8().value;
            Ok(Rune::from_char(char::from_u32(i as u32).unwrap()))
        }
        Type::I16 => {
            let i = args[0].as_int16().value;
            Ok(Rune::from_char(char::from_u32(i as u32).unwrap()))
        }
        Type::I32 => {
            let i = args[0].as_int32().value;
            Ok(Rune::from_char(char::from_u32(i as u32).unwrap()))
        }
        Type::I64 => {
            let i = args[0].as_int64().value;
            Ok(Rune::from_char(char::from_u32(i as u32).unwrap()))
        }
        Type::UI => {
            let i = args[0].as_uint().value;
            Ok(Rune::from_char(char::from_u32(i as u32).unwrap()))
        }
        Type::UI8 => {
            let i = args[0].as_uint8().value;
            Ok(Rune::from_char(char::from_u32(i as u32).unwrap()))
        }
        Type::UI16 => {
            let i = args[0].as_uint16().value;
            Ok(Rune::from_char(char::from_u32(i as u32).unwrap()))
        }
        Type::UI32 => {
            let i = args[0].as_uint32().value;
            Ok(Rune::from_char(char::from_u32(i as u32).unwrap()))
        }
        Type::UI64 => {
            let i = args[0].as_uint64().value;
            Ok(Rune::from_char(char::from_u32(i as u32).unwrap()))
        }
        _ => Err(Error::ArgumentError(format!(
            "invalid argument: {:#?}",
            args[0]
        ))),
    }
}
