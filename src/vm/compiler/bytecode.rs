use crate::vm::object::collections::{Array, Map, Slice};
use crate::vm::object::float::{Float32, Float64};
use crate::vm::object::function::Closure;
use crate::vm::object::int::{Byte, Int, Int16, Int32, Int64, Int8, Uint, Uint16, Uint32, Uint64, Uint8};
use crate::vm::object::rune::Rune;
use crate::vm::object::structure::{Alias, Interface, Struct, TypeValue};
use crate::vm::object::{FromString, Object, Type};
use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use super::{Bytecode, SourceMap, Span};

const MAGIC: &[u8; 4] = b"GOVM";
const FORMAT_VERSION: u32 = 1;

pub fn serialize_bytecode(bytecode: &Bytecode) -> Vec<u8> {
    let mut buf = Vec::new();

    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());

    let instr_len = bytecode.instructions.len() as u32;
    buf.extend_from_slice(&instr_len.to_le_bytes());
    buf.extend_from_slice(&bytecode.instructions);

    let const_count = bytecode.constants.len() as u32;
    buf.extend_from_slice(&const_count.to_le_bytes());
    for obj in &bytecode.constants {
        serialize_object(&mut buf, obj);
    }

    serialize_source_map(&mut buf, &bytecode.source_map);

    buf
}

fn serialize_source_map(buf: &mut Vec<u8>, source_map: &SourceMap) {
    let entries = source_map.entries();
    buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (ip, span) in entries {
        buf.extend_from_slice(&(*ip as u32).to_le_bytes());
        write_string(buf, &span.file);
        buf.extend_from_slice(&(span.line as u32).to_le_bytes());
        buf.extend_from_slice(&(span.col as u32).to_le_bytes());
    }
}

fn deserialize_source_map(cursor: &mut Cursor<&[u8]>) -> Result<SourceMap, String> {
    let count = read_u32(cursor)? as usize;
    let mut source_map = SourceMap::new();
    for _ in 0..count {
        let ip = read_u32(cursor)? as usize;
        let file = read_string(cursor)?;
        let line = read_u32(cursor)? as usize;
        let col = read_u32(cursor)? as usize;
        source_map.add(ip, Span { file, line, col });
    }
    Ok(source_map)
}

pub fn deserialize_bytecode(data: &[u8]) -> Result<Bytecode, String> {
    let mut cursor = Cursor::new(data);

    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic).map_err(|e| format!("failed to read magic: {}", e))?;
    if &magic != MAGIC {
        return Err("invalid bytecode file: bad magic".to_string());
    }

    let version = read_u32(&mut cursor)?;
    if version != FORMAT_VERSION {
        return Err(format!("unsupported bytecode version: {}", version));
    }

    let instr_len = read_u32(&mut cursor)? as usize;
    let mut instructions = vec![0u8; instr_len];
    cursor.read_exact(&mut instructions).map_err(|e| format!("failed to read instructions: {}", e))?;

    let const_count = read_u32(&mut cursor)? as usize;
    let mut constants = Vec::with_capacity(const_count);
    for _ in 0..const_count {
        constants.push(deserialize_object(&mut cursor)?);
    }

    let source_map = deserialize_source_map(&mut cursor)?;

    Ok(Bytecode {
        constants,
        instructions,
        assert_stdout: None,
        source_map,
    })
}

