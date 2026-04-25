use std::collections::HashSet;

use crate::parser::ast;
use crate::vm::compiler::compiler::Compiler;
use wasm_encoder::Instruction;

lazy_static::lazy_static! {
    static ref ALLOWED_STDLIB: HashSet<&'static str> = {
        let mut s = HashSet::new();
        s.insert("errors");
        s.insert("math");
        s.insert("math/bits");
        s.insert("runtime");
        s.insert("strconv");
        s.insert("time");
        s.insert("unicode");
        s.insert("unicode/utf8");
        s
    };
}

include!("stdlib_generated.rs");

pub type NativeEmitter = fn(&mut Compiler);

pub fn get_native_func(name: &str) -> Option<NativeEmitter> {
    match name {
        "Float64bits" => Some(|c: &mut Compiler| {
            c.wasm.active().local_get(0);
            c.wasm.active().emit(&Instruction::I64ReinterpretF64);
        }),
        "Float64frombits" => Some(|c: &mut Compiler| {
            c.wasm.active().local_get(0);
            c.wasm.active().emit(&Instruction::F64ReinterpretI64);
        }),
        "Float32bits" => Some(|c: &mut Compiler| {
            c.wasm.active().local_get(0);
            c.wasm.active().emit(&Instruction::I32ReinterpretF32);
        }),
        "Float32frombits" => Some(|c: &mut Compiler| {
            c.wasm.active().local_get(0);
            c.wasm.active().emit(&Instruction::F32ReinterpretI32);
        }),
        _ if is_intrinsic(name) => Some(|c: &mut Compiler| {
            c.wasm.active().emit(&Instruction::Unreachable);
        }),
        _ => None,
    }
}

pub fn is_intrinsic(name: &str) -> bool {
    matches!(
        name,
        "__mem_load_i32"
            | "__mem_store_i32"
            | "__mem_load_i64"
            | "__mem_store_i64"
            | "__memory_size"
            | "__memory_grow"
            | "__global_get_i32"
            | "__global_set_i32"
    )
}

pub const STDLIB_PREFIX: &str = "$$stdlib/";

pub fn is_stdlib_import(path: &str) -> bool {
    ALLOWED_STDLIB.contains(path)
}

pub fn stdlib_synthetic_path(pkg: &str) -> String {
    format!("{}{}", STDLIB_PREFIX, pkg)
}

pub fn pkg_short_name(pkg: &str) -> &str {
    pkg.rsplit('/').next().unwrap_or(pkg)
}

pub fn parse_stdlib_package(pkg_name: &str) -> Result<Vec<ast::File>, String> {
    let sources = get_stdlib_sources(pkg_name)
        .ok_or_else(|| format!("unknown stdlib package: {}", pkg_name))?;

    let mut files = Vec::new();
    for source in sources {
        let file = crate::parser::parse_source(source)
            .map_err(|e| format!("failed to parse stdlib {}: {}", pkg_name, e))?;
        files.push(file);
    }
    Ok(files)
}

pub fn collect_stdlib_deps(pkg_name: &str) -> Result<Vec<String>, String> {
    let files = parse_stdlib_package(pkg_name)?;
    let mut deps = Vec::new();
    for file in &files {
        for imp in &file.imports {
            let path = imp.path.value.trim_matches('"');
            if is_stdlib_import(path) && !deps.contains(&path.to_string()) {
                deps.push(path.to_string());
            }
        }
    }
    Ok(deps)
}
