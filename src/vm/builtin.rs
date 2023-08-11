use crate::vm::gc::GC;
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
        //Builtin::Length => call_length(args),
        _ => unimplemented!()
    }
}

/// Prints all the given arguments using a very simple format scheme
/// Example:
///     print("Hallo {}!", "wereld") => prints "Hallo wereld" to stdout
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

// Returns the given type of an object as a string object
// fn call_type(args: &[Object], gc: &mut Heap<Object>) -> Result<Object, Error> {
//     if args.len() != 1 {
//         return Err(Error::ArgumentError(format!(
//             "type() should have 1 argument given {}",
//             args.len()
//         )));
//     }
//
//     //add to gc
//     Ok(args[0].to_string())
// }

// Casts the given object to a string object
// fn call_string(args: &[Object], gc: &mut Heap<Object>) -> Result<Object, Error> {
//     if args.len() != 1 {
//         return Err(Error::ArgumentError(format!(
//             "expected 1 arg {}",
//             args.len()
//         )));
//     }
//
//     args[0].to_string_object()
// }

// Casts the given object to an object of type int
// fn call_int(args: &[Object]) -> Result<Object, Error> {
//     if args.len() != 1 {
//         return Err(Error::ArgumentError(format!(
//             "int() verwacht 1 argument, maar kreeg er {}",
//             args.len()
//         )));
//     }
//
//     let result = match args[0].tag() {
//         Type::Null => 0,
//         Type::Bool => {
//             if args[0].as_bool() {
//                 1
//             } else {
//                 0
//             }
//         }
//         Type::Float => unsafe { args[0].as_f64_unchecked() as isize },
//         Type::Int => return Ok(args[0]),
//         Type::String => unsafe {
//             match args[0].as_str_unchecked().trim().parse() {
//                 Ok(val) => val,
//                 Err(_) => {
//                     return Err(Error::ArgumentError(format!(
//                         "kan {:?} niet converteren naar een integer",
//                         args[0].as_str_unchecked()
//                     )))
//                 }
//             }
//         },
//         Type::Array | Type::Function => {
//             return Err(Error::ArgumentError(format!(
//                 "kan geen int maken van een {}",
//                 args[0].tag()
//             )))
//         }
//     };
//
//     Ok(Object::int(result))
// }

// Casts the given object to an object of type float
// fn call_float(args: &[Object], gc: &mut GC) -> Result<Object, Error> {
//     if args.len() != 1 {
//         return Err(Error::ArgumentError(format!(
//             "float() verwacht 1 argument, maar kreeg er {}",
//             args.len()
//         )));
//     }
//
//     let result = match args[0].tag() {
//         Type::Null => 0.0,
//         Type::Bool => {
//             if args[0].as_bool() {
//                 1.0
//             } else {
//                 0.0
//             }
//         }
//         Type::Float => return Ok(args[0]),
//         Type::Int => args[0].as_int() as f64,
//         Type::String => unsafe {
//             match args[0].as_str_unchecked().trim().parse() {
//                 Ok(val) => val,
//                 Err(_) => {
//                     return Err(Error::ArgumentError(format!(
//                         "kan {:?} niet converteren naar een float",
//                         args[0].as_str_unchecked()
//                     )))
//                 }
//             }
//         },
//         Type::Array | Type::Function => {
//             return Err(Error::ArgumentError(format!(
//                 "kan geen float maken van een {}",
//                 args[0].tag()
//             )))
//         }
//     };
//
//     Ok(Object::float(result, gc))
// }

// fn call_length(args: &[Object]) -> Result<Object, Error> {
//     if args.len() != 1 {
//         return Err(Error::ArgumentError(format!(
//             "expected 1 arg {}",
//             args.len()
//         )));
//     }
//
//     let length = match &args[0] {
//         Object::String(s) => s.as_str().chars().count(),
//         Object::List(l) => l.len(),
//         _ => {
//             return Err(Error::TypeError(format!(
//                 "does not support len:  {:#?}",
//                 args[0]
//             )))
//         }
//     };
//     Ok(Object::Int64(length as i64))
// }