fn serialize_object(buf: &mut Vec<u8>, obj: &Object) {
    let tag = obj.tag();
    buf.push(tag as u8);

    match tag {
        Type::Null => {}
        Type::Bool => {
            buf.push(if obj.as_bool() { 1 } else { 0 });
        }
        Type::Int => {
            let v = obj.as_int().value as i64;
            buf.extend_from_slice(&v.to_le_bytes());
        }
        Type::I8 => {
            buf.push(obj.as_int8().value as u8);
        }
        Type::I16 => {
            buf.extend_from_slice(&obj.as_int16().value.to_le_bytes());
        }
        Type::I32 => {
            buf.extend_from_slice(&obj.as_int32().value.to_le_bytes());
        }
        Type::I64 => {
            buf.extend_from_slice(&obj.as_int64().value.to_le_bytes());
        }
        Type::UI => {
            buf.extend_from_slice(&(obj.as_uint().value as u64).to_le_bytes());
        }
        Type::UI8 => {
            buf.push(obj.as_uint8().value);
        }
        Type::UI16 => {
            buf.extend_from_slice(&obj.as_uint16().value.to_le_bytes());
        }
        Type::UI32 => {
            buf.extend_from_slice(&obj.as_uint32().value.to_le_bytes());
        }
        Type::UI64 => {
            buf.extend_from_slice(&obj.as_uint64().value.to_le_bytes());
        }
        Type::Float32 => {
            buf.extend_from_slice(&obj.as_float32().to_le_bytes());
        }
        Type::Float64 => {
            buf.extend_from_slice(&obj.as_float64().to_le_bytes());
        }
        Type::Byte => {
            buf.push(obj.as_byte().value);
        }
        Type::String => {
            let s = obj.as_str();
            let bytes = s.as_bytes();
            buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(bytes);
        }
        Type::Rune => {
            let r = obj.as_rune().value as u32;
            buf.extend_from_slice(&r.to_le_bytes());
        }
        Type::Function => {
            let [ip, num_locals] = obj.as_function();
            buf.extend_from_slice(&ip.to_le_bytes());
            buf.extend_from_slice(&num_locals.to_le_bytes());
        }
        Type::Closure => {
            let cl = obj.as_closure();
            buf.extend_from_slice(&cl.ip.to_le_bytes());
            buf.extend_from_slice(&cl.num_locals.to_le_bytes());
            buf.push(if cl.is_null { 1 } else { 0 });
            buf.extend_from_slice(&(cl.captured.len() as u32).to_le_bytes());
            for cap in &cl.captured {
                serialize_object(buf, cap);
            }
        }
        Type::Struct => {
            let s = obj.as_struct();
            write_string(buf, &s.name);
            buf.push(if s.is_anonymous { 1 } else { 0 });
            buf.extend_from_slice(&(s.values.len() as u32).to_le_bytes());
            for v in &s.values {
                serialize_object(buf, v);
            }
            buf.extend_from_slice(&(s.method_dispatch.len() as u32).to_le_bytes());
            for (name, idx) in &s.method_dispatch {
                write_string(buf, name);
                buf.extend_from_slice(&(*idx as u32).to_le_bytes());
            }
            buf.extend_from_slice(&(s.tags.len() as u32).to_le_bytes());
            for tag in &s.tags {
                match tag {
                    Some(t) => {
                        buf.push(1);
                        write_string(buf, t);
                    }
                    None => {
                        buf.push(0);
                    }
                }
            }
        }
        Type::Interface => {
            let iface = unsafe { Interface::read(obj) };
            write_string(buf, &iface.name);
            buf.extend_from_slice(&(iface.methods.len() as u32).to_le_bytes());
            for m in &iface.methods {
                write_string(buf, m);
            }
            serialize_object(buf, &iface.value);
        }
        Type::Alias => {
            let alias = unsafe { Alias::read(obj) };
            write_string(buf, &alias.name);
            buf.push(if alias.is_transparent { 1 } else { 0 });
            serialize_object(buf, &alias.value);
            buf.extend_from_slice(&(alias.method_dispatch.len() as u32).to_le_bytes());
            for (name, idx) in &alias.method_dispatch {
                write_string(buf, name);
                buf.extend_from_slice(&(*idx as u32).to_le_bytes());
            }
        }
        Type::Type => {
            let tv = obj.as_type_value();
            buf.push(tv.value as u8);
            match &tv.inner_k {
                Some(k) => {
                    buf.push(1);
                    serialize_object(buf, k);
                }
                None => buf.push(0),
            }
            match &tv.inner_v {
                Some(v) => {
                    buf.push(1);
                    serialize_object(buf, v);
                }
                None => buf.push(0),
            }
        }
        Type::Array => {
            let arr = obj.as_vec();
            buf.extend_from_slice(&(arr.len() as u32).to_le_bytes());
            for el in arr {
                serialize_object(buf, el);
            }
        }
        Type::Slice => {
            let slice = obj.as_slice();
            buf.extend_from_slice(&(slice.len() as u32).to_le_bytes());
            for el in slice {
                serialize_object(buf, el);
            }
            let is_null = Slice::get_is_null(obj);
            buf.push(if is_null { 1 } else { 0 });
            let tv = Slice::get_type_value(obj);
            buf.push(tv.value as u8);
            match &tv.inner_k {
                Some(k) => {
                    buf.push(1);
                    serialize_object(buf, k);
                }
                None => buf.push(0),
            }
            match &tv.inner_v {
                Some(v) => {
                    buf.push(1);
                    serialize_object(buf, v);
                }
                None => buf.push(0),
            }
        }
        Type::Map => {
            let map = obj.as_map();
            buf.extend_from_slice(&(map.len() as u32).to_le_bytes());
            for (k, v) in map {
                serialize_object(buf, k);
                serialize_object(buf, v);
            }
        }
        Type::Ref => {
            let r = obj.as_ref();
            serialize_object(buf, &r.value);
        }
        Type::Variadic => {
            let v = unsafe { crate::vm::object::collections::Variadic::read(obj) };
            buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
            for el in v {
                serialize_object(buf, el);
            }
        }
        Type::Channel => {
            panic!("cannot serialize channel objects");
        }
        Type::Iter | Type::Complex64 | Type::Complex128 => {
            panic!("cannot serialize {:?} objects", tag);
        }
    }
}

