use crate::parser::ast::{
    AssignStmt, BasicLit, BranchStmt, Call, Decl, DeclStmt, Declaration,
    Element, Expression, Field, FieldList, ForStmt, FuncLit, Ident, IfStmt, IncDecStmt,
    InterfaceType, Operation, Package, RangeStmt, ReturnStmt, Statement, TypeSpec,
};
use crate::parser::parse_dir_recursive;
use crate::parser::token::{Keyword, LitKind, Operator};
use crate::vm::compiler::call::CallType;
use crate::vm::compiler::{make_ident_name, make_method_name, Context, FuncContext, LoopContext};
use std::path::PathBuf;

use crate::vm::compiler::declaration::{compile_const, compile_function, compile_variable};
use crate::vm::compiler::init_order::{compute_init_order, compute_package_order};
use crate::vm::module::ModuleResolver;
use crate::vm::symbols::{ContextType, DefineType, Qualifier, Resolved, SymbolTable};
use crate::vm::{builtin, Error};
use crate::wasm::WasmModuleBuilder;
use crate::wasm::layout::{
    struct_field_layout, elem_byte_size, array_byte_size,
    SLICE_HEADER_SIZE, SLICE_DATA_PTR_OFFSET, SLICE_LEN_OFFSET, SLICE_CAP_OFFSET,
};
use ahash::AHashMap;
use wasm_encoder::{BlockType, Instruction, ValType};

pub struct Compiler {
    pub(crate) symbols: SymbolTable,
    pub(crate) wasm: WasmModuleBuilder,
    pub(crate) contexts: Vec<Context>,
    pub(crate) func_contexts: Vec<FuncContext>,
    pub(crate) label_contexts: AHashMap<(usize, usize), String>,
    /// Symbol.index -> WASM local base index (strings occupy base and base+1)
    pub(crate) locals: AHashMap<u16, u32>,
    pub(crate) wasm_func_map: AHashMap<String, u32>,
    pub(crate) next_wasm_local: u32,
    str_concat_func_idx: Option<u32>,
    /// Maps a closure's WASM func_idx to the list of captured variable names and types.
    closure_captures: AHashMap<u32, Vec<(String, DefineType)>>,
    /// Maps a local variable's symbol index to a closure func_idx (when the variable holds a closure).
    closure_var_func: AHashMap<u16, u32>,
    next_closure_id: usize,
    anonymous_struct: usize,
    pub(crate) iota: usize,
    current_file: Option<String>,
    current_lines: Vec<usize>,
    nesting_depth: u32,
    /// WASM local holding the saved `$sp` for the current function.
    pub(crate) saved_sp_local: Option<u32>,
    /// Address-taken variables living in linear memory for the current function.
    pub(crate) mem_vars: AHashMap<String, MemVar>,
    /// WASM local holding the stack frame base pointer for the current function.
    pub(crate) frame_base_local: Option<u32>,
    /// Names of address-taken variables that escape and need heap allocation at declaration time.
    pub(crate) escaped_vars: std::collections::HashSet<String>,
}

#[derive(Clone)]
pub(crate) struct MemVar {
    pub addr_local: u32,
    pub size: u32,
}

const BUILTIN: &str = "0xbuiltin";

impl Compiler {
    pub fn new() -> Self {
        Self {
            symbols: SymbolTable::new(),
            wasm: WasmModuleBuilder::new(),
            contexts: Vec::new(),
            func_contexts: Vec::new(),
            label_contexts: AHashMap::new(),
            locals: AHashMap::new(),
            wasm_func_map: AHashMap::new(),
            next_wasm_local: 0,
            str_concat_func_idx: None,
            closure_captures: AHashMap::new(),
            closure_var_func: AHashMap::new(),
            next_closure_id: 0,
            anonymous_struct: 0,
            iota: 0,
            current_file: None,
            current_lines: Vec::new(),
            nesting_depth: 0,
            saved_sp_local: None,
            mem_vars: AHashMap::new(),
            frame_base_local: None,
            escaped_vars: std::collections::HashSet::new(),
        }
    }

    fn unsupported(&self, what: &str) -> Error {
        Error::SyntaxError(format!("WASM: unsupported on this branch: {}", what))
    }

    pub(crate) fn define_type_to_wasm(dt: &DefineType) -> ValType {
        match dt.unwrap_qualifiers() {
            DefineType::Int | DefineType::Int32 | DefineType::Uint | DefineType::Uint32
            | DefineType::Bool | DefineType::Byte | DefineType::Int8 | DefineType::Int16
            | DefineType::Uint8 | DefineType::Uint16 | DefineType::Rune => ValType::I32,
            DefineType::Int64 | DefineType::Uint64 => ValType::I64,
            DefineType::Float32 => ValType::F32,
            DefineType::Float64 => ValType::F64,
            _ => ValType::I32,
        }
    }

