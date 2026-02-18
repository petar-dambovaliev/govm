use super::{Error, Object};
use crate::vm::object::channel::Channel;
use crate::vm::object::collections::{Map, Slice};
use crate::vm::object::float::{Float32, Float64};
use crate::vm::object::int::{Byte, Int, Int64};
use crate::vm::object::rune::Rune;
use crate::vm::object::structure::TypeValue;
use crate::vm::object::{FromString, Type};
use crate::vm::symbols::{ContextType, DefineType};
use crate::vm::VM;
use bdwgc_alloc::Allocator;
use std::collections::BTreeMap;
use std::io::{BufWriter, Write};

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
pub fn call(
    builtin: Builtin,
    args: &[Object],
    stdout: Option<&mut BufWriter<Vec<u8>>>,
) -> Result<Object, Error> {
    match builtin {
        Builtin::Print => call_print(args, stdout),
        Builtin::Type => call_type(args),
        Builtin::String => call_string(args),
        Builtin::Bool => call_bool(args),
        Builtin::Float => call_float(args),
        Builtin::Int => call_int(args),
        Builtin::Byte => call_byte(args),
        Builtin::Length => call_length(args),
        Builtin::Rune => call_rune(args),
        Builtin::Println => call_println(args, stdout),
        Builtin::Int64 => call_int64(args),
        Builtin::Make => call_make(args),
        Builtin::Cap => call_cap(args),
        Builtin::Append => call_append(args),
        Builtin::Copy => call_copy(args),
        Builtin::Delete => call_delete(args),
        Builtin::Clear => call_clear(args),
        Builtin::GcCollect => call_collect(args),
        Builtin::Sprintf => call_sprintf(args),
        Builtin::Panic => {
            let value = if args.is_empty() {
                Object::null()
            } else {
                args[0]
            };
            Err(Error::GoPanic(value))
        }
        Builtin::Recover => Ok(Object::null()),
        Builtin::Close => call_close(args),
    }
}

fn call_int(args: &[Object]) -> Result<Object, Error> {
    if args.len() != 1 {
        return Err(Error::ArgumentError(format!(
            "int expects 1 argument, given {}",
            args.len()
        )));
    }
    let num = args[0];
    let i = match num.tag() {
        Type::Int => return Ok(num),
        Type::I8 => num.as_int8().value as isize,
        Type::I16 => num.as_int16().value as isize,
        Type::I32 => num.as_int32().value as isize,
        Type::I64 => num.as_int64().value as isize,
        Type::UI => num.as_uint().value as isize,
        Type::UI8 => num.as_uint8().value as isize,
        Type::UI16 => num.as_uint16().value as isize,
        Type::UI32 => num.as_uint32().value as isize,
        Type::UI64 => num.as_uint64().value as isize,
        Type::Float32 => num.as_float32() as isize,
        Type::Float64 => num.as_float64() as isize,
        Type::Byte => num.as_byte().value as isize,
        Type::Rune => num.as_rune().value as isize,
        Type::Bool => {
            if num.as_bool() {
                1
            } else {
                0
            }
        }
        Type::String => {
            let s = num.as_str().trim_matches('"');
            match s.parse::<isize>() {
                Ok(v) => v,
                Err(_) => {
                    return Err(Error::TypeError(format!(
                        "cannot convert string {:?} to int",
                        s
                    )))
                }
            }
        }
        _ => {
            return Err(Error::TypeError(format!(
                "cannot convert {:?} to int",
                num.tag()
            )))
        }
    };
    Ok(Int::from_isize(i))
}