fn deserialize_object(cursor: &mut Cursor<&[u8]>) -> Result<Object, String> {
    let tag_byte = read_u8(cursor)?;
    let tag: Type = unsafe { std::mem::transmute(tag_byte) };

    match tag {
        Type::Null => Ok(Object::null()),
        Type::Bool => {
            let v = read_u8(cursor)?;
            Ok(Object::bool(v != 0))
        }
        Type::Int => {
            let v = read_i64(cursor)?;
            Ok(Int::from_isize(v as isize))
        }
        Type::I8 => {
            let v = read_u8(cursor)? as i8;
            Ok(Int8::from_i8(v))
        }
        Type::I16 => {
            let v = read_i16(cursor)?;
            Ok(Int16::from_i16(v))
        }
        Type::I32 => {
            let v = read_i32(cursor)?;
            Ok(Int32::from_i32(v))
        }
        Type::I64 => {
            let v = read_i64(cursor)?;
            Ok(Int64::from_i64(v))
        }
        Type::UI => {
            let v = read_u64(cursor)?;
            Ok(Uint::from_usize(v as usize))
        }
        Type::UI8 => {
            let v = read_u8(cursor)?;
            Ok(Uint8::from_u8(v))
        }
        Type::UI16 => {
            let v = read_u16(cursor)?;
            Ok(Uint16::from_u16(v))
        }
        Type::UI32 => {
            let v = read_u32(cursor)?;
            Ok(Uint32::from_u32(v))
        }
        Type::UI64 => {
            let v = read_u64(cursor)?;
            Ok(Uint64::from_u64(v))
        }
        Type::Float32 => {
            let v = read_f32(cursor)?;
            Ok(Float32::from_f32(v))
        }
        Type::Float64 => {
            let v = read_f64(cursor)?;
            Ok(Float64::from_f64(v))
        }
        Type::Byte => {
            let v = read_u8(cursor)?;
            Ok(Byte::from_u8(v))
        }
        Type::String => {
            let s = read_string(cursor)?;
            Ok(Object::string(s))
        }
        Type::Rune => {
            let v = read_u32(cursor)?;
            let ch = char::from_u32(v).ok_or_else(|| format!("invalid rune value: {}", v))?;
            Ok(Rune::from_char(ch))
        }
        Type::Function => {
            let ip = read_u32(cursor)?;
            let num_locals = read_u32(cursor)? as u16;
            Ok(Object::function(ip, num_locals))
        }
        Type::Closure => {
            let ip = read_u32(cursor)?;
            let num_locals = read_u16(cursor)?;
            let is_null = read_u8(cursor)? != 0;
            let cap_count = read_u32(cursor)? as usize;
            let mut captured = Vec::with_capacity(cap_count);
            for _ in 0..cap_count {
                captured.push(deserialize_object(cursor)?);
            }
            let obj = Closure::object(ip, num_locals, captured);
            if is_null {
                unsafe { Closure::read_mut(&obj).is_null = true };
            }
            Ok(obj)
        }
        Type::Struct => {
            let name = read_string(cursor)?;
            let is_anonymous = read_u8(cursor)? != 0;
            let field_count = read_u32(cursor)? as usize;
            let mut values = Vec::with_capacity(field_count);
            for _ in 0..field_count {
                values.push(deserialize_object(cursor)?);
            }
            let method_count = read_u32(cursor)? as usize;
            let mut method_dispatch = Vec::with_capacity(method_count);
            for _ in 0..method_count {
                let mname = read_string(cursor)?;
                let idx = read_u32(cursor)? as usize;
                method_dispatch.push((mname, idx));
            }
            let tag_count = read_u32(cursor)? as usize;
            let mut tags = Vec::with_capacity(tag_count);
            for _ in 0..tag_count {
                let has_tag = read_u8(cursor)? != 0;
                if has_tag {
                    tags.push(Some(read_string(cursor)?));
                } else {
                    tags.push(None);
                }
            }
            Ok(Struct::object(name, values, method_dispatch, tags, is_anonymous))
        }
        Type::Interface => {
            let name = read_string(cursor)?;
            let method_count = read_u32(cursor)? as usize;
            let mut methods = Vec::with_capacity(method_count);
            for _ in 0..method_count {
                methods.push(read_string(cursor)?);
            }
            let value = deserialize_object(cursor)?;
            Ok(Interface::object(name, methods, value))
        }
        Type::Alias => {
            let name = read_string(cursor)?;
            let is_transparent = read_u8(cursor)? != 0;
            let value = deserialize_object(cursor)?;
            let method_count = read_u32(cursor)? as usize;
            let mut method_dispatch = Vec::with_capacity(method_count);
            for _ in 0..method_count {
                let mname = read_string(cursor)?;
                let idx = read_u32(cursor)? as usize;
                method_dispatch.push((mname, idx));
            }
            Ok(Alias::object(name, value, method_dispatch, is_transparent))
        }
        Type::Type => {
            let type_tag_byte = read_u8(cursor)?;
            let type_tag: Type = unsafe { std::mem::transmute(type_tag_byte) };
            let has_inner_k = read_u8(cursor)? != 0;
            let inner_k = if has_inner_k {
                Some(deserialize_object(cursor)?)
            } else {
                None
            };
            let has_inner_v = read_u8(cursor)? != 0;
            let inner_v = if has_inner_v {
                Some(deserialize_object(cursor)?)
            } else {
                None
            };
            if inner_v.is_some() {
                Ok(TypeValue::object_map(type_tag, inner_k, inner_v))
            } else {
                Ok(TypeValue::object(type_tag, inner_k))
            }
        }
        Type::Array => {
            let count = read_u32(cursor)? as usize;
            let mut elements = Vec::with_capacity(count);
            for _ in 0..count {
                elements.push(deserialize_object(cursor)?);
            }
            Ok(Array::from_vec(elements))
        }
        Type::Slice => {
            let count = read_u32(cursor)? as usize;
            let mut elements = Vec::with_capacity(count);
            for _ in 0..count {
                elements.push(deserialize_object(cursor)?);
            }
            let _is_null = read_u8(cursor)? != 0;
            let type_tag_byte = read_u8(cursor)?;
            let type_tag: Type = unsafe { std::mem::transmute(type_tag_byte) };
            let has_inner_k = read_u8(cursor)? != 0;
            let inner_k = if has_inner_k {
                Some(deserialize_object(cursor)?)
            } else {
                None
            };
            let has_inner_v = read_u8(cursor)? != 0;
            let inner_v = if has_inner_v {
                Some(deserialize_object(cursor)?)
            } else {
                None
            };
            let tv = TypeValue { value: type_tag, inner_k, inner_v };
            if _is_null {
                Ok(Slice::null(tv))
            } else {
                Ok(Slice::from_vec(elements, tv))
            }
        }
        Type::Map => {
            let count = read_u32(cursor)? as usize;
            let mut map = BTreeMap::new();
            for _ in 0..count {
                let k = deserialize_object(cursor)?;
                let v = deserialize_object(cursor)?;
                map.insert(k, v);
            }
            Ok(Map::from_map(map))
        }
        Type::Ref => {
            let value = deserialize_object(cursor)?;
            Ok(Object::ref_t(value))
        }
        Type::Variadic => {
            let count = read_u32(cursor)? as usize;
            let mut elements = Vec::with_capacity(count);
            for _ in 0..count {
                elements.push(deserialize_object(cursor)?);
            }
            Ok(crate::vm::object::collections::Variadic::from_vec(elements))
        }
        Type::Channel => {
            Err("cannot deserialize channel objects".to_string())
        }
        Type::Iter | Type::Complex64 | Type::Complex128 => {
            Err(format!("cannot deserialize {:?} objects", tag))
        }
    }
}