    fn unescape_go_string(s: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push(b'\n'),
                    Some('t') => out.push(b'\t'),
                    Some('r') => out.push(b'\r'),
                    Some('\\') => out.push(b'\\'),
                    Some('"') => out.push(b'"'),
                    Some('\'') => out.push(b'\''),
                    Some('0') => out.push(0),
                    Some(other) => {
                        out.push(b'\\');
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
                    }
                    None => out.push(b'\\'),
                }
            } else {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
        out
    }

    pub(crate) fn is_string_type(dt: &DefineType) -> bool {
        matches!(dt.unwrap_qualifiers(), DefineType::String)
    }

    fn emit_builtin_value(&mut self, name: &str, dt: &DefineType) {
        match dt.unwrap_to_base_type() {
            DefineType::Null => self.wasm.active().i32_const(0),
            DefineType::Bool => {
                let val = if name == "true" { 1 } else { 0 };
                self.wasm.active().i32_const(val);
            }
            _ => {}
        }
    }

    fn emit_nil_check(&mut self) {
        let tmp = self.next_wasm_local;
        self.wasm.active().local_tee(tmp);
        self.wasm.active().emit(&Instruction::I32Eqz);
        self.wasm.active().emit(&Instruction::If(BlockType::Empty));
        self.nesting_depth += 1;
        self.wasm.active().emit(&Instruction::Unreachable);
        self.nesting_depth -= 1;
        self.wasm.active().emit(&Instruction::End);
        self.wasm.active().local_get(tmp);
    }

    pub(crate) fn is_slice_type(dt: &DefineType) -> bool {
        matches!(dt.unwrap_qualifiers(), DefineType::Slice(_))
    }

    fn unwrap_slice_elem(dt: &DefineType) -> Option<DefineType> {
        match dt.unwrap_qualifiers() {
            DefineType::Slice(inner) => Some(inner.as_ref().clone()),
            _ => None,
        }
    }

    pub(crate) fn is_array_type(dt: &DefineType) -> bool {
        matches!(dt.unwrap_qualifiers(), DefineType::Array { .. })
    }

    fn unwrap_array_elem(dt: &DefineType) -> Option<(DefineType, usize)> {
        match dt.unwrap_qualifiers() {
            DefineType::Array { inner_type, len } => Some((inner_type.as_ref().clone(), len)),
            _ => None,
        }
    }

    fn is_numeric_type(dt: &DefineType) -> bool {
        matches!(
            dt.unwrap_qualifiers(),
            DefineType::Int
                | DefineType::Int8
                | DefineType::Int16
                | DefineType::Int32
                | DefineType::Int64
                | DefineType::Uint
                | DefineType::Uint8
                | DefineType::Uint16
                | DefineType::Uint32
                | DefineType::Uint64
                | DefineType::Byte
                | DefineType::Float32
                | DefineType::Float64
                | DefineType::Bool
                | DefineType::Rune
        )
    }

    /// Emit a store to `[base_ptr + offset]`. The **value** must already be on the stack.
    /// `base_local` is the WASM local holding the struct base pointer.
    fn emit_field_store(&mut self, base_local: u32, offset: u32, dt: &DefineType) {
        let base_dt = dt.unwrap_qualifiers();
        match base_dt {
            DefineType::String => {
                // Stack: [str_ptr, str_len]. Stash both, then store at base+offset and base+offset+4.
                let tmp_len = self.next_wasm_local;
                let tmp_ptr = self.next_wasm_local + 1;
                self.wasm.active().local_set(tmp_len);
                self.wasm.active().local_set(tmp_ptr);
                self.wasm.active().local_get(base_local);
                self.wasm.active().local_get(tmp_ptr);
                self.wasm.active().i32_store(offset as u64);
                self.wasm.active().local_get(base_local);
                self.wasm.active().local_get(tmp_len);
                self.wasm.active().i32_store((offset + 4) as u64);
            }
            DefineType::Int64 | DefineType::Uint64 => {
                let tmp = self.next_wasm_local;
                self.wasm.active().local_set(tmp);
                self.wasm.active().local_get(base_local);
                self.wasm.active().local_get(tmp);
                self.wasm.active().i64_store(offset as u64);
            }
            DefineType::Float64 => {
                let tmp = self.next_wasm_local;
                self.wasm.active().local_set(tmp);
                self.wasm.active().local_get(base_local);
                self.wasm.active().local_get(tmp);
                self.wasm.active().f64_store(offset as u64);
            }
            DefineType::Float32 => {
                let tmp = self.next_wasm_local;
                self.wasm.active().local_set(tmp);
                self.wasm.active().local_get(base_local);
                self.wasm.active().local_get(tmp);
                self.wasm.active().f32_store(offset as u64);
            }
            _ => {
                let tmp = self.next_wasm_local;
                self.wasm.active().local_set(tmp);
                self.wasm.active().local_get(base_local);
                self.wasm.active().local_get(tmp);
                self.wasm.active().i32_store(offset as u64);
            }
        }
    }

    fn emit_field_load(&mut self, offset: u64, dt: &DefineType) {
        let base_dt = dt.unwrap_qualifiers();
        match base_dt {
            DefineType::String => {
                // Stack has struct ptr. Dup it, load ptr at offset, then load len at offset+4.
                let tmp = self.next_wasm_local;
                self.wasm.active().local_set(tmp);
                self.wasm.active().local_get(tmp);
                self.wasm.active().i32_load(offset);
                self.wasm.active().local_get(tmp);
                self.wasm.active().i32_load(offset + 4);
            }
            DefineType::Int64 | DefineType::Uint64 => {
                self.wasm.active().i64_load(offset);
            }
            DefineType::Float64 => {
                self.wasm.active().f64_load(offset);
            }
            DefineType::Float32 => {
                self.wasm.active().f32_load(offset);
            }
            _ => {
                self.wasm.active().i32_load(offset);
            }
        }
    }

    /// Emit store of a value (already on stack) to `[addr_local + 0]` for a slice element.
    /// `addr_local` holds the computed target address.
    fn emit_elem_store(&mut self, addr_local: u32, elem_dt: &DefineType) {
        let base_dt = elem_dt.unwrap_qualifiers();
        match base_dt {
            DefineType::String => {
                let tmp_len = self.next_wasm_local;
                let tmp_ptr = self.next_wasm_local + 1;
                self.wasm.active().local_set(tmp_len);
                self.wasm.active().local_set(tmp_ptr);
                self.wasm.active().local_get(addr_local);
                self.wasm.active().local_get(tmp_ptr);
                self.wasm.active().i32_store(0);
                self.wasm.active().local_get(addr_local);
                self.wasm.active().local_get(tmp_len);
                self.wasm.active().i32_store(4);
            }
            DefineType::Int64 | DefineType::Uint64 => {
                let tmp = self.next_wasm_local;
                self.wasm.active().local_set(tmp);
                self.wasm.active().local_get(addr_local);
                self.wasm.active().local_get(tmp);
                self.wasm.active().i64_store(0);
            }
            DefineType::Float64 => {
                let tmp = self.next_wasm_local;
                self.wasm.active().local_set(tmp);
                self.wasm.active().local_get(addr_local);
                self.wasm.active().local_get(tmp);
                self.wasm.active().f64_store(0);
            }
            DefineType::Float32 => {
                let tmp = self.next_wasm_local;
                self.wasm.active().local_set(tmp);
                self.wasm.active().local_get(addr_local);
                self.wasm.active().local_get(tmp);
                self.wasm.active().f32_store(0);
            }
            _ => {
                let tmp = self.next_wasm_local;
                self.wasm.active().local_set(tmp);
                self.wasm.active().local_get(addr_local);
                self.wasm.active().local_get(tmp);
                self.wasm.active().i32_store(0);
            }
        }
    }

    /// Emit load of a slice element from address on the stack.
    fn emit_elem_load(&mut self, elem_dt: &DefineType) {
        let base_dt = elem_dt.unwrap_qualifiers();
        match base_dt {
            DefineType::String => {
                let tmp = self.next_wasm_local;
                self.wasm.active().local_set(tmp);
                self.wasm.active().local_get(tmp);
                self.wasm.active().i32_load(0);
                self.wasm.active().local_get(tmp);
                self.wasm.active().i32_load(4);
            }
            DefineType::Int64 | DefineType::Uint64 => {
                self.wasm.active().i64_load(0);
            }
            DefineType::Float64 => {
                self.wasm.active().f64_load(0);
            }
            DefineType::Float32 => {
                self.wasm.active().f32_load(0);
            }
            _ => {
                self.wasm.active().i32_load(0);
            }
        }
    }

    fn resolve_struct_fields(&mut self, pkg: &str, dt: &DefineType) -> Result<Vec<ContextType>, Error> {
        let base = dt.unwrap_qualifiers();
        match &base {
            DefineType::Struct { name, fields, .. } => {
                if fields.is_empty() {
                    if let Some(resolved) = self.symbols.resolve(pkg, name) {
                        let (_, f, _) = resolved.get_type().0.as_struct()?;
                        return Ok(f);
                    }
                }
                Ok(fields.clone())
            }
            _ => Err(Error::TypeError(format!("expected struct type, got {:?}", dt))),
        }
    }

    /// Compiles the given AST into WASM module bytes
    pub fn compile(
        &mut self,
        main: PathBuf,
        project_path: PathBuf,
        project: Vec<Package>,
        _output_assert: bool,
    ) -> Result<Vec<u8>, Error> {
        let pkg = project_path
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        self.register_builtin_types(&pkg);

        self.wasm.add_rt_alloc_import();
        self.wasm.add_print_string_import();
        self.wasm.add_println_string_import();
        self.wasm.add_default_memory();
        self.wasm.export_memory("memory", 0);
        self.wasm.add_stack_pointer_global();

        self.ensure_str_concat_func()?;

        let resolver = ModuleResolver::from_project_root(&project_path);

        let mut project = project;
        if let Some(ref resolver) = resolver {
            let mut remote_dirs: Vec<PathBuf> = Vec::new();
            for pkg in &project {
                for file in &pkg.files {
                    for import in &file.imports {
                        let import_path = import.path.value.trim_matches('"');
                        if !import_path.starts_with(resolver.module_path()) {
                            if let Ok(resolved) = resolver.resolve_import(import_path) {
                                if resolved.exists() && !remote_dirs.contains(&resolved) {
                                    remote_dirs.push(resolved);
                                }
                            }
                        }
                    }
                }
            }
            for dir in &remote_dirs {
                if let Ok(pkgs) = parse_dir_recursive(dir) {
                    for (_, pkg) in pkgs {
                        project.push(pkg);
                    }
                }
            }
        }

        let (pkg_order, pkgs_map) = compute_package_order(project, resolver.as_ref())
            .map_err(|e| Error::InternalError(e))?;

        for pkg_id in &pkg_order {
            let pkg = pkgs_map.get(pkg_id).unwrap();
            let cur_pkg = pkg
                .path
                .canonicalize()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();

            for file in &pkg.files {
                if let Some(ref path) = file.path {
                    self.current_file = Some(path.to_string_lossy().to_string());
                    self.current_lines = file.line_info.clone();
                }

                for import in &file.imports {
                    let import_path = import.path.value.trim_matches('"');

                    let p: PathBuf = if let Some(ref resolver) = resolver {
                        resolver.resolve_import(import_path).map_err(|e| {
                            Error::InternalError(format!(
                                "failed to resolve import '{}': {}",
                                import_path, e
                            ))
                        })?
                    } else {
                        project_path.join(import_path)
                    };

                    let alias = import.name.clone().map(|id| id.name.clone()).unwrap_or(
                        p.file_name()
                            .map(|f| f.to_str().unwrap())
                            .unwrap()
                            .to_string(),
                    );

                    self.symbols.define(
                        "",
                        &alias,
                        DefineType::Package {
                            path: p
                                .canonicalize()
                                .expect(&format!("cannot canonicalize: {:#?}", p))
                                .to_str()
                                .expect(&format!("cannot to_str: {:#?}", p))
                                .to_string(),
                            alias: alias.clone(),
                        },
                        false,
                    );
                }
            }

            let mut all_decls = Vec::new();
            for file in &pkg.files {
                all_decls.extend(file.decl.iter().cloned());
            }

            self.pre_register_declarations(&cur_pkg, &all_decls);

            let ordered =
                compute_init_order(&all_decls).map_err(|e| Error::InternalError(e))?;

            for decl in &ordered {
                self.compile_declaration(&cur_pkg, decl)?;
            }
        }

        let main_pkg = main
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let main_name = make_ident_name(&main_pkg, "main");
        if let Some(&func_idx) = self.wasm_func_map.get(&main_name) {
            self.wasm.export_func("main", func_idx);
        } else if let Some(&func_idx) = self.wasm_func_map.get("main") {
            self.wasm.export_func("main", func_idx);
        } else {
            return Err(Error::ReferenceError("main function not found".to_string()));
        }

        let wasm = std::mem::replace(&mut self.wasm, WasmModuleBuilder::new());
        Ok(wasm.finish())
    }

    fn register_builtin_types(&mut self, pkg: &str) {
        self.compile_declaration(
            pkg,
            &Declaration::Type(Decl {
                docs: vec![],
                pos0: 0,
                pos1: None,
                specs: vec![TypeSpec {
                    docs: vec![],
                    alias: false,
                    name: Default::default(),
                    params: Default::default(),
                    typ: Expression::TypeInterface(InterfaceType {
                        pos: 0,
                        methods: Default::default(),
                    }),
                }],
            }),
        )
        .ok();

        let type_defs = vec![
            ("string", DefineType::String),
            ("bool", DefineType::Bool),
            ("int", DefineType::Int),
            ("int8", DefineType::Int8),
            ("int16", DefineType::Int16),
            ("int32", DefineType::Int32),
            ("int64", DefineType::Int64),
            ("uint", DefineType::Uint),
            ("uint8", DefineType::Uint8),
            ("uint16", DefineType::Uint16),
            ("uint32", DefineType::Uint32),
            ("uint64", DefineType::Uint64),
            ("byte", DefineType::Byte),
            ("float32", DefineType::Float32),
            ("float64", DefineType::Float64),
            ("rune", DefineType::Rune),
        ];

        for (name, dt) in type_defs {
            let t = match &dt {
                DefineType::String => crate::vm::types::Type::String,
                DefineType::Bool => crate::vm::types::Type::Bool,
                DefineType::Int => crate::vm::types::Type::Int,
                DefineType::Int8 => crate::vm::types::Type::I8,
                DefineType::Int16 => crate::vm::types::Type::I16,
                DefineType::Int32 => crate::vm::types::Type::I32,
                DefineType::Int64 => crate::vm::types::Type::I64,
                DefineType::Uint => crate::vm::types::Type::UI,
                DefineType::Uint8 => crate::vm::types::Type::UI8,
                DefineType::Uint16 => crate::vm::types::Type::UI16,
                DefineType::Uint32 => crate::vm::types::Type::UI32,
                DefineType::Uint64 => crate::vm::types::Type::UI64,
                DefineType::Byte => crate::vm::types::Type::Byte,
                DefineType::Float32 => crate::vm::types::Type::Float32,
                DefineType::Float64 => crate::vm::types::Type::Float64,
                DefineType::Rune => crate::vm::types::Type::Rune,
                _ => unreachable!(),
            };
            self.symbols.define(
                BUILTIN,
                name,
                DefineType::Type(Box::new(dt), t),
                false,
            );
        }

        self.symbols.define(
            BUILTIN,
            "nil",
            DefineType::Type(Box::new(DefineType::Null), crate::vm::types::Type::Null),
            false,
        );
        self.symbols.define(BUILTIN, "true", DefineType::Bool, false);
        self.symbols.define(BUILTIN, "false", DefineType::Bool, false);
        self.symbols.define(
            BUILTIN,
            "_",
            DefineType::Qualified(Qualifier::Var, Box::new(DefineType::Null)),
            false,
        );
    }

    fn define_type_to_context_type(&self, dts: &[DefineType]) -> Vec<ContextType> {
        let mut decl_arg_types = Vec::with_capacity(dts.len());
        for dt in dts {
            decl_arg_types.push(ContextType::Unnamed(dt.clone()));
        }
        decl_arg_types
    }

    fn field_list_to_define_type(
        &mut self,
        pkg: &str,
        fl: &FieldList,
    ) -> (DefineType, Vec<DefineType>) {
        let mut decl_r_types = Vec::with_capacity(fl.list.len());

        fn field_to_define_type(c: &mut Compiler, pkg: &str, field: &Field) -> DefineType {
            match &field.typ {
                Expression::Ident(id) => c
                    .symbols
                        .resolve(pkg, id.name.as_str())
                        .unwrap()
                        .get_type()
                    .0,
                Expression::TypePointer(pt) => {
                    let id = pt.typ.as_ident().unwrap();
                    let t = c.symbols.resolve(pkg, id.name.as_str()).unwrap().get_type();
                    DefineType::Ref(Box::new(t.0))
                }
                Expression::TypeFunction(f) => {
                    let (_, t_vec) = c.field_list_to_define_type(pkg, &f.params);
                    let (dt, _) = c.field_list_to_define_type(pkg, &f.result);
                    DefineType::Func {
                        name: "".to_string(),
                        recv: None,
                        args: c.define_type_to_context_type(t_vec.as_ref()),
                        rt: Box::new(dt),
                    }
                }
                Expression::TypeStruct(st) => {
                    let fields = st
                        .fields
                        .iter()
                        .map(|f| {
                            ContextType::Named(
                                f.name.first().unwrap().name.clone(),
                                field_to_define_type(c, pkg, f),
                            )
                        })
                        .collect();
                    DefineType::Struct {
                        name: format!("anonymous_struct {}", c.anonymous_struct),
                        fields,
                        methods: vec![],
                    }
                }
                _ => panic!("function: unsupported parameter expression: {:#?}", field),
            }
        }

        for el in &fl.list {
            decl_r_types.push(field_to_define_type(self, pkg, el));
        }

        let r_t = if decl_r_types.is_empty() {
            DefineType::Null
        } else if decl_r_types.len() == 1 {
            decl_r_types[0].clone()
        } else {
            DefineType::Tuple(decl_r_types.clone())
        };

        (r_t, decl_r_types)
    }

    pub(crate) fn expression_to_define_type(
        &mut self,
        pkg: &str,
        expr: &Expression,
    ) -> Option<DefineType> {
        match expr {
            Expression::Ident(id) => {
                Some(self.symbols.resolve(pkg, id.name.as_str())?.get_type().0)
            }
            Expression::TypeFunction(tf) => {
                let (_, args) = self.field_list_to_define_type(pkg, &tf.params);
                let (ret, _) = self.field_list_to_define_type(pkg, &tf.result);
                Some(DefineType::Func {
                    name: "".to_string(),
                    recv: None,
                    args: self.define_type_to_context_type(args.as_ref()),
                    rt: Box::new(ret),
                })
            }
            Expression::TypePointer(tp) => Some(DefineType::Ref(Box::new(
                self.expression_to_define_type(pkg, &tp.typ)?,
            ))),
            Expression::TypeMap(map) => {
                let k = self.expression_to_define_type(pkg, map.key.as_ref())?;
                let v = self.expression_to_define_type(pkg, map.val.as_ref())?;
                Some(DefineType::Map(Box::new(k), Box::new(v)))
            }
            Expression::Invar(invar) => Some(DefineType::Qualified(
                Qualifier::Invar,
                Box::new(self.expression_to_define_type(pkg, invar.expr.as_ref())?),
            )),
            Expression::TypeInterface(i) => {
                assert!(i.methods.list.is_empty());
                Some(DefineType::Interface {
                    name: "".to_string(),
                    methods: vec![],
                })
            }
            Expression::TypeArray(ta) => {
                let inner = self.expression_to_define_type(pkg, &ta.typ)?;
                let len = ta.len.as_int_lit().unwrap();
                Some(DefineType::Array {
                    inner_type: Box::new(inner),
                    len: len as usize,
                })
            }
            Expression::TypeSlice(ts) => {
                let inner = self.expression_to_define_type(pkg, &ts.typ)?;
                Some(DefineType::Slice(Box::new(inner)))
            }
            Expression::Ellipsis(variadic) => {
                let inner =
                    self.expression_to_define_type(pkg, variadic.elt.as_ref().unwrap().as_ref())?;
                Some(DefineType::Variadic(Box::new(inner)))
            }
            Expression::TypeStruct(st) => {
                let mut fields = Vec::with_capacity(st.fields.len());
                for field in &st.fields {
                    fields.push(ContextType::Named(
                        field.name.first().unwrap().name.clone(),
                        self.expression_to_define_type(pkg, &field.typ)?,
                    ));
                }
                Some(DefineType::Struct {
                    name: format!("anonymous_struct {}", self.anonymous_struct),
                    fields,
                    methods: vec![],
                })
            }
            Expression::TypeChannel(ch) => {
                let inner = self.expression_to_define_type(pkg, &ch.typ)?;
                Some(DefineType::Channel(Box::new(inner)))
            }
            _ => panic!("expression_to_define_type: unsupported expr {:#?}", expr),
        }
    }

    pub(crate) fn pre_register_declarations(&mut self, pkg: &str, decls: &[Declaration]) {
        for decl in decls {
            if let Declaration::Type(tspec) = decl {
                for spec in &tspec.specs {
                    match &spec.typ {
                        Expression::Ident(_) => {
                            let dt = DefineType::Spec {
                                name: spec.name.name.clone(),
                                inner: Box::from(DefineType::Null),
                                methods: vec![],
                                is_transparent: spec.alias,
                            };
                            let _ = self.symbols.define(pkg, &spec.name.name, dt, false);
                        }
                        Expression::TypeInterface(_) => {
                            let dt = DefineType::Interface {
                                name: spec.name.name.clone(),
                                methods: vec![],
                            };
                            let _ = self.symbols.define(pkg, &spec.name.name, dt, false);
                        }
                        Expression::TypeStruct(_) => {
                            let dt = DefineType::Struct {
                                name: spec.name.name.clone(),
                                fields: vec![],
                                methods: vec![],
                            };
                            let _ = self.symbols.define(pkg, &spec.name.name, dt, false);
                        }
                        _ => {}
                    }
                }
            }
        }

        for decl in decls {
            match decl {
                Declaration::Const(v) => {
                    for spec in &v.specs {
                        for name in &spec.name {
                            let dt =
                                DefineType::Qualified(Qualifier::Const, Box::new(DefineType::Null));
                            let _ = self.symbols.define(pkg, &name.name, dt, false);
                        }
                    }
                }
                Declaration::Variable(v) => {
                    for spec in &v.specs {
                        for name in &spec.name {
                            let dt =
                                DefineType::Qualified(Qualifier::Var, Box::new(DefineType::Null));
                            let _ = self.symbols.define(pkg, &name.name, dt, false);
                        }
                    }
                }
                Declaration::Function(f) => {
                    let (f_name, _recv, recv_t) = if let Some(recv) = f.recv.as_ref() {
                        let recv_field = recv.list.first().unwrap();
                        let t = self.expression_to_define_type(pkg, &recv_field.typ).unwrap();
                        (
                            make_method_name(pkg, t.strip_ref(), &f.name.name),
                            Some(recv_field),
                            Some(Box::new(t)),
                        )
                    } else {
                        (f.name.name.clone(), None, None)
                    };

                    let mut decl_arg_types = Vec::with_capacity(f.typ.params.list.len());
                    for p in &f.typ.params.list {
                        let t = self
                            .expression_to_define_type(pkg, &p.typ)
                            .expect(&format!("{:#?}-{:#?}", pkg, p.typ));
                        for name in &p.name {
                            decl_arg_types
                                .push(ContextType::Named(name.name.clone(), t.clone()));
                        }
                    }

                    let mut decl_r_types = Vec::with_capacity(f.typ.result.list.len());
                    for el in &f.typ.result.list {
                        let t = self.expression_to_define_type(pkg, &el.typ).unwrap();
                        decl_r_types.push(t);
                    }

                    let r_t = if decl_r_types.is_empty() {
                        DefineType::Null
                    } else if decl_r_types.len() == 1 {
                        decl_r_types[0].clone()
                    } else {
                        DefineType::Tuple(decl_r_types.clone())
                    };

                    let func_def = DefineType::Func {
                        name: f.name.name.clone(),
                        recv: recv_t.clone(),
                        args: decl_arg_types,
                        rt: Box::new(r_t),
                    };

                    let _ = self.symbols.define(pkg, &f_name, func_def.clone(), false);

                    if let Some(recv) = recv_t {
                        let tt = self
                            .symbols
                            .resolve(pkg, &recv.get_type_name())
                            .unwrap()
                            .get_type()
                            .0;

                        if tt.is_struct() {
                            let (r_name, r_fields, mut r_methods) = tt.as_struct().unwrap();
                            r_methods.push(func_def.clone());
                            let updated = self.symbols.update_dt(
                                pkg,
                                &r_name,
                                DefineType::Struct {
                                    name: r_name.to_string(),
                                    fields: r_fields.clone(),
                                    methods: r_methods.clone(),
                                },
                            );
                            assert!(updated);
                        } else if tt.is_spec() {
                            let (name, inner, mut methods, is_transparent) =
                                tt.as_spec().unwrap();
                            methods.push(func_def.clone());
                            let updated = self.symbols.update_dt(
                                pkg,
                                &name,
                                DefineType::Spec {
                                    name: name.to_string(),
                                    inner: Box::new(inner.clone()),
                                    methods: methods.clone(),
                                    is_transparent,
                                },
                            );
                            assert!(updated);
                        } else {
                            unimplemented!("{:#?}", tt);
                        }
                    }
                }
                Declaration::Type(_) => {}
            }
        }
    }

    pub(crate) fn compile_declaration(
        &mut self,
        pkg: &str,
        decl: &Declaration,
    ) -> Result<(), Error> {
        match decl {
            Declaration::Variable(_v) => {
                compile_variable(pkg, _v, self)?;
            }
            Declaration::Function(f) => {
                compile_function(pkg, f, self)?;
            }
            Declaration::Const(c) => {
                compile_const(pkg, c, self)?;
            }
            Declaration::Type(t) => {
                for spec in &t.specs {
                    match &spec.typ {
                        Expression::TypeInterface(it) => {
                            self.compile_interface_type(pkg, spec, it)?;
                        }
                        Expression::TypeStruct(ta) => {
                            self.compile_struct_type(pkg, spec, ta)?;
                        }
                        Expression::Ident(_id) => {
                            self.compile_type_spec(pkg, spec)?;
                        }
                        _ => return Err(self.unsupported(&format!("type spec: {:#?}", spec))),
                    }
                }
            }
        }
        Ok(())
    }

    fn compile_interface_type(
        &mut self,
        pkg: &str,
        spec: &TypeSpec,
        it: &InterfaceType,
    ) -> Result<(), Error> {
        let mut funcs = Vec::with_capacity(it.methods.list.len());
        for field in &it.methods.list {
            let func_name = field
                .name
                .first()
                .ok_or_else(|| Error::SyntaxError("interface method has no name".to_string()))?;
            let (_, _, args, rt) = self
                .expression_to_define_type(pkg, &field.typ)
                .ok_or_else(|| {
                    Error::TypeError(format!(
                        "failed to resolve interface method type: {}",
                        func_name.name
                    ))
                })?
                .as_func();

            funcs.push(DefineType::Func {
                name: func_name.name.to_string(),
                recv: None,
                args,
                rt,
            });
        }

        match self.symbols.resolve(pkg, &spec.name.name) {
            Some(_) => {
                self.symbols.update_dt(
                    pkg,
                    &spec.name.name,
                    DefineType::Interface {
                        name: spec.name.name.clone(),
                        methods: funcs,
                    },
                );
            }
            None => {
                self.symbols.define(
                    pkg,
                    &spec.name.name,
                    DefineType::Interface {
                        name: spec.name.name.clone(),
                        methods: funcs,
                    },
                    false,
                );
            }
        };
        Ok(())
    }

    fn compile_struct_type(
        &mut self,
        pkg: &str,
        spec: &TypeSpec,
        ta: &crate::parser::ast::StructType,
    ) -> Result<(), Error> {
        let t = spec.name.clone();
        let mut field_types = vec![];

        for field in &ta.fields {
            let (inner_t, is_ref) = match &field.typ {
                Expression::TypePointer(p) => {
                    (p.typ.as_ident().map_err(|e| Error::TypeError(e))?, true)
                }
                _ => (
                    field.typ.as_ident().map_err(|e| Error::TypeError(e))?,
                    false,
                ),
            };

            if !is_ref && t.name == inner_t.name {
                return Err(Error::TypeError(format!(
                    "recursive type definition: {}",
                    t.name
                )));
            }

            let r = self
                .symbols
                .resolve(pkg, &inner_t.name)
                .ok_or_else(|| {
                    Error::ReferenceError(format!("undefined type: {}", inner_t.name))
                })?
                .get_type()
                .0;

            let dt = if is_ref {
                DefineType::Ref(Box::new(r.strip_type()))
            } else {
                r.strip_type()
            };

            if field.name.is_empty() {
                field_types.push(ContextType::Embedded(inner_t.name.clone(), dt.clone()));
            } else {
                for name in &field.name {
                    field_types.push(ContextType::Named(name.name.as_str().to_string(), dt.clone()));
                }
            }
        }

        let name = spec.name.name.as_str();

        match self.symbols.resolve(pkg, &spec.name.name) {
            Some(_) => {}
            None => {
                self.symbols.define(
                    pkg,
                    name,
                    DefineType::Struct {
                        name: name.to_string(),
                        fields: field_types.clone(),
                        methods: vec![],
                    },
                    false,
                );
            }
        };

        self.symbols
            .update_struct_fields(pkg, name, field_types.clone());

        Ok(())
    }

    fn compile_type_spec(&mut self, pkg: &str, spec: &TypeSpec) -> Result<(), Error> {
        let t = spec.name.clone();
        let inner_t = self
            .expression_to_define_type(pkg, &spec.typ)
            .ok_or_else(|| {
                Error::TypeError(format!("failed to resolve type spec: {}", t.name))
            })?;

        match self.symbols.resolve(pkg, &t.name) {
            Some(s) => {
                let s = s.get_symbol();
                self.symbols.update_dt(
                    pkg,
                    &t.name,
                    DefineType::Spec {
                        name: t.name.to_string(),
                        inner: Box::new(inner_t),
                        methods: vec![],
                        is_transparent: spec.alias,
                    },
                );
                let _ = s;
            }
            None => {
                self.symbols.define(
                    pkg,
                    &t.name,
                    DefineType::Spec {
                        name: t.name.to_string(),
                        inner: Box::new(inner_t),
                        methods: vec![],
                        is_transparent: spec.alias,
                    },
                    false,
                );
            }
        };
        Ok(())
    }

    pub(crate) fn compile_block_statement(
        &mut self,
        pkg: &str,
        block: &[Statement],
    ) -> Result<Option<bool>, Error> {
        if block.is_empty() {
            return Ok(Some(false));
        }

        self.symbols.enter_scope();
        let mut terminates = true;
        let mut last_term = false;

        for s in block {
            let term = self.compile_statement(pkg, s)?;

            let is_empty = matches!(s, Statement::Empty(_));

            if last_term
                && !is_empty
                && self
                    .func_contexts
                    .last()
                    .map_or(false, |fc| fc.expected_ret.is_some())
            {
                panic!("deadcode: {:#?}", s);
            }

            if let Some(te) = term {
                if !te {
                    terminates = false;
                } else {
                    last_term = true;
                }
            }
        }

        self.symbols.leave_scope();
        Ok(Some(terminates))
    }

    pub(crate) fn compile_statement(
        &mut self,
        pkg: &str,
        stmt: &Statement,
    ) -> Result<Option<bool>, Error> {
        match stmt {
            Statement::For(forstmt) => self.compile_for_statement(pkg, forstmt),
            Statement::If(ifstmt) => self.compile_if_statement(pkg, ifstmt),
            Statement::Assign(assign) => self.compile_assign_statement(pkg, assign),
            Statement::Expr(expr) => {
                let dt = self.compile_expression(pkg, &expr.expr)?;
                if dt != DefineType::Null && !dt.is_func() {
                    if Self::is_string_type(&dt) {
                        self.wasm.active().drop();
                        self.wasm.active().drop();
                    } else {
                        self.wasm.active().drop();
                    }
                }
                Ok(None)
            }
            Statement::Block(stmts) => self.compile_block_statement(pkg, &stmts.list),
            Statement::Declaration(declr) => {
                match declr {
                    DeclStmt::Type(t) => {
                        self.compile_declaration(pkg, &Declaration::Type(t.clone()))?;
                    }
                    DeclStmt::Const(t) => {
                        self.compile_declaration(pkg, &Declaration::Const(t.clone()))?;
                    }
                    DeclStmt::Variable(t) => {
                        self.compile_declaration(pkg, &Declaration::Variable(t.clone()))?;
                    }
                }
                Ok(None)
            }
            Statement::Return(expr) => self.compile_return_statement(pkg, expr),
            Statement::Branch(branch) => self.compile_branch_statement(pkg, branch),
            Statement::IncDec(incdec) => self.compile_incdec_statement(pkg, incdec),
            Statement::Empty(_) => Ok(None),
            Statement::Range(range) => self.compile_range_statement(pkg, range),
            Statement::Label(lstmt) => {
                let label = lstmt.name.name.clone();
                let inner = &lstmt.stmt;
                if let Statement::For(f) = inner.as_ref() {
                    self.label_contexts.insert((f.pos, 0), label);
                }
                self.compile_statement(pkg, inner)?;
                Ok(None)
            }
            _ => Err(self.unsupported(&format!("statement: {:#?}", stmt))),
        }
    }

    fn compile_for_statement(
        &mut self,
        pkg: &str,
        forstmt: &ForStmt,
    ) -> Result<Option<bool>, Error> {
        self.symbols.enter_scope();
        let label = self.label_contexts.get(&(forstmt.pos, 0)).cloned();

        if let Some(init) = &forstmt.init {
            self.compile_statement(pkg, init.as_ref())?;
        }

        let depth = self.nesting_depth;
        self.contexts
            .push(Context::For(LoopContext::new(label, depth)));

        // WASM pattern:
        //   block $break          ;; depth+1: break target
        //     loop $loop          ;; depth+2: loop restart (after post)
        //       <condition check>
        //       br_if $break      ;; exit if condition false
        //       block $continue   ;; depth+3: continue target (skips to post)
        //         <body>
        //       end
        //       <post statement>
        //       br $loop          ;; restart loop
        //     end
        //   end
        self.wasm.active().emit(&Instruction::Block(BlockType::Empty));
        self.nesting_depth += 1;
        self.wasm.active().emit(&Instruction::Loop(BlockType::Empty));
        self.nesting_depth += 1;

        let has_cond = forstmt.cond.is_some();
        if let Some(cond) = &forstmt.cond {
            if let Statement::Expr(expr_stmt) = cond.as_ref() {
                self.compile_expression(pkg, &expr_stmt.expr)?;
            } else {
        self.compile_statement(pkg, cond.as_ref())?;
            }
            self.wasm.active().emit(&Instruction::I32Eqz);
            self.wasm.active().emit(&Instruction::BrIf(1));
        }

        self.wasm.active().emit(&Instruction::Block(BlockType::Empty));
        self.nesting_depth += 1;

        let terminate = self.compile_block_statement(pkg, &forstmt.body.list)?;

        self.nesting_depth -= 1;
        self.wasm.active().emit(&Instruction::End); // end continue block

            if let Some(post) = &forstmt.post {
                self.compile_statement(pkg, post.as_ref())?;
            }

        self.wasm.active().emit(&Instruction::Br(0));

        self.nesting_depth -= 1;
        self.wasm.active().emit(&Instruction::End); // end loop
        self.nesting_depth -= 1;
        self.wasm.active().emit(&Instruction::End); // end break block

        let ctx = self.contexts.pop().unwrap().to_for();

        let loop_terminates = (terminate.unwrap_or_default() || forstmt.body.list.is_empty())
            && !has_cond
            && !ctx.has_break;

        self.symbols.leave_scope();
        Ok(Some(loop_terminates))
    }

    fn compile_range_statement(
        &mut self,
        pkg: &str,
        range: &RangeStmt,
    ) -> Result<Option<bool>, Error> {
        self.symbols.enter_scope();

        let coll_dt = self.compile_expression(pkg, &range.expr)?;

        let (elem_dt, data_ptr_local, len_local);

        if let Some((arr_elem, arr_len)) = Self::unwrap_array_elem(&coll_dt) {
            elem_dt = arr_elem;
            let e_size = elem_byte_size(&elem_dt);
            let _ = e_size;

            // Array pointer IS the data pointer.
            data_ptr_local = self.next_wasm_local;
            self.wasm.active().local_set(data_ptr_local);
            self.next_wasm_local = data_ptr_local + 1;

            // Length is a compile-time constant stored in a local for the loop.
            len_local = self.next_wasm_local;
            self.next_wasm_local += 1;
            self.wasm.active().i32_const(arr_len as i32);
            self.wasm.active().local_set(len_local);
        } else {
            let slice_elem = Self::unwrap_slice_elem(&coll_dt)
                .ok_or_else(|| Error::TypeError(format!("range over non-iterable type {:?}", coll_dt)))?;
            elem_dt = slice_elem;

            let hdr_local = self.next_wasm_local;
            self.wasm.active().local_set(hdr_local);
            self.next_wasm_local = hdr_local + 1;

            data_ptr_local = self.next_wasm_local;
            self.next_wasm_local += 1;
            len_local = self.next_wasm_local;
            self.next_wasm_local += 1;

            self.wasm.active().local_get(hdr_local);
            self.wasm.active().i32_load(SLICE_DATA_PTR_OFFSET as u64);
            self.wasm.active().local_set(data_ptr_local);

            self.wasm.active().local_get(hdr_local);
            self.wasm.active().i32_load(SLICE_LEN_OFFSET as u64);
            self.wasm.active().local_set(len_local);
        }

        let e_size = elem_byte_size(&elem_dt);

        // Counter local: i = 0
        let i_local = self.next_wasm_local;
        self.next_wasm_local += 1;
        self.wasm.active().i32_const(0);
        self.wasm.active().local_set(i_local);

        // Bind key variable (index) if present
        if let Some(Expression::Ident(key_id)) = &range.key {
            if key_id.name != "_" {
                let sym = self.symbols.define(
                    pkg,
                    &key_id.name,
                    DefineType::Qualified(Qualifier::Var, Box::new(DefineType::Int)),
                    false,
                );
                self.locals.insert(sym.index, i_local);
            }
        }

        // Value local
        let val_local = if let Some(Expression::Ident(val_id)) = &range.value {
            if val_id.name != "_" {
                let vl = self.next_wasm_local;
                if Self::is_string_type(&elem_dt) {
                    self.next_wasm_local += 2;
                } else {
                    self.next_wasm_local += 1;
                }
                let sym = self.symbols.define(
                    pkg,
                    &val_id.name,
                    DefineType::Qualified(Qualifier::Var, Box::new(elem_dt.clone())),
                    false,
                );
                self.locals.insert(sym.index, vl);
                Some(vl)
            } else {
                None
            }
        } else {
            None
        };

        // Loop: block $break { loop $loop { ... } }
        self.contexts.push(Context::For(LoopContext {
            depth: self.nesting_depth,
            has_break: false,
            has_continue: false,
            label: None,
        }));

        self.wasm.active().emit(&Instruction::Block(BlockType::Empty));
        self.nesting_depth += 1;
        self.wasm.active().emit(&Instruction::Loop(BlockType::Empty));
        self.nesting_depth += 1;

        // Condition: i < len
        self.wasm.active().local_get(i_local);
        self.wasm.active().local_get(len_local);
        self.wasm.active().emit(&Instruction::I32GeU);
        self.wasm.active().emit(&Instruction::BrIf(1));

        // Load value if needed
        if let Some(vl) = val_local {
            // addr = data_ptr + i * e_size
            self.wasm.active().local_get(data_ptr_local);
            self.wasm.active().local_get(i_local);
            self.wasm.active().i32_const(e_size as i32);
            self.wasm.active().emit(&Instruction::I32Mul);
            self.wasm.active().emit(&Instruction::I32Add);

            // Load element
            self.emit_elem_load(&elem_dt);

            // Store in value local
            if Self::is_string_type(&elem_dt) {
                self.wasm.active().local_set(vl + 1);
                self.wasm.active().local_set(vl);
            } else {
                self.wasm.active().local_set(vl);
            }
        }

        // Continue block
        self.wasm.active().emit(&Instruction::Block(BlockType::Empty));
        self.nesting_depth += 1;

        self.compile_block_statement(pkg, &range.body.list)?;

        self.nesting_depth -= 1;
        self.wasm.active().emit(&Instruction::End); // end continue block

        // i++
        self.wasm.active().local_get(i_local);
        self.wasm.active().i32_const(1);
        self.wasm.active().emit(&Instruction::I32Add);
        self.wasm.active().local_set(i_local);

        self.wasm.active().emit(&Instruction::Br(0)); // loop restart

        self.nesting_depth -= 1;
        self.wasm.active().emit(&Instruction::End); // end loop
        self.nesting_depth -= 1;
        self.wasm.active().emit(&Instruction::End); // end break block

        self.contexts.pop();
        self.symbols.leave_scope();
        Ok(None)
    }

    fn compile_if_statement(
        &mut self,
        pkg: &str,
        ifstmt: &IfStmt,
    ) -> Result<Option<bool>, Error> {
        if let Some(init) = &ifstmt.init {
            self.compile_statement(pkg, init.as_ref())?;
        }

        self.compile_expression(pkg, &ifstmt.cond)?;

        self.wasm
            .active()
            .emit(&Instruction::If(BlockType::Empty));
        self.nesting_depth += 1;

        let terminates = self.compile_block_statement(pkg, &ifstmt.body.list)?;

        let mut else_terminates = None;

        if let Some(alternative) = &ifstmt.else_ {
            self.wasm.active().emit(&Instruction::Else);

            match alternative.as_ref() {
                Statement::Block(bl) => {
                    else_terminates = self.compile_block_statement(pkg, &bl.list)?;
                }
                Statement::If(_) => {
                    self.compile_statement(pkg, alternative.as_ref())?;
                }
                _ => panic!("else should be a block: {:#?}", alternative),
            }
        }

        self.nesting_depth -= 1;
        self.wasm.active().emit(&Instruction::End);

        let terminates =
            terminates.unwrap_or_default() && else_terminates.unwrap_or_default();

        Ok(Some(terminates))
    }

    fn compile_assign_statement(
        &mut self,
        pkg: &str,
        assign: &AssignStmt,
    ) -> Result<Option<bool>, Error> {
        if assign.left.len() != assign.right.len() && assign.right.len() != 1 {
                return Err(Error::TypeError(format!(
                    "assignment mismatch: {} variables but {} values",
                assign.left.len(),
                assign.right.len()
            )));
        }

        for (left, right) in assign.left.iter().zip(assign.right.iter()) {
            match &assign.op {
                Operator::AddAssign
                | Operator::SubAssign
                | Operator::MulAssign
                | Operator::QuoAssign
                | Operator::RemAssign => {
                    let arith_op = match assign.op {
                        Operator::AddAssign => Operator::Add,
                        Operator::SubAssign => Operator::Sub,
                        Operator::MulAssign => Operator::Star,
                        Operator::QuoAssign => Operator::Quo,
                        Operator::RemAssign => Operator::Rem,
                        _ => unreachable!(),
                    };
                    self.compile_statement(
                        pkg,
                        &Statement::Assign(AssignStmt {
                            pos: 0,
                            op: Operator::Assign,
                            left: vec![left.clone()],
                            right: vec![Expression::Operation(Operation {
                                pos: 0,
                                op: arith_op,
                                x: Box::new(left.clone()),
                                y: Some(Box::new(right.clone())),
                            })],
                        }),
                    )?;
                }
                Operator::Define => {
                    let name = match left {
                        Expression::Ident(ident) => &ident.name,
                        _ => {
                            return Err(Error::SyntaxError(format!(
                                "non-identifier on left side of :=: {:#?}",
                                left
                            )))
                        }
                    };

                    let rt = self.compile_expression(pkg, right)?;

                    let symbol = self.symbols.define(
                        pkg,
                        name.as_str(),
                        DefineType::Qualified(Qualifier::Var, Box::new(rt.clone())),
                        rt.is_invar(),
                    );

                    // Memory-backed variable: store value to linear memory
                    if let Some(mv) = self.mem_vars.get(name.as_str()).cloned() {
                        let tmp = self.next_wasm_local;
                        self.wasm.active().local_set(tmp);
                        self.wasm.active().local_get(mv.addr_local);
                        self.wasm.active().local_get(tmp);
                        self.wasm.active().i32_store(0);
                    } else if self.escaped_vars.contains(name.as_str()) {
                        // Escaping var: allocate on heap at declaration time
                        let size = crate::wasm::layout::field_byte_size(&rt);
                        let rt_alloc_idx = self.wasm.rt_alloc_func_idx()
                            .expect("rt_alloc not registered");
                        let val_tmp = self.next_wasm_local;
                        self.wasm.active().local_set(val_tmp);
                        self.wasm.active().i32_const(size as i32);
                        self.wasm.active().call(rt_alloc_idx);
                        let addr_local = self.next_wasm_local + 1;
                        self.wasm.active().local_tee(addr_local);
                        self.wasm.active().local_get(val_tmp);
                        self.wasm.active().i32_store(0);
                        self.mem_vars.insert(name.to_string(), MemVar { addr_local, size });
                        self.next_wasm_local = addr_local + 1;
                    } else if rt.is_func() {
                        if let DefineType::Func { name: ref fname, .. } = rt {
                            if let Some(&fidx) = self.wasm_func_map.get(fname) {
                                self.closure_var_func.insert(symbol.index, fidx);
                            }
                        }
                    } else if Self::is_string_type(&rt) {
                        let base = self.next_wasm_local;
                        self.next_wasm_local += 2;
                        self.locals.insert(symbol.index, base);

                        self.wasm.active().local_set(base + 1);
                        self.wasm.active().local_set(base);
                    } else {
                        let local_idx = self.next_wasm_local;
                        self.next_wasm_local += 1;
                        self.locals.insert(symbol.index, local_idx);

                        self.wasm.active().local_set(local_idx);
                    }
                }
                Operator::Assign => {
                    // *p = v (dereference write)
                    if let Expression::Operation(deref) = left {
                        if deref.op == Operator::Star && deref.y.is_none() {
                            let ptr_dt = self.compile_expression(pkg, deref.x.as_ref())?;
                            let _inner = match ptr_dt.unwrap_qualifiers() {
                                DefineType::Ref(inner) => *inner,
                                other => return Err(Error::TypeError(
                                    format!("cannot dereference non-pointer type {:?}", other)
                                )),
                            };
                            self.emit_nil_check();
                            let addr_tmp = self.next_wasm_local;
                            self.wasm.active().local_set(addr_tmp);
                            let saved_next = self.next_wasm_local;
                            self.next_wasm_local = addr_tmp + 1;
                            self.compile_expression(pkg, right)?;
                            let val_tmp = self.next_wasm_local;
                            self.wasm.active().local_set(val_tmp);
                            self.wasm.active().local_get(addr_tmp);
                            self.wasm.active().local_get(val_tmp);
                            self.wasm.active().i32_store(0);
                            self.next_wasm_local = saved_next;
                            continue;
                        }
                    }

                    if let Expression::Selector(sel) = left {
                        let obj_dt = self.compile_expression(pkg, &sel.x)?;
                        self.emit_nil_check();
                        let fields = self.resolve_struct_fields(pkg, &obj_dt)?;
                        let (layout, _) = struct_field_layout(&fields);
                        let field_name = &sel.sel.name;
                        let (_, offset, field_dt) = layout.iter()
                            .find(|(n, _, _)| n == field_name)
                            .ok_or_else(|| Error::ReferenceError(format!(
                                "no field '{}' on struct {:?}", field_name, obj_dt
                            )))?;
                        let offset = *offset;
                        let field_dt = field_dt.clone();

                        let base_local = self.next_wasm_local;
                        self.wasm.active().local_set(base_local);

                        let saved_next = self.next_wasm_local;
                        self.next_wasm_local = base_local + 1;
                        self.compile_expression(pkg, right)?;
                        self.emit_field_store(base_local, offset, &field_dt);
                        self.next_wasm_local = saved_next;
                        continue;
                    }

                    if let Expression::Index(idx) = left {
                        let coll_dt = self.compile_expression(pkg, &idx.left)?;

                        if let Some((elem_dt, _arr_len)) = Self::unwrap_array_elem(&coll_dt) {
                            let e_size = elem_byte_size(&elem_dt);
                            let arr_local = self.next_wasm_local;
                            self.wasm.active().local_set(arr_local);
                            let saved_next = self.next_wasm_local;
                            self.next_wasm_local = arr_local + 1;

                            // addr = arr_ptr + idx * e_size  (no header)
                            self.wasm.active().local_get(arr_local);
                            self.compile_expression(pkg, &idx.index)?;
                            self.wasm.active().i32_const(e_size as i32);
                            self.wasm.active().emit(&Instruction::I32Mul);
                            self.wasm.active().emit(&Instruction::I32Add);
                            let addr_local = self.next_wasm_local;
                            self.wasm.active().local_set(addr_local);
                            self.next_wasm_local = addr_local + 1;

                            self.compile_expression(pkg, right)?;
                            self.emit_elem_store(addr_local, &elem_dt);
                            self.next_wasm_local = saved_next;
                            continue;
                        }

                        self.emit_nil_check();

                        let elem_dt = Self::unwrap_slice_elem(&coll_dt)
                            .ok_or_else(|| Error::TypeError(format!(
                                "index assign on non-indexable type {:?}", coll_dt
                            )))?;
                        let e_size = elem_byte_size(&elem_dt);

                        let hdr_local = self.next_wasm_local;
                        self.wasm.active().local_set(hdr_local);

                        let saved_next = self.next_wasm_local;
                        self.next_wasm_local = hdr_local + 1;

                        // Load data_ptr
                        self.wasm.active().local_get(hdr_local);
                        self.wasm.active().i32_load(SLICE_DATA_PTR_OFFSET as u64);

                        // Compile index
                        self.compile_expression(pkg, &idx.index)?;

                        // addr = data_ptr + idx * e_size
                        self.wasm.active().i32_const(e_size as i32);
                        self.wasm.active().emit(&Instruction::I32Mul);
                        self.wasm.active().emit(&Instruction::I32Add);
                        let addr_local = self.next_wasm_local;
                        self.wasm.active().local_set(addr_local);
                        self.next_wasm_local = addr_local + 1;

                        // Compile RHS value
                        self.compile_expression(pkg, right)?;
                        self.emit_elem_store(addr_local, &elem_dt);
                        self.next_wasm_local = saved_next;
                        continue;
                    }

                    let name = match &left {
                        Expression::Ident(name) => name.name.to_string(),
                        _ => {
                            return Err(self.unsupported(&format!(
                                "non-ident assignment target: {:#?}",
                                left
                            )))
                        }
                    };

                    if name == "_" {
                        let rt = self.compile_expression(pkg, right)?;
                        if Self::is_string_type(&rt) {
                            self.wasm.active().drop();
                            self.wasm.active().drop();
                                } else {
                            self.wasm.active().drop();
                        }
                        continue;
                    }

                    if let Some(mv) = self.mem_vars.get(&name).cloned() {
                        self.compile_expression(pkg, right)?;
                        let tmp = self.next_wasm_local;
                        self.wasm.active().local_set(tmp);
                        self.wasm.active().local_get(mv.addr_local);
                        self.wasm.active().local_get(tmp);
                        self.wasm.active().i32_store(0);
                        continue;
                    }

                    let resolved = self.symbols.resolve(pkg, &name).ok_or(
                        Error::ReferenceError(format!("assign: `{name}` is not defined")),
                    )?;

                    self.compile_expression(pkg, right)?;

                    match resolved {
                        Resolved::Local((symbol, dt, _)) => {
                            if let Some(&local_idx) = self.locals.get(&symbol.index) {
                                if Self::is_string_type(&dt) {
                                    self.wasm.active().local_set(local_idx + 1);
                                    self.wasm.active().local_set(local_idx);
                            } else {
                                    self.wasm.active().local_set(local_idx);
                                }
                            } else {
                                return Err(Error::InternalError(format!(
                                    "no WASM local for symbol '{}' (idx={})",
                                    name, symbol.index
                            )));
                        }
                    }
                        _ => {
                            return Err(
                                self.unsupported("enclosed variable assignment in WASM")
                            )
                        }
                    }
                }
                _ => return Err(self.unsupported(&format!("assign op: {:?}", assign.op))),
            }
        }
        Ok(None)
    }

    fn compile_return_statement(
        &mut self,
        pkg: &str,
        expr: &ReturnStmt,
    ) -> Result<Option<bool>, Error> {
        let mut rts = Vec::with_capacity(expr.ret.len());

        for r in &expr.ret {
            let t = self.compile_expression(pkg, r)?;
            rts.push(t);
        }

        let rt = if rts.is_empty() {
            DefineType::Null
        } else if rts.len() == 1 {
            rts[0].clone()
        } else {
            DefineType::Tuple(rts)
        };

        self.func_contexts
            .last_mut()
            .unwrap()
            .ret_types
            .push((rt, false));

        // Restore $sp before returning.
        if let (Some(saved_sp), Some(sp_idx)) = (self.saved_sp_local, self.wasm.sp_global_idx()) {
            self.wasm.active().local_get(saved_sp);
            self.wasm.active().global_set(sp_idx);
        }

        self.wasm.active().ret();

        Ok(Some(true))
    }

    fn compile_branch_statement(
        &mut self,
        _pkg: &str,
        branch: &BranchStmt,
    ) -> Result<Option<bool>, Error> {
        match branch.key {
            Keyword::Break => {
                let loop_depth = if let Some(l) = branch.ident.clone() {
                    let mut found_depth = None;
                    for ctx in self.contexts.iter_mut().rev() {
                        if let Some(label) = ctx.label() {
                            if &l.name == label {
                                ctx.push_break(0);
                                found_depth = Some(ctx.as_for().depth);
                                break;
                            }
                        }
                    }
                    found_depth.ok_or_else(|| {
                        Error::SyntaxError(format!("label not found: {:?}", branch.ident))
                    })?
                } else {
                    let ctx = self
                        .contexts
                        .last_mut()
                        .ok_or_else(|| Error::SyntaxError("break outside loop".to_string()))?;
                    ctx.push_break(0);
                    ctx.as_for().depth
                };
                // break targets the outer block at loop_depth + 1
                let br_depth = self.nesting_depth - (loop_depth + 1);
                self.wasm.active().emit(&Instruction::Br(br_depth));
            }
            Keyword::Continue => {
                let loop_depth = if let Some(l) = branch.ident.clone() {
                    let mut found_depth = None;
                    for ctx in self.contexts.iter_mut().rev() {
                        if let Some(label) = ctx.label() {
                            if &l.name == label {
                                ctx.push_continue(0);
                                found_depth = Some(ctx.as_for().depth);
                                break;
                            }
                        }
                    }
                    found_depth.ok_or_else(|| {
                        Error::SyntaxError(format!("label not found: {:?}", branch.ident))
                    })?
                } else {
                    let ctx = self
                        .contexts
                        .last_mut()
                        .ok_or_else(|| {
                            Error::SyntaxError("continue outside loop".to_string())
                        })?;
                    ctx.push_continue(0);
                    ctx.as_for().depth
                };
                // continue targets the continue-block at loop_depth + 3
                // (falls through to post statement, then br 0 restarts loop)
                let br_depth = self.nesting_depth - (loop_depth + 3);
                self.wasm.active().emit(&Instruction::Br(br_depth));
            }
            _ => return Err(self.unsupported(&format!("branch keyword: {:?}", branch.key))),
        }
        Ok(None)
    }

    fn compile_incdec_statement(
        &mut self,
        pkg: &str,
        incdec: &IncDecStmt,
    ) -> Result<Option<bool>, Error> {
        let arith_op = if incdec.op == Operator::Inc {
            Operator::Add
        } else {
            Operator::Sub
        };
        self.compile_statement(
            pkg,
            &Statement::Assign(AssignStmt {
                pos: 0,
                op: Operator::Assign,
                left: vec![incdec.expr.clone()],
                right: vec![Expression::Operation(Operation {
                    pos: 0,
                    op: arith_op,
                    x: Box::new(incdec.expr.clone()),
                    y: Some(Box::new(Expression::BasicLit(BasicLit {
                        pos: 0,
                        kind: LitKind::Integer,
                        value: "1".to_string(),
                    }))),
                })],
            }),
        )
    }

    pub(crate) fn compile_expression(
        &mut self,
        pkg: &str,
        expr: &Expression,
    ) -> Result<DefineType, Error> {
        match expr {
            Expression::Call(call) => self.compile_call_expression(pkg, call),
            Expression::Operation(op) => self.compile_operation_expression(pkg, op),
            Expression::BasicLit(lit)
                if lit.kind == LitKind::Ident && lit.value == "iota" =>
            {
                self.compile_expression(
                    pkg,
                    &Expression::BasicLit(BasicLit {
            pos: 0,
                        kind: LitKind::Integer,
                        value: self.iota.to_string(),
                    }),
                )
            }
            Expression::BasicLit(lit) if lit.kind == LitKind::String => {
                let raw = &lit.value;
                let unquoted = if raw.starts_with('"') && raw.ends_with('"') {
                    &raw[1..raw.len() - 1]
                } else if raw.starts_with('`') && raw.ends_with('`') {
                    &raw[1..raw.len() - 1]
                } else {
                    raw.as_str()
                };
                let bytes = Self::unescape_go_string(unquoted);
                let len = bytes.len() as i32;

                let rt_alloc_idx = self.wasm.rt_alloc_func_idx()
                    .ok_or_else(|| Error::InternalError("rt_alloc not registered".into()))?;

                // rt_alloc(len) -> ptr
                self.wasm.active().i32_const(len);
                self.wasm.active().call(rt_alloc_idx);

                // Store ptr in scratch local, write each byte
                let scratch = self.next_wasm_local;
                self.wasm.active().local_set(scratch);

                for (i, &b) in bytes.iter().enumerate() {
                    self.wasm.active().local_get(scratch);
                    if i > 0 {
                        self.wasm.active().i32_const(i as i32);
                        self.wasm.active().emit(&Instruction::I32Add);
                    }
                    self.wasm.active().i32_const(b as i32);
                    self.wasm.active().i32_store8(0);
                }

                // Push (ptr, len) onto the stack
                self.wasm.active().local_get(scratch);
                self.wasm.active().i32_const(len);

                Ok(DefineType::String)
            }
            Expression::BasicLit(lit) if lit.kind == LitKind::Float => {
                match lit.value.parse::<f64>() {
                    Ok(f) => {
                        self.wasm
                            .active()
                            .emit(&Instruction::F64Const(f.into()));
                        Ok(DefineType::Float64)
                    }
                    _ => {
                        let f: f32 = lit.value.parse().unwrap();
                        self.wasm
                            .active()
                            .emit(&Instruction::F32Const(f.into()));
                        Ok(DefineType::Float32)
                    }
                }
            }
            Expression::BasicLit(lit) if lit.kind == LitKind::Integer => {
                let value = lit
                    .value
                    .parse::<isize>()
                    .or_else(|_| isize::from_str_radix(&lit.value, 16))
                    .unwrap();
                self.wasm.active().i32_const(value as i32);
                Ok(DefineType::Qualified(
                    Qualifier::Const,
                    Box::new(DefineType::Int),
                ))
            }
            Expression::BasicLit(lit) if lit.kind == LitKind::Ident => {
                if let Some(mv) = self.mem_vars.get(&lit.value).cloned() {
                    let resolved = self.symbols.resolve(pkg, &lit.value).ok_or(
                        Error::ReferenceError(format!("identifier: {} not found", lit.value)),
                    )?;
                    let dt = resolved.get_type().0;
                    self.wasm.active().local_get(mv.addr_local);
                    self.wasm.active().i32_load(0);
                    return Ok(dt);
                }

                let resolved = self.symbols.resolve(pkg, &lit.value).ok_or(
                    Error::ReferenceError(format!("identifier: {} not found", lit.value)),
                )?;

                match resolved {
                    Resolved::Local((symbol, dt, _)) => {
                        if let Some(&local_idx) = self.locals.get(&symbol.index) {
                            if Self::is_string_type(&dt) {
                                self.wasm.active().local_get(local_idx);
                                self.wasm.active().local_get(local_idx + 1);
                            } else {
                                self.wasm.active().local_get(local_idx);
                            }
                        } else {
                            self.emit_builtin_value(&lit.value, &dt);
                        }
                        Ok(dt)
                    }
                    _ => {
                        match resolved {
                            Resolved::Enclosed((symbol, dt, _)) => {
                                if let Some(&local_idx) = self.locals.get(&symbol.index) {
                                    if Self::is_string_type(&dt) {
                                        self.wasm.active().local_get(local_idx);
                                        self.wasm.active().local_get(local_idx + 1);
                        } else {
                                        self.wasm.active().local_get(local_idx);
                                    }
                                } else {
                                    self.emit_builtin_value(&lit.value, &dt);
                                }
                                Ok(dt)
                            }
                            _ => Err(self.unsupported("enclosed variable access")),
                        }
                    }
                }
            }
            Expression::Ident(ident) => {
                if let Some(mv) = self.mem_vars.get(&ident.name).cloned() {
                    let dt = self.symbols.resolve(pkg, &ident.name)
                        .map(|r| r.get_type().0)
                        .unwrap_or(DefineType::Int);
                    self.wasm.active().local_get(mv.addr_local);
                    self.wasm.active().i32_load(0);
                    return Ok(dt);
                }

                if ident.name == "iota" {
                    return self.compile_expression(
                        pkg,
                        &Expression::BasicLit(BasicLit {
                        pos: 0,
                            kind: LitKind::Integer,
                            value: self.iota.to_string(),
                        }),
                    );
                }

                match self.symbols.resolve(pkg, &ident.name) {
                    Some(Resolved::Local((symbol, dt, _))) => {
                        if let Some(&local_idx) = self.locals.get(&symbol.index) {
                            if Self::is_string_type(&dt) {
                                self.wasm.active().local_get(local_idx);
                                self.wasm.active().local_get(local_idx + 1);
                            } else {
                                self.wasm.active().local_get(local_idx);
                            }
                        } else {
                            self.emit_builtin_value(&ident.name, &dt);
                        }
                        Ok(dt)
                    }
                    Some(Resolved::Enclosed((symbol, dt, _))) => {
                        if let Some(&local_idx) = self.locals.get(&symbol.index) {
                            if Self::is_string_type(&dt) {
                                self.wasm.active().local_get(local_idx);
                                self.wasm.active().local_get(local_idx + 1);
                    } else {
                                self.wasm.active().local_get(local_idx);
                            }
                        } else {
                            self.emit_builtin_value(&ident.name, &dt);
                        }
                        Ok(dt)
                    }
                    None => Err(Error::ReferenceError(format!(
                        "ident: `{}` is not defined in pkg: `{}`",
                        ident.name, pkg,
                    ))),
                }
            }
            Expression::CompositeLit(cl) => {
                if let Expression::TypeSlice(ts) = cl.typ.as_ref() {
                    let elem_dt = self.expression_to_define_type(pkg, &ts.typ)
                        .ok_or_else(|| Error::TypeError("cannot resolve slice element type".into()))?;
                    let e_size = elem_byte_size(&elem_dt);
                    let n = cl.val.values.len() as u32;

                    let rt_alloc_idx = self.wasm.rt_alloc_func_idx()
                        .ok_or_else(|| Error::InternalError("rt_alloc not registered".into()))?;

                    // Allocate data block
                    self.wasm.active().i32_const((n * e_size) as i32);
                    self.wasm.active().call(rt_alloc_idx);
                    let data_ptr_local = self.next_wasm_local;
                    self.wasm.active().local_set(data_ptr_local);

                    let saved_next = self.next_wasm_local;
                    self.next_wasm_local = data_ptr_local + 1;

                    // Store each element
                    for (i, kv) in cl.val.values.iter().enumerate() {
                        let val_expr = match &kv.val {
                            Element::Expr(e) => e,
                            _ => return Err(self.unsupported("nested literal value in slice")),
                        };

                        // Compute target address: data_ptr + i * e_size
                        self.wasm.active().local_get(data_ptr_local);
                        self.wasm.active().i32_const((i as u32 * e_size) as i32);
                        self.wasm.active().emit(&Instruction::I32Add);
                        let addr_local = self.next_wasm_local;
                        self.wasm.active().local_set(addr_local);
                        self.next_wasm_local = addr_local + 1;

                        self.compile_expression(pkg, val_expr)?;
                        self.emit_elem_store(addr_local, &elem_dt);

                        self.next_wasm_local = addr_local + 1;
                    }

                    // Allocate header
                    self.wasm.active().i32_const(SLICE_HEADER_SIZE as i32);
                    self.wasm.active().call(rt_alloc_idx);
                    let hdr_local = self.next_wasm_local;
                    self.wasm.active().local_set(hdr_local);

                    // Store data_ptr at hdr+0
                    self.wasm.active().local_get(hdr_local);
                    self.wasm.active().local_get(data_ptr_local);
                    self.wasm.active().i32_store(SLICE_DATA_PTR_OFFSET as u64);

                    // Store len at hdr+4
                    self.wasm.active().local_get(hdr_local);
                    self.wasm.active().i32_const(n as i32);
                    self.wasm.active().i32_store(SLICE_LEN_OFFSET as u64);

                    // Store cap at hdr+8 (cap = len for literals)
                    self.wasm.active().local_get(hdr_local);
                    self.wasm.active().i32_const(n as i32);
                    self.wasm.active().i32_store(SLICE_CAP_OFFSET as u64);

                    self.next_wasm_local = saved_next;
                    self.wasm.active().local_get(hdr_local);

                    let slice_dt = DefineType::Slice(Box::new(elem_dt));
                    return Ok(slice_dt);
                }

                if let Expression::TypeArray(ta) = cl.typ.as_ref() {
                    let elem_dt = self.expression_to_define_type(pkg, &ta.typ)
                        .ok_or_else(|| Error::TypeError("cannot resolve array element type".into()))?;
                    let arr_len: usize = match ta.len.as_ref() {
                        Expression::BasicLit(lit) if lit.kind == LitKind::Integer => {
                            lit.value.parse().map_err(|_| {
                                Error::TypeError(format!("invalid array length: {}", lit.value))
                            })?
                        }
                        _ => return Err(self.unsupported("non-constant array length")),
                    };
                    let e_size = elem_byte_size(&elem_dt);
                    let total = array_byte_size(&elem_dt, arr_len);

                    let sp_idx = self.wasm.sp_global_idx()
                        .expect("$sp global not registered");

                    // $sp -= total  (allocate on stack)
                    self.wasm.active().global_get(sp_idx);
                    self.wasm.active().i32_const(total as i32);
                    self.wasm.active().emit(&Instruction::I32Sub);
                    self.wasm.active().global_set(sp_idx);

                    // arr_ptr = $sp
                    let arr_local = self.next_wasm_local;
                    self.wasm.active().global_get(sp_idx);
                    self.wasm.active().local_set(arr_local);
                    let saved_next = self.next_wasm_local;
                    self.next_wasm_local = arr_local + 1;

                    for (i, kv) in cl.val.values.iter().enumerate() {
                        let val_expr = match &kv.val {
                            Element::Expr(e) => e,
                            _ => return Err(self.unsupported("nested literal value in array")),
                        };

                        self.wasm.active().local_get(arr_local);
                        self.wasm.active().i32_const((i as u32 * e_size) as i32);
                        self.wasm.active().emit(&Instruction::I32Add);
                        let addr_local = self.next_wasm_local;
                        self.wasm.active().local_set(addr_local);
                        self.next_wasm_local = addr_local + 1;

                        self.compile_expression(pkg, val_expr)?;
                        self.emit_elem_store(addr_local, &elem_dt);
                        self.next_wasm_local = addr_local + 1;
                    }

                    self.next_wasm_local = saved_next;
                    self.wasm.active().local_get(arr_local);

                    return Ok(DefineType::Array {
                        inner_type: Box::new(elem_dt),
                        len: arr_len,
                    });
                }

                let type_name = cl.typ.as_ident().map_err(|e| Error::TypeError(e))?;
                let resolved = self.symbols.resolve(pkg, &type_name.name)
                    .ok_or_else(|| Error::ReferenceError(format!("unresolved type '{}'", type_name.name)))?;
                let struct_dt = resolved.get_type().0.unwrap_qualifiers();
                let fields = self.resolve_struct_fields(pkg, &struct_dt)?;
                let (layout, total_size) = struct_field_layout(&fields);

                let rt_alloc_idx = self.wasm.rt_alloc_func_idx()
                    .ok_or_else(|| Error::InternalError("rt_alloc not registered".into()))?;

                self.wasm.active().i32_const(total_size as i32);
                self.wasm.active().call(rt_alloc_idx);

                let scratch = self.next_wasm_local;
                self.wasm.active().local_set(scratch);
                let saved_next = self.next_wasm_local;
                self.next_wasm_local = scratch + 1;

                for kv in &cl.val.values {
                    let key_name = match &kv.key {
                        Some(Element::Expr(Expression::Ident(id))) => id.name.clone(),
                        _ => return Err(Error::SyntaxError(
                            "struct literal field must be a named key".into(),
                        )),
                    };
                    let val_expr = match &kv.val {
                        Element::Expr(e) => e,
                        _ => return Err(self.unsupported("nested literal value in struct")),
                    };

                    let (_, offset, field_dt) = layout.iter()
                        .find(|(n, _, _)| n == &key_name)
                        .ok_or_else(|| Error::ReferenceError(format!(
                            "struct '{}' has no field '{}'", type_name.name, key_name
                        )))?;
                    let offset = *offset;
                    let field_dt = field_dt.clone();

                    self.compile_expression(pkg, val_expr)?;
                    self.emit_field_store(scratch, offset, &field_dt);
                }

                self.next_wasm_local = saved_next;
                self.wasm.active().local_get(scratch);
                Ok(struct_dt)
            }
            Expression::Selector(sel) => {
                if let Expression::Ident(ref id) = *sel.x {
                    if let Some(p) = self.symbols.get_package_path(&id.name) {
                        let member = Expression::Ident(sel.sel.clone());
                        return self.compile_expression(&p, &member);
                    }
                }
                let obj_dt = self.compile_expression(pkg, &sel.x)?;
                self.emit_nil_check();
                let fields = self.resolve_struct_fields(pkg, &obj_dt)?;
                let (layout, _) = struct_field_layout(&fields);
                let field_name = &sel.sel.name;
                let (_, offset, field_dt) = layout.iter()
                    .find(|(n, _, _)| n == field_name)
                    .ok_or_else(|| Error::ReferenceError(format!(
                        "no field '{}' on struct {:?}", field_name, obj_dt
                    )))?;
                let offset = *offset as u64;
                let field_dt = field_dt.clone();
                self.emit_field_load(offset, &field_dt);
                Ok(field_dt)
            }
            Expression::FuncLit(fl) => {
                let func_dt = self.compile_func_lit(pkg, fl)?;
                Ok(func_dt)
            }
            Expression::Index(idx) => {
                let coll_dt = self.compile_expression(pkg, &idx.left)?;

                if let Some((elem_dt, _arr_len)) = Self::unwrap_array_elem(&coll_dt) {
                    let e_size = elem_byte_size(&elem_dt);
                    let arr_local = self.next_wasm_local;
                    self.wasm.active().local_set(arr_local);
                    let saved_next = self.next_wasm_local;
                    self.next_wasm_local = arr_local + 1;

                    // ptr + idx * e_size  (no header indirection)
                    self.wasm.active().local_get(arr_local);
                    self.compile_expression(pkg, &idx.index)?;
                    self.wasm.active().i32_const(e_size as i32);
                    self.wasm.active().emit(&Instruction::I32Mul);
                    self.wasm.active().emit(&Instruction::I32Add);

                    self.emit_elem_load(&elem_dt);
                    self.next_wasm_local = saved_next;
                    return Ok(elem_dt);
                }

                self.emit_nil_check();

                let elem_dt = Self::unwrap_slice_elem(&coll_dt)
                    .ok_or_else(|| Error::TypeError(format!("index on non-indexable type {:?}", coll_dt)))?;
                let e_size = elem_byte_size(&elem_dt);

                let hdr_local = self.next_wasm_local;
                self.wasm.active().local_set(hdr_local);

                let saved_next = self.next_wasm_local;
                self.next_wasm_local = hdr_local + 1;

                // Load data_ptr from header
                self.wasm.active().local_get(hdr_local);
                self.wasm.active().i32_load(SLICE_DATA_PTR_OFFSET as u64);

                // Compile index
                self.compile_expression(pkg, &idx.index)?;

                // Compute data_ptr + idx * elem_size
                self.wasm.active().i32_const(e_size as i32);
                self.wasm.active().emit(&Instruction::I32Mul);
                self.wasm.active().emit(&Instruction::I32Add);

                // Load element
                self.emit_elem_load(&elem_dt);

                self.next_wasm_local = saved_next;
                Ok(elem_dt)
            }
            Expression::Slice(sl) => {
                let slice_dt = self.compile_expression(pkg, &sl.left)?;
                let elem_dt = Self::unwrap_slice_elem(&slice_dt)
                    .ok_or_else(|| Error::TypeError(format!("slice expr on non-slice type {:?}", slice_dt)))?;
                let e_size = elem_byte_size(&elem_dt);

                let hdr_local = self.next_wasm_local;
                self.wasm.active().local_set(hdr_local);

                let saved_next = self.next_wasm_local;
                self.next_wasm_local = hdr_local + 1;

                // Load data_ptr, len, cap from old header
                let data_ptr_local = self.next_wasm_local;
                self.next_wasm_local += 1;
                let old_len_local = self.next_wasm_local;
                self.next_wasm_local += 1;
                let old_cap_local = self.next_wasm_local;
                self.next_wasm_local += 1;

                self.wasm.active().local_get(hdr_local);
                self.wasm.active().i32_load(SLICE_DATA_PTR_OFFSET as u64);
                self.wasm.active().local_set(data_ptr_local);

                self.wasm.active().local_get(hdr_local);
                self.wasm.active().i32_load(SLICE_LEN_OFFSET as u64);
                self.wasm.active().local_set(old_len_local);

                self.wasm.active().local_get(hdr_local);
                self.wasm.active().i32_load(SLICE_CAP_OFFSET as u64);
                self.wasm.active().local_set(old_cap_local);

                // Evaluate lo (default 0)
                let lo_local = self.next_wasm_local;
                self.next_wasm_local += 1;
                if let Some(lo_expr) = &sl.index[0] {
                    self.compile_expression(pkg, lo_expr)?;
                } else {
                    self.wasm.active().i32_const(0);
                }
                self.wasm.active().local_set(lo_local);

                // Evaluate hi (default len)
                let hi_local = self.next_wasm_local;
                self.next_wasm_local += 1;
                if let Some(hi_expr) = &sl.index[1] {
                    self.compile_expression(pkg, hi_expr)?;
                } else {
                    self.wasm.active().local_get(old_len_local);
                }
                self.wasm.active().local_set(hi_local);

                // Allocate new header
                let rt_alloc_idx = self.wasm.rt_alloc_func_idx()
                    .ok_or_else(|| Error::InternalError("rt_alloc not registered".into()))?;
                self.wasm.active().i32_const(SLICE_HEADER_SIZE as i32);
                self.wasm.active().call(rt_alloc_idx);
                let new_hdr = self.next_wasm_local;
                self.wasm.active().local_set(new_hdr);

                // new_data_ptr = data_ptr + lo * e_size
                self.wasm.active().local_get(new_hdr);
                self.wasm.active().local_get(data_ptr_local);
                self.wasm.active().local_get(lo_local);
                self.wasm.active().i32_const(e_size as i32);
                self.wasm.active().emit(&Instruction::I32Mul);
                self.wasm.active().emit(&Instruction::I32Add);
                self.wasm.active().i32_store(SLICE_DATA_PTR_OFFSET as u64);

                // new_len = hi - lo
                self.wasm.active().local_get(new_hdr);
                self.wasm.active().local_get(hi_local);
                self.wasm.active().local_get(lo_local);
                self.wasm.active().emit(&Instruction::I32Sub);
                self.wasm.active().i32_store(SLICE_LEN_OFFSET as u64);

                // new_cap = old_cap - lo
                self.wasm.active().local_get(new_hdr);
                self.wasm.active().local_get(old_cap_local);
                self.wasm.active().local_get(lo_local);
                self.wasm.active().emit(&Instruction::I32Sub);
                self.wasm.active().i32_store(SLICE_CAP_OFFSET as u64);

                self.next_wasm_local = saved_next;
                self.wasm.active().local_get(new_hdr);

                Ok(slice_dt)
            }
            _ => Err(self.unsupported(&format!("expression: {:#?}", expr))),
        }
    }

    fn scan_free_vars_expr(&mut self, pkg: &str, expr: &Expression) {
        match expr {
            Expression::Ident(id) => { let _ = self.symbols.resolve(pkg, &id.name); }
            Expression::BasicLit(lit) if lit.kind == LitKind::Ident => {
                let _ = self.symbols.resolve(pkg, &lit.value);
            }
            Expression::Operation(op) => {
                self.scan_free_vars_expr(pkg, &op.x);
                if let Some(y) = &op.y { self.scan_free_vars_expr(pkg, y); }
            }
            Expression::Call(call) => {
                self.scan_free_vars_expr(pkg, &call.func);
                for a in &call.args { self.scan_free_vars_expr(pkg, a); }
            }
            Expression::Selector(sel) => {
                self.scan_free_vars_expr(pkg, &sel.x);
            }
            Expression::CompositeLit(cl) => {
                for kv in &cl.val.values {
                    if let Element::Expr(e) = &kv.val { self.scan_free_vars_expr(pkg, e); }
                }
            }
            Expression::FuncLit(fl) => {
                for stmt in &fl.body.list { self.scan_free_vars_stmt(pkg, stmt); }
            }
            Expression::Paren(inner) => { self.scan_free_vars_expr(pkg, &inner.expr); }
            Expression::Index(idx) => {
                self.scan_free_vars_expr(pkg, &idx.left);
                self.scan_free_vars_expr(pkg, &idx.index);
            }
            Expression::Slice(sl) => {
                self.scan_free_vars_expr(pkg, &sl.left);
                for opt in &sl.index {
                    if let Some(e) = opt { self.scan_free_vars_expr(pkg, e); }
                }
            }
            _ => {}
        }
    }

    fn scan_free_vars_stmt(&mut self, pkg: &str, stmt: &Statement) {
        match stmt {
            Statement::Expr(e) => { self.scan_free_vars_expr(pkg, &e.expr); }
            Statement::Return(r) => {
                for e in &r.ret { self.scan_free_vars_expr(pkg, e); }
            }
            Statement::Assign(a) => {
                for e in &a.left { self.scan_free_vars_expr(pkg, e); }
                for e in &a.right { self.scan_free_vars_expr(pkg, e); }
            }
            Statement::If(i) => {
                if let Some(init) = &i.init { self.scan_free_vars_stmt(pkg, init); }
                self.scan_free_vars_expr(pkg, &i.cond);
                for s in &i.body.list { self.scan_free_vars_stmt(pkg, s); }
                if let Some(els) = &i.else_ { self.scan_free_vars_stmt(pkg, els); }
            }
            Statement::For(f) => {
                if let Some(init) = &f.init { self.scan_free_vars_stmt(pkg, init); }
                if let Some(cond) = &f.cond { self.scan_free_vars_stmt(pkg, cond); }
                if let Some(post) = &f.post { self.scan_free_vars_stmt(pkg, post); }
                for s in &f.body.list { self.scan_free_vars_stmt(pkg, s); }
            }
            Statement::Block(b) => {
                for s in &b.list { self.scan_free_vars_stmt(pkg, s); }
            }
            Statement::IncDec(id) => { self.scan_free_vars_expr(pkg, &id.expr); }
            Statement::Range(r) => {
                self.scan_free_vars_expr(pkg, &r.expr);
                for s in &r.body.list { self.scan_free_vars_stmt(pkg, s); }
            }
            Statement::Declaration(decl_stmt) => {
                if let DeclStmt::Variable(vs) = decl_stmt {
                    for spec in &vs.specs {
                        for v in &spec.values { self.scan_free_vars_expr(pkg, v); }
                    }
                }
            }
            _ => {}
        }
    }

    fn compile_func_lit(
        &mut self,
        pkg: &str,
        fl: &FuncLit,
    ) -> Result<DefineType, Error> {
        let closure_name = format!("__closure_{}", self.next_closure_id);
        self.next_closure_id += 1;

        // Phase 1: Open a temporary closure context to discover captures.
        self.symbols.new_context(true);
        for p in &fl.typ.params.list {
            let t = self
                .expression_to_define_type(pkg, &p.typ)
                .ok_or_else(|| Error::TypeError("closure: failed to resolve param type".into()))?;
            for name in &p.name {
                self.symbols.define(
                    pkg,
                    &name.name,
                    DefineType::Qualified(Qualifier::Var, Box::new(t.clone())),
                    false,
                );
            }
        }
        for stmt in &fl.body.list {
            self.scan_free_vars_stmt(pkg, stmt);
        }
        let scan_ctx = self.symbols.leave_context();
        let captured_names: Vec<String> = scan_ctx.captured.clone();

        let mut captures: Vec<(String, DefineType)> = Vec::with_capacity(captured_names.len());
        for cap_name in &captured_names {
            let resolved = self.symbols.resolve(pkg, cap_name).ok_or_else(|| {
                Error::ReferenceError(format!("closure: captured var '{}' not found", cap_name))
            })?;
            captures.push((cap_name.clone(), resolved.get_type().0));
        }

        // Phase 2: Build WASM signature with declared params + capture params.
        let saved_locals = self.locals.clone();
        let saved_next_local = self.next_wasm_local;
        let saved_mem_vars = self.mem_vars.clone();
        let saved_frame_base = self.frame_base_local;
        let saved_escaped = self.escaped_vars.clone();
        self.locals.clear();
        self.next_wasm_local = 0;
        self.mem_vars.clear();
        self.frame_base_local = None;
        self.escaped_vars.clear();

        self.symbols.new_context(true);

        let mut wasm_params: Vec<ValType> = Vec::new();
        let mut decl_arg_types = Vec::new();

        for p in &fl.typ.params.list {
            let t = self
                .expression_to_define_type(pkg, &p.typ)
                .ok_or_else(|| Error::TypeError("closure: failed to resolve param type".into()))?;
            for name in &p.name {
                decl_arg_types.push(ContextType::Named(name.name.clone(), t.clone()));
                let sym = self.symbols.define(
                    pkg,
                    &name.name,
                    DefineType::Qualified(Qualifier::Var, Box::new(t.clone())),
                    false,
                );
                let local_idx = self.next_wasm_local;
                if Self::is_string_type(&t) {
                    self.next_wasm_local += 2;
                    self.locals.insert(sym.index, local_idx);
                    wasm_params.push(ValType::I32);
                    wasm_params.push(ValType::I32);
                    } else {
                    self.next_wasm_local += 1;
                    self.locals.insert(sym.index, local_idx);
                    wasm_params.push(Compiler::define_type_to_wasm(&t));
                }
            }
        }

        let capture_base = self.next_wasm_local;
        for (i, (cap_name, cap_dt)) in captures.iter().enumerate() {
            let sym = self.symbols.define(
                pkg,
                cap_name,
                DefineType::Qualified(Qualifier::Var, Box::new(cap_dt.clone())),
                false,
            );
            let local_idx = capture_base + i as u32;
            if Self::is_string_type(cap_dt) {
                self.next_wasm_local += 2;
                self.locals.insert(sym.index, local_idx);
                wasm_params.push(ValType::I32);
                wasm_params.push(ValType::I32);
                        } else {
                self.next_wasm_local += 1;
                self.locals.insert(sym.index, local_idx);
                wasm_params.push(Compiler::define_type_to_wasm(cap_dt));
            }
        }

        let mut decl_r_types = Vec::new();
        for el in &fl.typ.result.list {
            let t = self
                .expression_to_define_type(pkg, &el.typ)
                .ok_or_else(|| Error::TypeError("closure: failed to resolve return type".into()))?;
            decl_r_types.push(t);
        }

        let r_t = if decl_r_types.is_empty() {
            DefineType::Null
        } else if decl_r_types.len() == 1 {
            decl_r_types[0].clone()
                } else {
            DefineType::Tuple(decl_r_types.clone())
        };

        let wasm_results: Vec<ValType> = decl_r_types
            .iter()
            .map(|dt| Compiler::define_type_to_wasm(dt))
            .collect();

        let type_idx = self.wasm.add_func_type(wasm_params.clone(), wasm_results);
        let func_idx = self.wasm.define_function(type_idx);
        self.wasm_func_map.insert(closure_name.clone(), func_idx);
        self.closure_captures.insert(func_idx, captures.clone());

        let num_params = wasm_params.len() as u32;

        self.func_contexts.push(FuncContext::new(func_idx));
        self.func_contexts.last_mut().unwrap().expected_ret = if r_t == DefineType::Null {
            None
        } else {
            Some(r_t.clone())
        };

        let body_locals_count = 40_u32;
        self.wasm.begin_func_body(func_idx, vec![(body_locals_count, ValType::I32)]);
        self.next_wasm_local = num_params;

        // Save $sp for closures too.
        let sp_idx = self.wasm.sp_global_idx().expect("$sp global not registered");
        let closure_saved_sp = self.next_wasm_local;
        self.next_wasm_local += 1;
        let outer_saved_sp = self.saved_sp_local;
        self.saved_sp_local = Some(closure_saved_sp);
        self.wasm.active().global_get(sp_idx);
        self.wasm.active().local_set(closure_saved_sp);

        self.compile_block_statement(pkg, &fl.body.list)?;

        self.func_contexts.pop();

        // Restore $sp before closure end.
        self.wasm.active().local_get(closure_saved_sp);
        self.wasm.active().global_set(sp_idx);

        if !decl_r_types.is_empty() {
            self.wasm.active().emit(&Instruction::Unreachable);
        }

        self.wasm.end_func_body();
        self.symbols.leave_context();

        self.locals = saved_locals;
        self.next_wasm_local = saved_next_local;
        self.saved_sp_local = outer_saved_sp;
        self.mem_vars = saved_mem_vars;
        self.frame_base_local = saved_frame_base;
        self.escaped_vars = saved_escaped;

        let func_dt = DefineType::Func {
            name: closure_name,
            recv: None,
            args: decl_arg_types,
            rt: Box::new(r_t),
        };

        Ok(func_dt)
    }

    fn compile_call_expression(
        &mut self,
        pkg: &str,
        call: &Call,
    ) -> Result<DefineType, Error> {
        if let Expression::Ident(name) = call.func.as_ref() {
            match name.name.as_str() {
                "println" => return self.compile_println(pkg, call),
                "print" => return self.compile_print(pkg, call),
                "len" => return self.compile_len(pkg, call),
                "make" => return self.compile_make(pkg, call),
                "append" => return self.compile_append(pkg, call),
                _ => {}
            }
            if builtin::resolve(&name.name).is_some() {
                return Err(self.unsupported(&format!("builtin function: {}", name.name)));
            }

            if let Some(resolved) = self.symbols.resolve(pkg, &name.name) {
                let sym = resolved.get_symbol();
                if let Some(&closure_idx) = self.closure_var_func.get(&sym.index) {
                    let (_, _, _func_arg_types, rt) = resolved.get_type().0.unwrap_qualifiers().as_func();
                    let rts = rt.type_to_val_t();

                    for a in &call.args {
                        self.compile_expression(pkg, a)?;
                    }

                    if let Some(captures) = self.closure_captures.get(&closure_idx).cloned() {
                        for (cap_name, cap_dt) in &captures {
                            let cap_resolved = self.symbols.resolve(pkg, cap_name)
                                .ok_or_else(|| Error::ReferenceError(format!(
                                    "captured var '{}' not found at call site", cap_name
                                )))?;
                            let cap_sym = cap_resolved.get_symbol();
                            if let Some(&local_idx) = self.locals.get(&cap_sym.index) {
                                if Self::is_string_type(cap_dt) {
                                    self.wasm.active().local_get(local_idx);
                                    self.wasm.active().local_get(local_idx + 1);
                        } else {
                                    self.wasm.active().local_get(local_idx);
                                }
                            }
                        }
                    }

                    self.wasm.active().call(closure_idx);
                    return Ok(rts);
                }
            }
        }

        let (ct, _cpkg) = CallType::from_call(pkg, call, self)?;

        match ct {
            CallType::Func {
                name: f_name,
                func_dt,
                ..
            } => {
                let (_, _, arg_types, rts) = func_dt.as_func();
                let rts = rts.type_to_val_t();

                for (a, _t) in call.args.iter().zip(arg_types.iter()) {
                    self.compile_expression(pkg, a)?;
                }

                let wasm_idx = self.wasm_func_map.get(&f_name).copied().ok_or_else(|| {
                    Error::ReferenceError(format!("WASM function not found: {}", f_name))
                })?;

                self.wasm.active().call(wasm_idx);

                Ok(rts)
            }
            CallType::Method {
                mangled_name,
                method_dt,
                ..
            } => {
                let (_, _, arg_types, rt) = method_dt.as_func();
                let rts = rt.type_to_val_t();

                for (a, _t) in call.args.iter().zip(arg_types.iter()) {
                    self.compile_expression(pkg, a)?;
                }

                let wasm_idx = self.wasm_func_map.get(&mangled_name).copied().ok_or_else(|| {
                    Error::ReferenceError(format!("WASM function not found: {}", mangled_name))
                })?;

                self.wasm.active().call(wasm_idx);
                Ok(rts)
            }
            _ => Err(self.unsupported("dynamic dispatch calls")),
        }
    }

    fn compile_println(
        &mut self,
        pkg: &str,
        call: &Call,
    ) -> Result<DefineType, Error> {
        if call.args.is_empty() {
            let idx = self.wasm.println_string_func_idx()
                .ok_or_else(|| Error::InternalError("println_string not imported".into()))?;
            self.wasm.active().i32_const(0);
            self.wasm.active().i32_const(0);
            self.wasm.active().call(idx);
            return Ok(DefineType::Null);
        }
        if call.args.len() != 1 {
            return Err(self.unsupported("println with multiple arguments"));
        }
        let arg_type = self.compile_expression(pkg, &call.args[0])?;
        if !Self::is_string_type(&arg_type) {
            return Err(self.unsupported("println with non-string argument"));
        }
        let idx = self.wasm.println_string_func_idx()
            .ok_or_else(|| Error::InternalError("println_string not imported".into()))?;
        self.wasm.active().call(idx);
        Ok(DefineType::Null)
    }

    fn compile_print(
        &mut self,
        pkg: &str,
        call: &Call,
    ) -> Result<DefineType, Error> {
        if call.args.is_empty() {
            return Ok(DefineType::Null);
        }
        if call.args.len() != 1 {
            return Err(self.unsupported("print with multiple arguments"));
        }
        let arg_type = self.compile_expression(pkg, &call.args[0])?;
        if !Self::is_string_type(&arg_type) {
            return Err(self.unsupported("print with non-string argument"));
        }
        let idx = self.wasm.print_string_func_idx()
            .ok_or_else(|| Error::InternalError("print_string not imported".into()))?;
        self.wasm.active().call(idx);
        Ok(DefineType::Null)
    }

    fn compile_len(
        &mut self,
        pkg: &str,
        call: &Call,
    ) -> Result<DefineType, Error> {
        if call.args.len() != 1 {
            return Err(Error::TypeError("len() takes exactly 1 argument".into()));
        }
        let arg_type = self.compile_expression(pkg, &call.args[0])?;
        if Self::is_string_type(&arg_type) {
            // Stack has [ptr, len] (len on top). Save len, drop ptr, push len.
            let tmp = self.next_wasm_local;
            self.wasm.active().local_set(tmp);
            self.wasm.active().drop();
            self.wasm.active().local_get(tmp);
            Ok(DefineType::Int)
        } else if Self::is_slice_type(&arg_type) {
            // Stack has hdr_ptr. Load len from hdr+4.
            self.wasm.active().i32_load(SLICE_LEN_OFFSET as u64);
            Ok(DefineType::Int)
        } else if let Some((_elem_dt, arr_len)) = Self::unwrap_array_elem(&arg_type) {
            // Array pointer on stack -- drop it, push compile-time constant.
            self.wasm.active().drop();
            self.wasm.active().i32_const(arr_len as i32);
            Ok(DefineType::Int)
                        } else {
            Err(self.unsupported(&format!("len() with argument type {:?}", arg_type)))
        }
    }

    fn compile_make(
        &mut self,
        pkg: &str,
        call: &Call,
    ) -> Result<DefineType, Error> {
        // make([]T, length) or make([]T, length, cap)
        if call.args.len() < 2 || call.args.len() > 3 {
            return Err(Error::TypeError("make() takes 2 or 3 arguments".into()));
        }

        let type_expr = &call.args[0];
        let elem_dt = match type_expr {
            Expression::TypeSlice(ts) => {
                self.expression_to_define_type(pkg, &ts.typ)
                    .ok_or_else(|| Error::TypeError("make: cannot resolve slice element type".into()))?
            }
            _ => return Err(self.unsupported("make() with non-slice type")),
        };
        let e_size = elem_byte_size(&elem_dt);

        // Evaluate length
        self.compile_expression(pkg, &call.args[1])?;
        let len_local = self.next_wasm_local;
        self.wasm.active().local_set(len_local);

        let saved_next = self.next_wasm_local;
        self.next_wasm_local = len_local + 1;

        // Evaluate cap (default = length)
        let cap_local = self.next_wasm_local;
        self.next_wasm_local += 1;
        if call.args.len() == 3 {
            self.compile_expression(pkg, &call.args[2])?;
            self.wasm.active().local_set(cap_local);
                        } else {
            self.wasm.active().local_get(len_local);
            self.wasm.active().local_set(cap_local);
        }

        let rt_alloc_idx = self.wasm.rt_alloc_func_idx()
            .ok_or_else(|| Error::InternalError("rt_alloc not registered".into()))?;

        // Allocate data block: cap * e_size
        self.wasm.active().local_get(cap_local);
        self.wasm.active().i32_const(e_size as i32);
        self.wasm.active().emit(&Instruction::I32Mul);
        self.wasm.active().call(rt_alloc_idx);
        let data_ptr_local = self.next_wasm_local;
        self.next_wasm_local += 1;
        self.wasm.active().local_set(data_ptr_local);

        // Zero-fill data block: memory.fill(data_ptr, 0, cap * e_size)
        self.wasm.active().local_get(data_ptr_local);
        self.wasm.active().i32_const(0);
        self.wasm.active().local_get(cap_local);
        self.wasm.active().i32_const(e_size as i32);
        self.wasm.active().emit(&Instruction::I32Mul);
        self.wasm.active().memory_fill();

        // Allocate header
        self.wasm.active().i32_const(SLICE_HEADER_SIZE as i32);
        self.wasm.active().call(rt_alloc_idx);
        let hdr_local = self.next_wasm_local;
        self.wasm.active().local_set(hdr_local);

        // Store data_ptr, len, cap
        self.wasm.active().local_get(hdr_local);
        self.wasm.active().local_get(data_ptr_local);
        self.wasm.active().i32_store(SLICE_DATA_PTR_OFFSET as u64);

        self.wasm.active().local_get(hdr_local);
        self.wasm.active().local_get(len_local);
        self.wasm.active().i32_store(SLICE_LEN_OFFSET as u64);

        self.wasm.active().local_get(hdr_local);
        self.wasm.active().local_get(cap_local);
        self.wasm.active().i32_store(SLICE_CAP_OFFSET as u64);

        self.next_wasm_local = saved_next;
        self.wasm.active().local_get(hdr_local);

        Ok(DefineType::Slice(Box::new(elem_dt)))
    }

    fn compile_append(
        &mut self,
        pkg: &str,
        call: &Call,
    ) -> Result<DefineType, Error> {
        if call.args.len() < 2 {
            return Err(Error::TypeError("append() requires at least 2 arguments".into()));
        }

        // Compile slice argument
        let slice_dt = self.compile_expression(pkg, &call.args[0])?;
        let elem_dt = Self::unwrap_slice_elem(&slice_dt)
            .ok_or_else(|| Error::TypeError(format!("append on non-slice type {:?}", slice_dt)))?;
        let e_size = elem_byte_size(&elem_dt);
        let n_new = (call.args.len() - 1) as u32;

        let hdr_local = self.next_wasm_local;
        self.wasm.active().local_set(hdr_local);

        let saved_next = self.next_wasm_local;
        self.next_wasm_local = hdr_local + 1;

        // Load header fields
        let data_ptr_local = self.next_wasm_local;
        self.next_wasm_local += 1;
        let len_local = self.next_wasm_local;
        self.next_wasm_local += 1;
        let cap_local = self.next_wasm_local;
        self.next_wasm_local += 1;

        self.wasm.active().local_get(hdr_local);
        self.wasm.active().i32_load(SLICE_DATA_PTR_OFFSET as u64);
        self.wasm.active().local_set(data_ptr_local);

        self.wasm.active().local_get(hdr_local);
        self.wasm.active().i32_load(SLICE_LEN_OFFSET as u64);
        self.wasm.active().local_set(len_local);

        self.wasm.active().local_get(hdr_local);
        self.wasm.active().i32_load(SLICE_CAP_OFFSET as u64);
        self.wasm.active().local_set(cap_local);

        // Check if len + n_new > cap. If so, grow.
        // new_needed = len + n_new
        let needed_local = self.next_wasm_local;
        self.next_wasm_local += 1;
        self.wasm.active().local_get(len_local);
        self.wasm.active().i32_const(n_new as i32);
        self.wasm.active().emit(&Instruction::I32Add);
        self.wasm.active().local_set(needed_local);

        // if needed > cap -> grow
        self.wasm.active().local_get(needed_local);
        self.wasm.active().local_get(cap_local);
        self.wasm.active().emit(&Instruction::I32GtU);
        self.wasm.active().emit(&Instruction::If(BlockType::Empty));

        // new_cap = max(cap * 2, needed)
        let new_cap_local = self.next_wasm_local;
        self.next_wasm_local += 1;
        self.wasm.active().local_get(cap_local);
        self.wasm.active().i32_const(2);
        self.wasm.active().emit(&Instruction::I32Mul);
        self.wasm.active().local_set(new_cap_local);

        // if new_cap < needed { new_cap = needed }
        self.wasm.active().local_get(new_cap_local);
        self.wasm.active().local_get(needed_local);
        self.wasm.active().emit(&Instruction::I32LtU);
        self.wasm.active().emit(&Instruction::If(BlockType::Empty));
        self.wasm.active().local_get(needed_local);
        self.wasm.active().local_set(new_cap_local);
        self.wasm.active().emit(&Instruction::End);

        // Allocate new data block
        let rt_alloc_idx = self.wasm.rt_alloc_func_idx()
            .ok_or_else(|| Error::InternalError("rt_alloc not registered".into()))?;
        self.wasm.active().local_get(new_cap_local);
        self.wasm.active().i32_const(e_size as i32);
        self.wasm.active().emit(&Instruction::I32Mul);
        self.wasm.active().call(rt_alloc_idx);
        let new_data_local = self.next_wasm_local;
        self.next_wasm_local += 1;
        self.wasm.active().local_set(new_data_local);

        // memory.copy(new_data, old_data, len * e_size)
        self.wasm.active().local_get(new_data_local);
        self.wasm.active().local_get(data_ptr_local);
        self.wasm.active().local_get(len_local);
        self.wasm.active().i32_const(e_size as i32);
        self.wasm.active().emit(&Instruction::I32Mul);
        self.wasm.active().memory_copy();

        // Update header data_ptr and cap
        self.wasm.active().local_get(new_data_local);
        self.wasm.active().local_set(data_ptr_local);

        self.wasm.active().local_get(hdr_local);
        self.wasm.active().local_get(new_data_local);
        self.wasm.active().i32_store(SLICE_DATA_PTR_OFFSET as u64);

        self.wasm.active().local_get(new_cap_local);
        self.wasm.active().local_set(cap_local);

        self.wasm.active().local_get(hdr_local);
        self.wasm.active().local_get(new_cap_local);
        self.wasm.active().i32_store(SLICE_CAP_OFFSET as u64);

        self.wasm.active().emit(&Instruction::End); // end if grow

        // Store new elements at data_ptr + (len + i) * e_size
        for (i, arg) in call.args.iter().skip(1).enumerate() {
            // addr = data_ptr + (len + i) * e_size
            self.wasm.active().local_get(data_ptr_local);
            self.wasm.active().local_get(len_local);
            self.wasm.active().i32_const(i as i32);
            self.wasm.active().emit(&Instruction::I32Add);
            self.wasm.active().i32_const(e_size as i32);
            self.wasm.active().emit(&Instruction::I32Mul);
            self.wasm.active().emit(&Instruction::I32Add);
            let addr_local = self.next_wasm_local;
            self.wasm.active().local_set(addr_local);
            self.next_wasm_local = addr_local + 1;

            self.compile_expression(pkg, arg)?;
            self.emit_elem_store(addr_local, &elem_dt);

            self.next_wasm_local = addr_local + 1;
        }

        // Update len in header: len + n_new
        self.wasm.active().local_get(hdr_local);
        self.wasm.active().local_get(needed_local);
        self.wasm.active().i32_store(SLICE_LEN_OFFSET as u64);

        self.next_wasm_local = saved_next;
        self.wasm.active().local_get(hdr_local);

        Ok(slice_dt)
    }

    /// Emit the `__str_concat(ptr1, len1, ptr2, len2) -> (ptr, len)` helper once.
    /// Returns its function index.
    fn ensure_str_concat_func(&mut self) -> Result<u32, Error> {
        if let Some(idx) = self.str_concat_func_idx {
            return Ok(idx);
        }

        let rt_alloc_idx = self.wasm.rt_alloc_func_idx()
            .ok_or_else(|| Error::InternalError("rt_alloc not registered".into()))?;

        // Type: (i32, i32, i32, i32) -> (i32, i32)
        let type_idx = self.wasm.add_func_type(
            vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            vec![ValType::I32, ValType::I32],
        );
        let func_idx = self.wasm.define_function(type_idx);
        self.str_concat_func_idx = Some(func_idx);

        // Params: 0=ptr1, 1=len1, 2=ptr2, 3=len2
        // Locals: 4=new_ptr, 5=total_len
        self.wasm.begin_func_body(func_idx, vec![(2, ValType::I32)]);

        // total_len = len1 + len2
        self.wasm.active().local_get(1);
        self.wasm.active().local_get(3);
        self.wasm.active().emit(&Instruction::I32Add);
        self.wasm.active().local_set(5);

        // new_ptr = rt_alloc(total_len)
        self.wasm.active().local_get(5);
        self.wasm.active().call(rt_alloc_idx);
        self.wasm.active().local_set(4);

        // memory.copy(new_ptr, ptr1, len1) -- copy first string
        self.wasm.active().local_get(4);       // dst
        self.wasm.active().local_get(0);       // src
        self.wasm.active().local_get(1);       // len
        self.wasm.active().memory_copy();

        // memory.copy(new_ptr + len1, ptr2, len2) -- copy second string
        self.wasm.active().local_get(4);       // dst base
        self.wasm.active().local_get(1);       // + len1
        self.wasm.active().emit(&Instruction::I32Add);
        self.wasm.active().local_get(2);       // src
        self.wasm.active().local_get(3);       // len
        self.wasm.active().memory_copy();

        // Return (new_ptr, total_len)
        self.wasm.active().local_get(4);
        self.wasm.active().local_get(5);

        self.wasm.end_func_body();

        Ok(func_idx)
    }

    fn compile_operation_expression(
        &mut self,
        pkg: &str,
        op: &Operation,
    ) -> Result<DefineType, Error> {
        match op.op {
            Operator::Add
            | Operator::Sub
            | Operator::Star
            | Operator::Quo
            | Operator::Rem => match &op.y {
                Some(y) => {
                    let rt_left = self.compile_expression(pkg, op.x.as_ref())?;
                    let rt_right = self.compile_expression(pkg, y.as_ref())?;

                    if op.op == Operator::Add
                        && Self::is_string_type(&rt_left)
                        && Self::is_string_type(&rt_right)
                    {
                        // Stack: [ptr1, len1, ptr2, len2]
                        let concat_idx = self.ensure_str_concat_func()?;
                        self.wasm.active().call(concat_idx);
                        return Ok(DefineType::String);
                    }

                    let result_type = self.coerce_binary_operands_wasm(&rt_left, &rt_right)?;
                    let vt = Self::define_type_to_wasm(&result_type);

                    match (op.op, vt) {
                        (Operator::Add, ValType::I32) => {
                            self.wasm.active().emit(&Instruction::I32Add)
                        }
                        (Operator::Sub, ValType::I32) => {
                            self.wasm.active().emit(&Instruction::I32Sub)
                        }
                        (Operator::Star, ValType::I32) => {
                            self.wasm.active().emit(&Instruction::I32Mul)
                        }
                        (Operator::Quo, ValType::I32) => {
                            self.wasm.active().emit(&Instruction::I32DivS)
                        }
                        (Operator::Rem, ValType::I32) => {
                            self.wasm.active().emit(&Instruction::I32RemS)
                        }
                        (Operator::Add, ValType::I64) => {
                            self.wasm.active().emit(&Instruction::I64Add)
                        }
                        (Operator::Sub, ValType::I64) => {
                            self.wasm.active().emit(&Instruction::I64Sub)
                        }
                        (Operator::Star, ValType::I64) => {
                            self.wasm.active().emit(&Instruction::I64Mul)
                        }
                        (Operator::Quo, ValType::I64) => {
                            self.wasm.active().emit(&Instruction::I64DivS)
                        }
                        (Operator::Rem, ValType::I64) => {
                            self.wasm.active().emit(&Instruction::I64RemS)
                        }
                        (Operator::Add, ValType::F64) => {
                            self.wasm.active().emit(&Instruction::F64Add)
                        }
                        (Operator::Sub, ValType::F64) => {
                            self.wasm.active().emit(&Instruction::F64Sub)
                        }
                        (Operator::Star, ValType::F64) => {
                            self.wasm.active().emit(&Instruction::F64Mul)
                        }
                        (Operator::Quo, ValType::F64) => {
                            self.wasm.active().emit(&Instruction::F64Div)
                        }
                        (Operator::Add, ValType::F32) => {
                            self.wasm.active().emit(&Instruction::F32Add)
                        }
                        (Operator::Sub, ValType::F32) => {
                            self.wasm.active().emit(&Instruction::F32Sub)
                        }
                        (Operator::Star, ValType::F32) => {
                            self.wasm.active().emit(&Instruction::F32Mul)
                        }
                        (Operator::Quo, ValType::F32) => {
                            self.wasm.active().emit(&Instruction::F32Div)
                    }
                    _ => {
                            return Err(self
                                .unsupported(&format!("arithmetic op {:?} for {:?}", op.op, vt)))
                        }
                    }

                    if op.x.as_ref().is_int_lit() && op.y.as_ref().map_or(false, |y| y.is_int_lit())
                    {
                        Ok(DefineType::Qualified(
                            Qualifier::Const,
                            Box::new(result_type),
                        ))
            } else {
                        Ok(result_type)
                    }
                }
                None => {
                    if op.op == Operator::Sub {
                        let left = self.compile_expression(pkg, op.x.as_ref())?;
                        assert!(left.is_numeric());
                        self.wasm.active().emit(&Instruction::I32Const(-1));
                        self.wasm.active().emit(&Instruction::I32Mul);
                        return Ok(left);
                    }
                    if op.op == Operator::Star {
                        let ptr_dt = self.compile_expression(pkg, op.x.as_ref())?;
                        let inner = match ptr_dt.unwrap_qualifiers() {
                            DefineType::Ref(inner) => *inner,
                            other => return Err(Error::TypeError(
                                format!("cannot dereference non-pointer type {:?}", other)
                            )),
                        };
                        self.emit_nil_check();
                        self.wasm.active().i32_load(0);
                        return Ok(inner);
                    }
                    Err(self.unsupported(&format!("unary op {:?}", op.op)))
                }
            },
            Operator::Less
            | Operator::LessEqual
            | Operator::Greater
            | Operator::GreaterEqual
            | Operator::Equal
            | Operator::NotEqual => match &op.y {
                Some(y) => {
                    self.compile_expression(pkg, op.x.as_ref())?;
                    self.compile_expression(pkg, y.as_ref())?;

                    match op.op {
                        Operator::Less => self.wasm.active().emit(&Instruction::I32LtS),
                        Operator::LessEqual => {
                            self.wasm.active().emit(&Instruction::I32LeS)
                        }
                        Operator::Greater => {
                            self.wasm.active().emit(&Instruction::I32GtS)
                        }
                        Operator::GreaterEqual => {
                            self.wasm.active().emit(&Instruction::I32GeS)
                        }
                        Operator::Equal => self.wasm.active().emit(&Instruction::I32Eq),
                        Operator::NotEqual => {
                            self.wasm.active().emit(&Instruction::I32Ne)
                        }
                        _ => unreachable!(),
                    }

                    Ok(DefineType::Bool)
                }
                None => Err(self.unsupported("unary comparison")),
            },
            Operator::AndAnd => match &op.y {
                Some(y) => {
                    self.compile_expression(pkg, op.x.as_ref())?;
                    self.wasm.active().emit(&Instruction::If(BlockType::Result(ValType::I32)));
                    self.nesting_depth += 1;
                    self.compile_expression(pkg, y.as_ref())?;
                    self.wasm.active().emit(&Instruction::Else);
                    self.wasm.active().i32_const(0);
                    self.nesting_depth -= 1;
                    self.wasm.active().emit(&Instruction::End);
                    Ok(DefineType::Bool)
                }
                None => Err(self.unsupported("unary &&")),
            },
            Operator::OrOr => match &op.y {
                Some(y) => {
                    self.compile_expression(pkg, op.x.as_ref())?;
                    self.wasm.active().emit(&Instruction::If(BlockType::Result(ValType::I32)));
                    self.nesting_depth += 1;
                    self.wasm.active().i32_const(1);
                    self.wasm.active().emit(&Instruction::Else);
                    self.compile_expression(pkg, y.as_ref())?;
                    self.nesting_depth -= 1;
                    self.wasm.active().emit(&Instruction::End);
                    Ok(DefineType::Bool)
                }
                None => Err(self.unsupported("unary ||")),
            },
            Operator::Not => match &op.y {
                None => {
                    self.compile_expression(pkg, &op.x)?;
                    self.wasm.active().emit(&Instruction::I32Eqz);
                    Ok(DefineType::Bool)
                }
                Some(_) => Err(self.unsupported("binary !")),
            },
            Operator::And if op.y.is_none() => {
                if let Expression::Ident(id) = op.x.as_ref() {
                    let mv = self.mem_vars.get(&id.name).cloned()
                        .ok_or_else(|| Error::InternalError(
                            format!("&{}: variable not in linear memory", id.name)
                        ))?;
                    self.wasm.active().local_get(mv.addr_local);
                    let resolved_dt = self.symbols.resolve(pkg, &id.name)
                        .map(|r| r.get_type().0.unwrap_qualifiers())
                        .unwrap_or(DefineType::Int);
                    Ok(DefineType::Ref(Box::new(resolved_dt)))
                } else {
                    Err(self.unsupported("address-of non-identifier"))
                }
            }
            _ => Err(self.unsupported(&format!("operator: {:?}", op.op))),
        }
    }

    fn coerce_binary_operands_wasm(
        &self,
        left: &DefineType,
        right: &DefineType,
    ) -> Result<DefineType, Error> {
        let l = left.unwrap_qualifiers();
        let r = right.unwrap_qualifiers();
        if l == r {
            return Ok(l);
        }
        // If one is a const int and the other is typed, use the typed one
        if left.is_const_coerceable_to(right) {
            return Ok(r);
        }
        if right.is_const_coerceable_to(left) {
            return Ok(l);
        }
        if l.unwrap_to_base_type() == r.unwrap_to_base_type() {
            return Ok(r);
        }
        Err(Error::TypeError(format!(
            "mismatched types: {:#?} and {:#?}",
            left, right
        )))
    }

    pub(crate) fn make_type_default_val(&mut self, t: DefineType) -> Expression {
        match t {
            DefineType::Type(inner, _) => self.make_type_default_val(*inner),
            DefineType::Int
            | DefineType::Int8
            | DefineType::Int16
            | DefineType::Int32
            | DefineType::Int64
            | DefineType::Uint
            | DefineType::Uint8
            | DefineType::Uint16
            | DefineType::Uint32
            | DefineType::Uint64
            | DefineType::Byte => Expression::BasicLit(BasicLit {
                pos: 0,
                kind: LitKind::Integer,
                value: "0".to_string(),
            }),
            DefineType::Bool => Expression::Ident(Ident {
                pos: 0,
                name: "false".to_string(),
            }),
            DefineType::Float32 | DefineType::Float64 => Expression::BasicLit(BasicLit {
                pos: 0,
                kind: LitKind::Float,
                value: "0.0".to_string(),
            }),
            DefineType::Qualified(_, inner) => self.make_type_default_val(*inner),
            DefineType::Ref(_) => Expression::Ident(Ident {
                pos: 0,
                name: "nil".to_string(),
            }),
            _ => Expression::BasicLit(BasicLit {
                pos: 0,
                kind: LitKind::Integer,
                value: "0".to_string(),
            }),
        }
    }
}