fn call_float(args: &[Object]) -> Result<Object, Error> {
    if args.len() != 1 {
        return Err(Error::ArgumentError(format!(
            "float expects 1 argument, given {}",
            args.len()
        )));
    }
    let num = args[0];
    let f = match num.tag() {
        Type::Float64 => return Ok(num),
        Type::Float32 => num.as_float32() as f64,
        Type::Int => num.as_int().value as f64,
        Type::I8 => num.as_int8().value as f64,
        Type::I16 => num.as_int16().value as f64,
        Type::I32 => num.as_int32().value as f64,
        Type::I64 => num.as_int64().value as f64,
        Type::UI => num.as_uint().value as f64,
        Type::UI8 => num.as_uint8().value as f64,
        Type::UI16 => num.as_uint16().value as f64,
        Type::UI32 => num.as_uint32().value as f64,
        Type::UI64 => num.as_uint64().value as f64,
        Type::Byte => num.as_byte().value as f64,
        Type::Rune => num.as_rune().value as u32 as f64,
        Type::String => {
            let s = num.as_str().trim_matches('"');
            match s.parse::<f64>() {
                Ok(v) => v,
                Err(_) => {
                    return Err(Error::TypeError(format!(
                        "cannot convert string {:?} to float",
                        s
                    )))
                }
            }
        }
        _ => {
            return Err(Error::TypeError(format!(
                "cannot convert {:?} to float",
                num.tag()
            )))
        }
    };
    Ok(Float64::from_f64(f))
}

fn call_string(args: &[Object]) -> Result<Object, Error> {
    if args.len() != 1 {
        return Err(Error::ArgumentError(format!(
            "string expects 1 argument, given {}",
            args.len()
        )));
    }
    let obj = args[0];
    let s = match obj.tag() {
        Type::String => return Ok(obj),
        Type::Byte => {
            let b = obj.as_byte().value;
            std::string::String::from(b as char)
        }
        Type::Rune => {
            let r = obj.as_rune().value;
            std::string::String::from(r)
        }
        Type::Slice => {
            let slice = obj.as_slice();
            if !slice.is_empty() && slice[0].tag() == Type::Byte {
                let bytes: Vec<u8> = slice.iter().map(|o| o.as_byte().value).collect();
                match std::string::String::from_utf8(bytes) {
                    Ok(s) => s,
                    Err(e) => {
                        return Err(Error::TypeError(format!(
                            "invalid UTF-8 in byte slice: {}",
                            e
                        )))
                    }
                }
            } else {
                format!("{}", obj)
            }
        }
        _ => format!("{}", obj),
    };
    Ok(Object::string(s))
}

fn call_bool(args: &[Object]) -> Result<Object, Error> {
    if args.len() != 1 {
        return Err(Error::ArgumentError(format!(
            "bool expects 1 argument, given {}",
            args.len()
        )));
    }
    let obj = args[0];
    let b = match obj.tag() {
        Type::Bool => return Ok(obj),
        Type::Int => obj.as_int().value != 0,
        Type::I8 => obj.as_int8().value != 0,
        Type::I16 => obj.as_int16().value != 0,
        Type::I32 => obj.as_int32().value != 0,
        Type::I64 => obj.as_int64().value != 0,
        Type::UI => obj.as_uint().value != 0,
        Type::UI8 => obj.as_uint8().value != 0,
        Type::UI16 => obj.as_uint16().value != 0,
        Type::UI32 => obj.as_uint32().value != 0,
        Type::UI64 => obj.as_uint64().value != 0,
        Type::Float32 => obj.as_float32() != 0.0,
        Type::Float64 => obj.as_float64() != 0.0,
        Type::Byte => obj.as_byte().value != 0,
        Type::Null => false,
        Type::String => {
            let s = obj.as_str();
            !s.is_empty() && s != "\"\""
        }
        _ => true,
    };
    Ok(Object::bool(b))
}

fn call_type(args: &[Object]) -> Result<Object, Error> {
    if args.len() != 1 {
        return Err(Error::ArgumentError(format!(
            "type expects 1 argument, given {}",
            args.len()
        )));
    }
    let obj = args[0];
    let t = match obj.tag() {
        Type::Ref => obj.as_ref().value.tag(),
        other => other,
    };
    Ok(TypeValue::object(t, None))
}

fn call_sprintf(args: &[Object]) -> Result<Object, Error> {
    let mut iter = args.iter();
    let mut s = String::with_capacity(args.len());

    while let Some(a) = iter.next() {
        s.push_str(&format!("{}", a));
    }

    Ok(Object::string(s))
}