fn write_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(bytes);
}

fn read_u8(cursor: &mut Cursor<&[u8]>) -> Result<u8, String> {
    let mut buf = [0u8; 1];
    cursor.read_exact(&mut buf).map_err(|e| format!("read error: {}", e))?;
    Ok(buf[0])
}

fn read_u16(cursor: &mut Cursor<&[u8]>) -> Result<u16, String> {
    let mut buf = [0u8; 2];
    cursor.read_exact(&mut buf).map_err(|e| format!("read error: {}", e))?;
    Ok(u16::from_le_bytes(buf))
}

fn read_i16(cursor: &mut Cursor<&[u8]>) -> Result<i16, String> {
    let mut buf = [0u8; 2];
    cursor.read_exact(&mut buf).map_err(|e| format!("read error: {}", e))?;
    Ok(i16::from_le_bytes(buf))
}

fn read_u32(cursor: &mut Cursor<&[u8]>) -> Result<u32, String> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf).map_err(|e| format!("read error: {}", e))?;
    Ok(u32::from_le_bytes(buf))
}

fn read_i32(cursor: &mut Cursor<&[u8]>) -> Result<i32, String> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf).map_err(|e| format!("read error: {}", e))?;
    Ok(i32::from_le_bytes(buf))
}

fn read_u64(cursor: &mut Cursor<&[u8]>) -> Result<u64, String> {
    let mut buf = [0u8; 8];
    cursor.read_exact(&mut buf).map_err(|e| format!("read error: {}", e))?;
    Ok(u64::from_le_bytes(buf))
}

