use crate::vm::gc::GC;
use crate::vm::object::Type;
use super::{Error, Object};

#[repr(u8)]
pub enum Builtin {
    Print,
    Type,
    Bool,
    Float,
    Int,
    String,
    Length,
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
            _ => panic!("Builtin::from: invalid byte")
        }
    }
}

pub(crate) fn resolve(name: &str) -> Option<Builtin> {
    match name {
        "print" => Some(Builtin::Print),
        "type" => Some(Builtin::Type),
        "int" => Some(Builtin::Int),
        "float" => Some(Builtin::Float),
        "bool" => Some(Builtin::Bool),
        "string" => Some(Builtin::String),
        "len" => Some(Builtin::Length),
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
        Builtin::Length => call_length(args),
        _ => unimplemented!()
    }
}

/// Prints all the given arguments using a very simple format scheme
/// Example:
///     print("hello {}!", "world") => prints "hello world" to stdout
fn call_print(args: &[Object]) -> Result<Object, Error> {
    if !args.is_empty() {
        let mut args = args.iter();
        let mut format_str = format!("{:#?}", args.next());

        for replacement in args {
            format_str = format_str.replacen("{}", &format!("{:#?}", replacement), 1);
        }

        print!("{format_str}");
    }

    println!();
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
        Type::Ref => {
            args[0].as_ref().value
        }
        _ => args[0]
    };

    let length = match obj.tag() {
        Type::String => obj.as_str().chars().count() - 2,
        Type::Array => obj.as_vec().len(),
        _ => {
            return Err(Error::TypeError(format!(
                "type doesn't support len {}",
                obj.tag()
            )))
        }
    };
    Ok(Object::int(length as isize))
}