fn call_collect(args: &[Object]) -> Result<Object, Error> {
    assert_eq!(args.len(), 0);

    println!("collecting");
    Allocator::force_collect();
    println!("collected");

    Ok(Object::null())
}

fn call_clear(args: &[Object]) -> Result<Object, Error> {
    assert_eq!(args.len(), 1);

    let mut iter = args.iter();
    let collection = iter.next().unwrap();

    match collection.tag() {
        Type::Map => {
            let m = unsafe { Map::read_mut(collection) };
            m.clear();
        }
        Type::Slice => {
            let s = collection.as_slice_mut();
            s.clear();
        }
        t => unimplemented!("builtin::clear: {:#?}", t),
    }

    Ok(Object::null())
}

fn call_delete(args: &[Object]) -> Result<Object, Error> {
    assert_eq!(args.len(), 2);

    let mut iter = args.iter();
    let collection = iter.next().unwrap();
    let m = unsafe { Map::read_mut(collection) };

    let k = iter.next().unwrap();
    m.remove(k);
    Ok(Object::null())
}

fn call_copy(args: &[Object]) -> Result<Object, Error> {
    assert_eq!(2, args.len());
    let mut dst = args[0];
    let src = args[1];

    if dst.tag() == Type::Slice && src.tag() == Type::String {
        let bytes = dst.as_slice_mut();
        let src_str = src.as_str().as_bytes();
        //this could be typed checked in the compiler
        // to avoid runtime overhead
        let mut i = 0;
        for b in src_str {
            if bytes.capacity() == bytes.len() - 1 {
                break;
            }
            if i >= bytes.len() {
                break;
            }
            bytes[i] = Object::byte(*b);
            i += 1;
        }
        return Ok(Object::null());
    }

    assert_eq!(dst.tag(), src.tag());

    match dst.tag() {
        Type::String => {
            let dst_str = dst.as_string_mut();
            let src_str = src.as_str();

            for ch in src_str.chars() {
                if dst_str.capacity() == dst_str.len() - 1 {
                    break;
                }
                dst_str.push(ch);
            }
        }
        Type::Slice => {
            let dst_slice = dst.as_slice_mut();
            let src_slice = src.as_slice();

            let mut i = 0;
            for el in src_slice {
                if dst_slice.capacity() == dst_slice.len() - 1 {
                    break;
                }
                if i >= dst_slice.len() {
                    break;
                }
                dst_slice[i] = *el;
                i += 1;
            }
        }
        _ => unimplemented!("copy: {:#?}", dst.tag()),
    }

    Ok(Object::null())
}

fn call_append(args: &[Object]) -> Result<Object, Error> {
    assert!(args.len() > 1);
    let mut iter = args.iter();
    let collection = iter.next().unwrap();

    match collection.tag() {
        Type::Slice => {
            while let Some(el) = iter.next() {
                collection.as_slice_mut().push(*el);
            }
        }
        k => unimplemented!("append: {:#?}", k),
    };

    Ok(*collection)
}

fn call_cap(args: &[Object]) -> Result<Object, Error> {
    assert_eq!(1, args.len());
    let obj = args[0];

    let i = match obj.tag() {
        Type::Slice => obj.as_slice().capacity(),
        Type::Array => obj.as_vec().capacity(),
        Type::Map => obj.as_map().len(),
        Type::Null => 0,
        _ => unimplemented!("cap: {:#?}", obj),
    };

    Ok(Int::from_isize(i as isize))
}

fn call_close(args: &[Object]) -> Result<Object, Error> {
    if args.len() != 1 {
        return Err(Error::ArgumentError(
            "close expects 1 argument".to_string(),
        ));
    }
    let ch_obj = args[0];
    if ch_obj.tag() != Type::Channel {
        return Err(Error::TypeError(format!(
            "close: expected channel, got {}",
            ch_obj.tag()
        )));
    }
    let ch = unsafe { Channel::read_mut(&ch_obj) };
    ch.closed = true;
    ch.sender.close();
    Ok(Object::null())
}