fn read_i64(cursor: &mut Cursor<&[u8]>) -> Result<i64, String> {
    let mut buf = [0u8; 8];
    cursor.read_exact(&mut buf).map_err(|e| format!("read error: {}", e))?;
    Ok(i64::from_le_bytes(buf))
}

fn read_f32(cursor: &mut Cursor<&[u8]>) -> Result<f32, String> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf).map_err(|e| format!("read error: {}", e))?;
    Ok(f32::from_le_bytes(buf))
}

fn read_f64(cursor: &mut Cursor<&[u8]>) -> Result<f64, String> {
    let mut buf = [0u8; 8];
    cursor.read_exact(&mut buf).map_err(|e| format!("read error: {}", e))?;
    Ok(f64::from_le_bytes(buf))
}

fn read_string(cursor: &mut Cursor<&[u8]>) -> Result<String, String> {
    let len = read_u32(cursor)? as usize;
    let mut buf = vec![0u8; len];
    cursor.read_exact(&mut buf).map_err(|e| format!("read error: {}", e))?;
    String::from_utf8(buf).map_err(|e| format!("invalid UTF-8: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_empty_bytecode() {
        let bc = Bytecode {
            constants: vec![],
            instructions: vec![],
            assert_stdout: None,
            source_map: SourceMap::new(),
        };
        let data = serialize_bytecode(&bc);
        let bc2 = deserialize_bytecode(&data).unwrap();
        assert_eq!(bc2.instructions.len(), 0);
        assert_eq!(bc2.constants.len(), 0);
    }

    #[test]
    fn test_roundtrip_primitives() {
        let mut sm = SourceMap::new();
        sm.add(0, Span { file: "test.go".to_string(), line: 1, col: 0 });
        sm.add(3, Span { file: "test.go".to_string(), line: 5, col: 10 });

        let bc = Bytecode {
            constants: vec![
                Object::null(),
                Object::bool(true),
                Object::bool(false),
                Object::int(42),
                Object::int(-100),
                Object::float64(3.14),
                Object::string("hello"),
                Object::byte(0xFF),
                Object::function(100, 5),
            ],
            instructions: vec![1, 2, 3, 4, 5],
            assert_stdout: None,
            source_map: sm,
        };

        let data = serialize_bytecode(&bc);
        let bc2 = deserialize_bytecode(&data).unwrap();

        assert_eq!(bc2.instructions, vec![1, 2, 3, 4, 5]);
        assert_eq!(bc2.constants.len(), 9);

        assert_eq!(bc2.constants[0].tag(), Type::Null);
        assert_eq!(bc2.constants[1].tag(), Type::Bool);
        assert_eq!(bc2.constants[1].as_bool(), true);
        assert_eq!(bc2.constants[2].as_bool(), false);
        assert_eq!(bc2.constants[3].as_isize(), 42);
        assert_eq!(bc2.constants[4].as_isize(), -100);
        assert_eq!(bc2.constants[5].as_float64(), 3.14);
        assert_eq!(bc2.constants[6].as_str(), "hello");
        assert_eq!(bc2.constants[7].as_byte().value, 0xFF);

        let [ip, locals] = bc2.constants[8].as_function();
        assert_eq!(ip, 100);
        assert_eq!(locals, 5);

        let entries = bc2.source_map.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, 0);
        assert_eq!(entries[0].1.file, "test.go");
        assert_eq!(entries[0].1.line, 1);
        assert_eq!(entries[1].0, 3);
        assert_eq!(entries[1].1.line, 5);
        assert_eq!(entries[1].1.col, 10);
    }

    #[test]
    fn test_roundtrip_struct() {
        let s = Struct::object(
            "Point".to_string(),
            vec![Object::int(10), Object::int(20)],
            vec![("area".to_string(), 42)],
            vec![Some("json:\"x\"".to_string()), None],
            false,
        );
        let bc = Bytecode {
            constants: vec![s],
            instructions: vec![],
            assert_stdout: None,
            source_map: SourceMap::new(),
        };

        let data = serialize_bytecode(&bc);
        let bc2 = deserialize_bytecode(&data).unwrap();

        let s2 = bc2.constants[0].as_struct();
        assert_eq!(s2.name, "Point");
        assert!(!s2.is_anonymous);
        assert_eq!(s2.values.len(), 2);
        assert_eq!(s2.values[0].as_isize(), 10);
        assert_eq!(s2.values[1].as_isize(), 20);
        assert_eq!(s2.method_dispatch.len(), 1);
        assert_eq!(s2.method_dispatch[0], ("area".to_string(), 42));
        assert_eq!(s2.tags.len(), 2);
        assert_eq!(s2.tags[0], Some("json:\"x\"".to_string()));
        assert_eq!(s2.tags[1], None);
    }
}