fn call_make(args: &[Object]) -> Result<Object, Error> {
    let mut arg_iter = args.iter();
    let t = arg_iter.next().unwrap();
    let len = arg_iter.next();
    let cap = arg_iter.next();

    assert_eq!(Type::Type, t.tag());

    let tv = unsafe { TypeValue::read(t) };

    let obj = match tv.value {
        Type::Channel => {
            let capacity = len.map(|l| l.as_isize() as usize).unwrap_or(0);
            Channel::new(capacity)
        }
        Type::Slice => {
            let p = tv.inner_k.unwrap();
            let inner = unsafe { TypeValue::read(&p) };

            let l = len.unwrap().as_isize();

            if l < 0 {
                panic!("len can't be negative");
            }

            let mut v = Vec::with_capacity(
                cap.map(|a| {
                    let i = a.as_isize();

                    if i < l {
                        panic!("len can't be greater than the cap");
                    }

                    i
                })
                .unwrap_or(l) as usize,
            );

            let def_value = match inner.value {
                Type::String => Object::string(""),
                Type::Int => Object::int(0),
                Type::Slice => {
                    call_make(&[TypeValue::object(Type::Slice, inner.inner_k), *len.unwrap()])?
                }
                _ => unimplemented!("{:#?}", inner.value),
            };

            for _ in 0..l {
                v.push(def_value);
            }

            Slice::from_vec(v, tv.clone())
        }
        Type::Map => {
            let k = tv.inner_k.unwrap();
            let v = tv.inner_v.unwrap();

            let _k = unsafe { TypeValue::read(&k) };
            let _v = unsafe { TypeValue::read(&v) };

            let m: BTreeMap<Object, Object> = BTreeMap::new();
            Map::from_map(m)
        }
        _ => panic!(),
    };

    Ok(obj)
}

fn call_int64(args: &[Object]) -> Result<Object, Error> {
    let num = args[0];

    let i = match num.tag() {
        Type::Int => num.as_isize() as i64,
        Type::I64 => return Ok(num),
        Type::UI => num.as_uint().value as i64,
        Type::I8 => num.as_int8().value as i64,
        Type::I16 => num.as_int16().value as i64,
        Type::I32 => num.as_int32().value as i64,
        Type::UI8 => num.as_uint8().value as i64,
        Type::UI16 => num.as_uint16().value as i64,
        Type::UI32 => num.as_uint32().value as i64,
        Type::UI64 => num.as_uint64().value as i64,
        Type::Float32 => num.as_float32() as i64,
        Type::Float64 => num.as_float64() as i64,
        Type::Byte => num.as_byte().value as i64,
        Type::Rune => num.as_rune().value as i64,
        _ => {
            return Err(Error::TypeError(format!(
                "cannot convert {:?} to int64",
                num.tag()
            )))
        }
    };

    Ok(Int64::from_i64(i))
}

fn call_println(
    args: &[Object],
    mut stdout: Option<&mut BufWriter<Vec<u8>>>,
) -> Result<Object, Error> {
    if !args.is_empty() {
        let args = args.iter();

        //let mut output = Vec::with_capacity(args.len());
        for arg in args {
            let s = format!("{:#?}", arg);
            if let Some(stdout) = &mut stdout {
                stdout.write_all(s.as_bytes()).unwrap();
            }
            print!("{} ", s);
        }
    }
    if let Some(stdout) = &mut stdout {
        stdout.write_all(&[b'\n']).unwrap();
    }
    println!();
    Ok(Object::null())
}

/// Prints all the given arguments using a very simple format scheme
/// Example:
///     print("hello {}!", "world") => prints "hello world" to stdout
fn call_print(
    args: &[Object],
    mut stdout: Option<&mut BufWriter<Vec<u8>>>,
) -> Result<Object, Error> {
    if !args.is_empty() {
        let args = args.iter();

        let mut output = Vec::with_capacity(args.len());
        for arg in args {
            let s = format!("{:#?}", arg);
            if let Some(stdout) = &mut stdout {
                stdout.write_all(s.as_bytes()).unwrap();
            }
            output.push(s);
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
        Type::Slice => obj.as_slice().len(),
        Type::Map => obj.as_map().len(),
        Type::Null => 0,
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
            Ok(Rune::from_char(char::from_u32(i).unwrap()))
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
