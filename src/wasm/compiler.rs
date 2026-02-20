use crate::parser::ast;
use crate::parser::token::{Keyword, LitKind, Operator};
use crate::symbols::{Error, SymbolTable};
use crate::wasm::types::WasmType;
use crate::wasm::udf::{
    AggregateDescriptor, FieldDescriptor, FunctionDescriptor, Manifest, OutputDescriptor,
    TableDescriptor,
};
use std::collections::HashMap;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, ElementSection, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction, MemArg, MemorySection,
    MemoryType, Module, TableSection, TypeSection, ValType,
};

pub struct CompileResult {
    pub wasm_bytes: Vec<u8>,
    pub manifest: Manifest,
}

#[allow(dead_code)]
#[derive(Clone)]
struct FuncInfo {
    wasm_func_idx: u32,
    type_idx: u32,
    name: String,
    params: Vec<(String, WasmType)>,
    results: Vec<WasmType>,
    result_go_types: Vec<String>,
    is_exported: bool,
    recv_type: Option<String>,
    is_variadic: bool,
    variadic_elem_vt: Option<ValType>,
    iface_param_indices: Vec<usize>,
}

struct DeferredCall {
    func_idx: u32,
    arg_locals: Vec<(u32, ValType)>,
    named_return_captures: Vec<NamedReturnCapture>,
}

struct NamedReturnCapture {
    env_local: u32,
    env_offset: u32,
    named_return_local: u32,
    val_type: ValType,
}

#[derive(Debug, Clone)]
struct StructDef {
    fields: Vec<StructFieldDef>,
    total_size: u32,
    embedded_types: Vec<(String, u32)>, // (type_name, offset_in_parent)
}

#[derive(Debug, Clone)]
struct StructFieldDef {
    name: String,
    wasm_type: WasmType,
    offset: u32,
    go_type_tag: Option<String>,
}

#[derive(Debug, Clone)]
enum ConstValue {
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(String),
    Complex128(f64, f64),
}

impl StructDef {
    fn find_field(&self, name: &str) -> Option<&StructFieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }
}

#[derive(Clone)]
struct CapturedVar {
    name: String,
    val_type: ValType,
    outer_local_idx: u32,
    env_offset: u32,
}

fn val_type_byte_size(vt: ValType) -> u32 {
    match vt {
        ValType::I64 | ValType::F64 => 8,
        _ => 4,
    }
}

fn aligned_capture_env_offset(captures: &[CapturedVar], next_vt: ValType) -> u32 {
    let mut offset: u32 = 0;
    for c in captures {
        let size = val_type_byte_size(c.val_type);
        let align = size;
        offset = (offset + align - 1) & !(align - 1);
        offset += size;
    }
    let next_align = val_type_byte_size(next_vt);
    (offset + next_align - 1) & !(next_align - 1)
}

struct ClosureCaptureState {
    outer_locals: Vec<(String, ValType)>,
    captures: Vec<CapturedVar>,
    outer_closure_info: HashMap<String, (u32, u32)>,
    outer_closure_env_captures: HashMap<String, Vec<(String, u32, ValType)>>,
}

#[derive(Clone)]
struct MapTypeInfo {
    key_vt: ValType,
    val_vt: ValType,
    key_size: u32,
    val_size: u32,
    is_string_key: bool,
    is_string_val: bool,
    val_struct_type: Option<String>,
    nested_map_val_type: Option<Box<MapTypeInfo>>,
}

struct LocalAlloc {
    params: Vec<(String, ValType)>,
    locals: Vec<(String, ValType, u32)>,
    scope_depth: u32,
    next_scope_id: u32,
    scope_stack: Vec<u32>,
    var_types: HashMap<String, String>,
    closure_info: HashMap<String, (u32, u32)>,
    closure_env_captures: HashMap<String, Vec<(String, u32, ValType)>>,
    method_expr_vars: std::collections::HashSet<String>,
    slice_elem_types: HashMap<String, ValType>,
    slice_elem_struct_types: HashMap<String, String>,
    nested_slice_inner_elem_types: HashMap<String, ValType>,
    string_locals: HashMap<String, (u32, u32)>,
    unsigned_vars: std::collections::HashSet<String>,
    map_types: HashMap<String, MapTypeInfo>,
    array_info: HashMap<String, (ValType, u32)>, // (elem_type, array_length)
    nested_array_inner_info: HashMap<String, (ValType, u32)>, // inner (elem_type, inner_length) for [M][N]T
    rune_slices: std::collections::HashSet<String>,
}

impl LocalAlloc {
    fn new(params: Vec<(String, ValType)>) -> Self {
        Self {
            params,
            locals: Vec::new(),
            scope_depth: 0,
            next_scope_id: 1,
            scope_stack: vec![0],
            var_types: HashMap::new(),
            closure_info: HashMap::new(),
            closure_env_captures: HashMap::new(),
            method_expr_vars: std::collections::HashSet::new(),
            slice_elem_types: HashMap::new(),
            slice_elem_struct_types: HashMap::new(),
            nested_slice_inner_elem_types: HashMap::new(),
            string_locals: HashMap::new(),
            unsigned_vars: std::collections::HashSet::new(),
            map_types: HashMap::new(),
            array_info: HashMap::new(),
            nested_array_inner_info: HashMap::new(),
            rune_slices: std::collections::HashSet::new(),
        }
    }

    fn param_count(&self) -> u32 {
        self.params.len() as u32
    }

    fn current_scope_id(&self) -> u32 {
        *self.scope_stack.last().expect("scope_stack must never be empty")
    }

    fn push_scope(&mut self) {
        self.scope_depth += 1;
        let id = self.next_scope_id;
        self.next_scope_id += 1;
        self.scope_stack.push(id);
    }

    fn pop_scope(&mut self) {
        self.scope_depth = self.scope_depth.saturating_sub(1);
        if self.scope_stack.len() > 1 {
            self.scope_stack.pop();
        }
    }

    fn add_local(&mut self, name: &str, vt: ValType) -> u32 {
        let idx = self.param_count() + self.locals.len() as u32;
        self.locals.push((name.to_string(), vt, self.current_scope_id()));
        idx
    }

    fn find(&self, name: &str) -> Option<u32> {
        for (i, (n, _)) in self.params.iter().enumerate() {
            if n == name {
                return Some(i as u32);
            }
        }
        for (i, (n, _, scope_id)) in self.locals.iter().enumerate().rev() {
            if n == name && self.scope_stack.contains(scope_id) {
                return Some(self.param_count() + i as u32);
            }
        }
        None
    }

    fn find_val_type(&self, name: &str) -> Option<ValType> {
        for (n, vt) in &self.params {
            if n == name {
                return Some(*vt);
            }
        }
        for (n, vt, scope_id) in self.locals.iter().rev() {
            if n == name && self.scope_stack.contains(scope_id) {
                return Some(*vt);
            }
        }
        None
    }

    fn find_at_current_scope(&self, name: &str) -> Option<u32> {
        let current = self.current_scope_id();
        for (i, (n, _, scope_id)) in self.locals.iter().enumerate().rev() {
            if n == name && *scope_id == current {
                return Some(self.param_count() + i as u32);
            }
            if n == name && self.scope_stack.contains(scope_id) && *scope_id != current {
                return None;
            }
        }
        None
    }

    fn find_type(&self, name: &str) -> Option<ValType> {
        for (n, vt) in &self.params {
            if n == name {
                return Some(*vt);
            }
        }
        for (n, vt, scope_id) in self.locals.iter().rev() {
            if n == name && self.scope_stack.contains(scope_id) {
                return Some(*vt);
            }
        }
        None
    }

    fn local_types(&self) -> Vec<(u32, ValType)> {
        self.locals.iter().map(|(_, vt, _)| (1, *vt)).collect()
    }

    fn set_var_struct_type(&mut self, name: &str, type_name: &str) {
        self.var_types.insert(name.to_string(), type_name.to_string());
    }

    fn get_var_struct_type(&self, name: &str) -> Option<&str> {
        self.var_types.get(name).map(|s| s.as_str())
    }

    fn all_entries(&self) -> Vec<(String, ValType)> {
        self.params
            .iter()
            .cloned()
            .chain(self.locals.iter().map(|(n, vt, _)| (n.clone(), *vt)))
            .collect()
    }
}

pub struct WasmCompiler {
    pub symbols: SymbolTable,
    type_section: TypeSection,
    function_section: FunctionSection,
    memory_section: MemorySection,
    global_section: GlobalSection,
    export_section: ExportSection,
    import_section: ImportSection,
    code_section: CodeSection,
    table_section: TableSection,
    element_section: ElementSection,

    next_type_idx: u32,
    next_func_idx: u32,
    next_global_idx: u32,
    import_func_count: u32,
    heap_ptr_global: u32,
    panicking_global: u32,
    panic_value_ptr_global: u32,
    panic_value_len_global: u32,
    map_iter_counter_global: u32,
    oom_func_idx: u32,

    functions: Vec<FuncInfo>,
    deferred_calls: Vec<Vec<DeferredCall>>,
    loop_depth: Vec<(Option<String>, u32, bool, bool)>,
    manifest: Manifest,
    struct_defs: HashMap<String, StructDef>,
    closure_captures: Option<ClosureCaptureState>,
    last_closure_func_idx: Option<u32>,
    last_closure_env: Option<u32>,
    last_closure_captures: Vec<CapturedVar>,
    last_func_value_idx: Option<u32>,
    last_is_method_expr: bool,
    pending_closures: Vec<Function>,
    constants: HashMap<String, ConstValue>,
    global_vars: HashMap<String, (u32, ValType)>,
    current_iota: Option<i64>,
    named_returns: Vec<(String, ValType)>,
    current_result_types: Vec<ValType>,
    current_result_go_types: Vec<String>,

    // Named type definitions: type MyInt int → "MyInt" → ("int", is_alias)
    type_aliases: HashMap<String, (String, bool)>,

    // Named composite types: type MySlice []int → "MySlice" → TypeSlice(int)
    named_composite_types: HashMap<String, ast::Expression>,

    // Interface support
    type_registry: HashMap<String, u32>,
    next_type_id: u32,
    iface_defs: HashMap<String, Vec<String>>,
    iface_method_sigs: HashMap<String, HashMap<String, (Vec<WasmType>, Vec<WasmType>)>>,
    iface_var_type_ids: HashMap<String, u32>,

    // init() function support
    init_func_indices: Vec<u32>,
    start_func_idx: Option<u32>,

    // Deferred global variable initializers (non-constant or string expressions)
    global_var_inits: Vec<(String, ast::Expression, ValType)>,

    // Global variables that hold function values: var_name -> wasm_func_idx
    global_func_vars: HashMap<String, u32>,

    // Generics support: store uncompiled generic function declarations
    generic_funcs: HashMap<String, ast::FuncDecl>,
    // Generic type definitions: type Pair[T any] struct { ... }
    generic_types: HashMap<String, ast::TypeSpec>,
    // Track monomorphized specializations: "FuncName<int,string>" -> func_idx
    monomorphized: HashMap<String, u32>,

    // Map type info stored per struct field: ("StructName", "fieldName") -> MapTypeInfo
    struct_field_map_types: HashMap<(String, String), MapTypeInfo>,
}

impl WasmCompiler {
    pub fn new() -> Self {
        Self {
            symbols: SymbolTable::new(),
            type_section: TypeSection::new(),
            function_section: FunctionSection::new(),
            memory_section: MemorySection::new(),
            global_section: GlobalSection::new(),
            export_section: ExportSection::new(),
            import_section: ImportSection::new(),
            code_section: CodeSection::new(),
            table_section: TableSection::new(),
            element_section: ElementSection::new(),

            next_type_idx: 0,
            next_func_idx: 0,
            next_global_idx: 0,
            import_func_count: 0,
            heap_ptr_global: 0,
            panicking_global: 0,
            panic_value_ptr_global: 0,
            panic_value_len_global: 0,
            map_iter_counter_global: 0,
            oom_func_idx: 0,

            functions: Vec::new(),
            deferred_calls: Vec::new(),
            loop_depth: Vec::new(),
            manifest: Manifest::new(),
            struct_defs: HashMap::new(),
            closure_captures: None,
            last_closure_func_idx: None,
            last_closure_env: None,
            last_closure_captures: Vec::new(),
            last_func_value_idx: None,
            last_is_method_expr: false,
            pending_closures: Vec::new(),
            constants: HashMap::new(),
            global_vars: HashMap::new(),
            current_iota: None,
            named_returns: Vec::new(),
            current_result_types: Vec::new(),
            current_result_go_types: Vec::new(),

            type_aliases: HashMap::new(),
            named_composite_types: HashMap::new(),

            type_registry: HashMap::new(),
            next_type_id: 1, // 0 = nil
            iface_defs: HashMap::new(),
            iface_method_sigs: HashMap::new(),
            iface_var_type_ids: HashMap::new(),

            init_func_indices: Vec::new(),
            start_func_idx: None,

            global_var_inits: Vec::new(),
            global_func_vars: HashMap::new(),

            generic_funcs: HashMap::new(),
            generic_types: HashMap::new(),
            monomorphized: HashMap::new(),
            struct_field_map_types: HashMap::new(),
        }
    }

    fn get_or_create_type_id(&mut self, type_name: &str) -> u32 {
        if let Some(&id) = self.type_registry.get(type_name) {
            return id;
        }
        let id = self.next_type_id;
        self.next_type_id += 1;
        self.type_registry.insert(type_name.to_string(), id);
        id
    }

    fn register_builtin_types(&mut self) {
        self.get_or_create_type_id("int");
        self.get_or_create_type_id("int8");
        self.get_or_create_type_id("int16");
        self.get_or_create_type_id("int32");
        self.get_or_create_type_id("int64");
        self.get_or_create_type_id("uint");
        self.get_or_create_type_id("uint8");
        self.get_or_create_type_id("uint16");
        self.get_or_create_type_id("uint32");
        self.get_or_create_type_id("uint64");
        self.get_or_create_type_id("float32");
        self.get_or_create_type_id("float64");
        self.get_or_create_type_id("bool");
        self.get_or_create_type_id("string");
        self.get_or_create_type_id("byte");
        self.get_or_create_type_id("rune");

        self.iface_defs.insert("error".to_string(), vec!["Error".to_string()]);
        let mut error_sigs = HashMap::new();
        error_sigs.insert("Error".to_string(), (vec![], vec![WasmType::I32, WasmType::I32]));
        self.iface_method_sigs.insert("error".to_string(), error_sigs);
    }

    fn type_id_for_val_type(vt: ValType) -> &'static str {
        match vt {
            ValType::I32 => "int32",
            ValType::I64 => "int",
            ValType::F32 => "float32",
            ValType::F64 => "float64",
            _ => "int",
        }
    }

    fn val_type_for_type_name(name: &str) -> ValType {
        match name {
            "int" | "int64" | "uint" | "uint64" => ValType::I64,
            "int32" | "uint32" | "int16" | "uint16" | "int8" | "uint8"
            | "byte" | "rune" | "bool" | "uintptr" => ValType::I32,
            "float32" => ValType::F32,
            "float64" => ValType::F64,
            _ => ValType::I32, // structs are pointers
        }
    }

    fn parse_go_int(s: &str) -> Result<i64, String> {
        let s = s.replace('_', "");
        if let Some(digits) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
            i64::from_str_radix(digits, 2)
                .map_err(|e| format!("invalid binary literal '{}': {}", s, e))
        } else if let Some(digits) = s.strip_prefix("0o").or_else(|| s.strip_prefix("0O")) {
            i64::from_str_radix(digits, 8)
                .map_err(|e| format!("invalid octal literal '{}': {}", s, e))
        } else if let Some(digits) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            i64::from_str_radix(digits, 16)
                .map_err(|e| format!("invalid hex literal '{}': {}", s, e))
        } else if s.starts_with('0') && s.len() > 1 && s[1..].chars().all(|c| c.is_ascii_digit()) {
            // Legacy octal: 0600, 0777, etc. (leading zero without o/O prefix)
            i64::from_str_radix(&s[1..], 8)
                .map_err(|e| format!("invalid octal literal '{}': {}", s, e))
        } else {
            s.parse::<i64>()
                .map_err(|e| format!("invalid integer literal '{}': {}", s, e))
        }
    }

    fn parse_go_float(s: &str) -> Result<f64, String> {
        let s = s.replace('_', "");
        if s.starts_with("0x") || s.starts_with("0X") {
            Self::parse_hex_float(&s)
        } else {
            s.parse::<f64>()
                .map_err(|e| format!("invalid float literal '{}': {}", s, e))
        }
    }

    fn parse_hex_float(s: &str) -> Result<f64, String> {
        let s = if s.starts_with("0x") || s.starts_with("0X") { &s[2..] } else { s };
        let (mantissa_str, exp_str) = if let Some(pos) = s.find(|c: char| c == 'p' || c == 'P') {
            (&s[..pos], &s[pos + 1..])
        } else {
            return Err(format!("hex float missing exponent: 0x{}", s));
        };

        let (int_part, frac_part) = if let Some(dot_pos) = mantissa_str.find('.') {
            (&mantissa_str[..dot_pos], &mantissa_str[dot_pos + 1..])
        } else {
            (mantissa_str, "")
        };

        let int_val = if int_part.is_empty() {
            0u64
        } else {
            u64::from_str_radix(int_part, 16)
                .map_err(|e| format!("invalid hex mantissa '{}': {}", int_part, e))?
        };

        let mut frac_val: f64 = 0.0;
        let mut frac_scale: f64 = 1.0 / 16.0;
        for ch in frac_part.chars() {
            let digit = ch.to_digit(16)
                .ok_or_else(|| format!("invalid hex digit '{}' in mantissa", ch))?;
            frac_val += digit as f64 * frac_scale;
            frac_scale /= 16.0;
        }

        let mantissa = int_val as f64 + frac_val;

        let exp: i32 = exp_str.parse()
            .map_err(|e| format!("invalid exponent '{}': {}", exp_str, e))?;

        Ok(mantissa * (2.0f64).powi(exp))
    }

    pub fn compile_source(&mut self, source: &str) -> Result<CompileResult, Error> {
        let file = crate::parser::parse_source(source)
            .map_err(|e| Error::SyntaxError(e.to_string()))?;
        self.compile_file(&file)
    }

    pub fn compile_file(&mut self, file: &ast::File) -> Result<CompileResult, Error> {
        self.emit_memory();
        self.emit_heap_globals();
        self.emit_host_imports();
        self.emit_alloc_function();
        self.emit_reset_function();
        self.register_builtin_types();
        self.prescan_type_declarations(file);

        let sorted_decls = Self::sort_declarations_by_deps(&file.decl);
        for decl in &sorted_decls {
            self.compile_declaration(decl)?;
        }

        self.emit_global_var_init_function()?;
        self.emit_init_function();

        self.build_manifest(file)?;

        Ok(CompileResult {
            wasm_bytes: self.build_module(),
            manifest: self.manifest.clone(),
        })
    }

    fn emit_memory(&mut self) {
        self.memory_section.memory(MemoryType {
            minimum: 1,
            maximum: Some(256),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        self.export_section.export("memory", ExportKind::Memory, 0);
    }

    fn emit_heap_globals(&mut self) {
        self.heap_ptr_global = self.next_global_idx;
        self.global_section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(1024),
        );
        self.next_global_idx += 1;

        self.panicking_global = self.next_global_idx;
        self.global_section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        self.next_global_idx += 1;

        self.panic_value_ptr_global = self.next_global_idx;
        self.global_section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        self.next_global_idx += 1;

        self.panic_value_len_global = self.next_global_idx;
        self.global_section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        self.next_global_idx += 1;

        self.map_iter_counter_global = self.next_global_idx;
        self.global_section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        self.next_global_idx += 1;
    }

    fn emit_host_imports(&mut self) {
        let pairs: &[(&str, &[ValType], &[ValType])] = &[
            ("ctx_log", &[ValType::I32, ValType::I32], &[]),
            ("ctx_query_id", &[ValType::I32], &[ValType::I32]),
            ("ctx_database", &[ValType::I32], &[ValType::I32]),
            ("ctx_schema", &[ValType::I32], &[ValType::I32]),
            ("ctx_user", &[ValType::I32], &[ValType::I32]),
            (
                "ctx_config",
                &[ValType::I32, ValType::I32, ValType::I32],
                &[ValType::I32],
            ),
            ("ctx_oom", &[], &[]),
        ];

        for (name, params, results) in pairs {
            let type_idx = self.next_type_idx;
            self.type_section.ty().function(
                params.iter().copied().collect::<Vec<_>>(),
                results.iter().copied().collect::<Vec<_>>(),
            );
            self.next_type_idx += 1;

            if *name == "ctx_oom" {
                self.oom_func_idx = self.next_func_idx;
            }

            self.import_section.import(
                "env",
                *name,
                wasm_encoder::EntityType::Function(type_idx),
            );
            self.next_func_idx += 1;
            self.import_func_count += 1;
        }
    }

    fn emit_alloc_function(&mut self) {
        let type_idx = self.next_type_idx;
        self.type_section
            .ty()
            .function(vec![ValType::I32], vec![ValType::I32]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let mut func = Function::new(vec![(1, ValType::I32)]);

        // Align size to 8: size = (size + 7) & ~7
        func.instruction(&Instruction::LocalGet(0));
        func.instruction(&Instruction::I32Const(7));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::I32Const(!7));
        func.instruction(&Instruction::I32And);
        func.instruction(&Instruction::LocalSet(0));

        // Save current pointer
        func.instruction(&Instruction::GlobalGet(self.heap_ptr_global));
        func.instruction(&Instruction::LocalSet(1));

        // Bump heap pointer
        func.instruction(&Instruction::GlobalGet(self.heap_ptr_global));
        func.instruction(&Instruction::LocalGet(0));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::GlobalSet(self.heap_ptr_global));

        // Overflow check: if new_ptr < old_ptr, the add wrapped around
        func.instruction(&Instruction::GlobalGet(self.heap_ptr_global));
        func.instruction(&Instruction::LocalGet(1));
        func.instruction(&Instruction::I32LtU);
        func.instruction(&Instruction::If(BlockType::Empty));
        func.instruction(&Instruction::Call(self.oom_func_idx));
        func.instruction(&Instruction::Unreachable);
        func.instruction(&Instruction::End);

        // Bounds check: if new heap_ptr >= memory size in bytes, grow
        func.instruction(&Instruction::GlobalGet(self.heap_ptr_global));
        func.instruction(&Instruction::MemorySize(0));
        func.instruction(&Instruction::I32Const(16));
        func.instruction(&Instruction::I32Shl);
        func.instruction(&Instruction::I32GeU);
        func.instruction(&Instruction::If(BlockType::Empty));
        // Compute pages needed: (heap_ptr - current_bytes + 65535) >> 16
        func.instruction(&Instruction::GlobalGet(self.heap_ptr_global));
        func.instruction(&Instruction::MemorySize(0));
        func.instruction(&Instruction::I32Const(16));
        func.instruction(&Instruction::I32Shl);
        func.instruction(&Instruction::I32Sub);
        func.instruction(&Instruction::I32Const(65535));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::I32Const(16));
        func.instruction(&Instruction::I32ShrU);
        func.instruction(&Instruction::MemoryGrow(0));
        func.instruction(&Instruction::I32Const(-1));
        func.instruction(&Instruction::I32Eq);
        func.instruction(&Instruction::If(BlockType::Empty));
        func.instruction(&Instruction::Call(self.oom_func_idx));
        func.instruction(&Instruction::Unreachable);
        func.instruction(&Instruction::End);
        func.instruction(&Instruction::End);

        // Return old pointer
        func.instruction(&Instruction::LocalGet(1));
        func.instruction(&Instruction::End);

        self.code_section.function(&func);
        self.export_section
            .export("alloc", ExportKind::Func, func_idx);

        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: "alloc".to_string(),
            params: vec![("size".to_string(), WasmType::I32)],
            results: vec![WasmType::I32],
            result_go_types: vec![],
            is_exported: true,
            recv_type: None,
            is_variadic: false,
            variadic_elem_vt: None,
            iface_param_indices: vec![],
        });
    }

    fn emit_reset_function(&mut self) {
        let type_idx = self.next_type_idx;
        self.type_section.ty().function(vec![], vec![]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let mut func = Function::new(vec![]);
        func.instruction(&Instruction::I32Const(1024));
        func.instruction(&Instruction::GlobalSet(self.heap_ptr_global));
        func.instruction(&Instruction::End);

        self.code_section.function(&func);
        self.export_section
            .export("reset", ExportKind::Func, func_idx);

        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: "reset".to_string(),
            params: vec![],
            results: vec![],
            result_go_types: vec![],
            is_exported: true,
            recv_type: None,
            is_variadic: false,
            variadic_elem_vt: None,
            iface_param_indices: vec![],
        });
    }

    fn emit_global_var_init_function(&mut self) -> Result<(), Error> {
        let inits = std::mem::take(&mut self.global_var_inits);
        if inits.is_empty() {
            return Ok(());
        }

        let type_idx = self.next_type_idx;
        self.type_section.ty().function(vec![], vec![]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let param_entries: Vec<(String, ValType)> = Vec::new();
        let mut locals = LocalAlloc::new(param_entries);
        let mut body: Vec<Instruction<'static>> = Vec::new();

        for (var_name, init_expr, _vt) in &inits {
            let is_string_global = self.global_vars.contains_key(&format!("{}_1", var_name));

            if is_string_global {
                self.compile_expression(init_expr, &mut body, &mut locals)?;
                let len_tmp = locals.add_local(
                    &format!("__ginit_len_{}", locals.locals.len()),
                    ValType::I32,
                );
                let ptr_tmp = locals.add_local(
                    &format!("__ginit_ptr_{}", locals.locals.len()),
                    ValType::I32,
                );
                body.push(Instruction::LocalSet(len_tmp));
                body.push(Instruction::LocalSet(ptr_tmp));

                let (ptr_global, _) = self.global_vars[var_name];
                let (len_global, _) = self.global_vars[&format!("{}_1", var_name)];
                body.push(Instruction::LocalGet(ptr_tmp));
                body.push(Instruction::GlobalSet(ptr_global));
                body.push(Instruction::LocalGet(len_tmp));
                body.push(Instruction::GlobalSet(len_global));
            } else {
                self.compile_expression(init_expr, &mut body, &mut locals)?;
                let (global_idx, _) = self.global_vars[var_name];
                body.push(Instruction::GlobalSet(global_idx));
            }
        }

        body.push(Instruction::End);

        let mut func = Function::new(locals.local_types());
        for instr in &body {
            func.instruction(instr);
        }
        self.code_section.function(&func);

        // Insert before user init() functions
        self.init_func_indices.insert(0, func_idx);

        Ok(())
    }

    fn emit_init_function(&mut self) {
        if self.init_func_indices.is_empty() {
            return;
        }

        let type_idx = self.next_type_idx;
        self.type_section.ty().function(vec![], vec![]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let mut func = Function::new(vec![]);
        for &init_idx in &self.init_func_indices {
            func.instruction(&Instruction::Call(init_idx));
        }
        func.instruction(&Instruction::End);

        self.code_section.function(&func);
        self.start_func_idx = Some(func_idx);
    }

    fn alloc_func_idx(&self) -> Result<u32, Error> {
        self.functions
            .iter()
            .find(|f| f.name == "alloc")
            .map(|f| f.wasm_func_idx)
            .ok_or_else(|| {
                Error::InternalError(
                    "alloc function not registered; emit_alloc_function must be called before compilation".to_string(),
                )
            })
    }

    fn prescan_type_declarations(&mut self, file: &ast::File) {
        for decl in &file.decl {
            if let ast::Declaration::Type(type_decl) = decl {
                for spec in &type_decl.specs {
                    let name = &spec.name.name;
                    if let ast::Expression::TypeInterface(iface) = &spec.typ {
                        let mut methods = Vec::new();
                        let mut embedded_ifaces = Vec::new();
                        let mut method_sigs: HashMap<String, (Vec<WasmType>, Vec<WasmType>)> = HashMap::new();
                        for field in &iface.methods.list {
                            if field.name.is_empty() {
                                if let ast::Expression::Ident(embedded_id) = &field.typ {
                                    embedded_ifaces.push(embedded_id.name.clone());
                                }
                            } else {
                                if let ast::Expression::TypeFunction(ft) = &field.typ {
                                    let param_types: Vec<WasmType> = ft.params.list.iter()
                                        .flat_map(|p| self.field_to_wasm_types(p))
                                        .collect();
                                    let result_types: Vec<WasmType> = ft.result.list.iter()
                                        .flat_map(|r| self.field_to_wasm_types(r))
                                        .collect();
                                    for ident in &field.name {
                                        methods.push(ident.name.clone());
                                        method_sigs.insert(ident.name.clone(), (param_types.clone(), result_types.clone()));
                                    }
                                } else {
                                    for ident in &field.name {
                                        methods.push(ident.name.clone());
                                    }
                                }
                            }
                        }
                        for embedded_name in &embedded_ifaces {
                            if let Some(embedded_methods) = self.iface_defs.get(embedded_name) {
                                methods.extend(embedded_methods.clone());
                            }
                            if let Some(embedded_sigs) = self.iface_method_sigs.get(embedded_name) {
                                method_sigs.extend(embedded_sigs.clone());
                            }
                        }
                        self.iface_defs.insert(name.clone(), methods);
                        self.iface_method_sigs.insert(name.clone(), method_sigs);
                    } else if let ast::Expression::TypeStruct(_) = &spec.typ {
                        self.get_or_create_type_id(name);
                    }
                }
            }
            // Register methods from function declarations
            if let ast::Declaration::Function(func_decl) = decl {
                if let Some(recv) = &func_decl.recv {
                    if let Some(type_name) = self.extract_recv_type_name(recv) {
                        self.get_or_create_type_id(&type_name);
                    }
                }
            }
        }

        // Resolve forward-declared embedded interfaces
        let iface_names: Vec<String> = self.iface_defs.keys().cloned().collect();
        for _ in 0..iface_names.len() {
            let mut changed = false;
            for name in &iface_names {
                let methods = self.iface_defs.get(name).cloned().unwrap_or_default();
                for decl in &file.decl {
                    if let ast::Declaration::Type(type_decl) = decl {
                        for spec in &type_decl.specs {
                            if spec.name.name == *name {
                                if let ast::Expression::TypeInterface(iface) = &spec.typ {
                                    for field in &iface.methods.list {
                                        if field.name.is_empty() {
                                            if let ast::Expression::Ident(embedded_id) = &field.typ {
                                                if let Some(embedded_methods) = self.iface_defs.get(&embedded_id.name).cloned() {
                                                    for m in &embedded_methods {
                                                        if !methods.contains(m) {
                                                            changed = true;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
            for name in &iface_names {
                let mut methods = self.iface_defs.get(name).cloned().unwrap_or_default();
                let mut sigs = self.iface_method_sigs.get(name).cloned().unwrap_or_default();
                for decl in &file.decl {
                    if let ast::Declaration::Type(type_decl) = decl {
                        for spec in &type_decl.specs {
                            if spec.name.name == *name {
                                if let ast::Expression::TypeInterface(iface) = &spec.typ {
                                    for field in &iface.methods.list {
                                        if field.name.is_empty() {
                                            if let ast::Expression::Ident(embedded_id) = &field.typ {
                                                if let Some(embedded_methods) = self.iface_defs.get(&embedded_id.name).cloned() {
                                                    for m in embedded_methods {
                                                        if !methods.contains(&m) {
                                                            methods.push(m);
                                                        }
                                                    }
                                                }
                                                if let Some(embedded_sigs) = self.iface_method_sigs.get(&embedded_id.name).cloned() {
                                                    for (k, v) in embedded_sigs {
                                                        sigs.entry(k).or_insert(v);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                self.iface_defs.insert(name.clone(), methods);
                self.iface_method_sigs.insert(name.clone(), sigs);
            }
        }
    }

    fn collect_ident_refs(expr: &ast::Expression, refs: &mut Vec<String>) {
        match expr {
            ast::Expression::Ident(id) => {
                if !matches!(id.name.as_str(), "true" | "false" | "nil" | "iota") {
                    refs.push(id.name.clone());
                }
            }
            ast::Expression::Operation(op) => {
                Self::collect_ident_refs(&op.x, refs);
                if let Some(ref y) = op.y {
                    Self::collect_ident_refs(y, refs);
                }
            }
            ast::Expression::Call(call) => {
                Self::collect_ident_refs(&call.func, refs);
                for arg in &call.args {
                    Self::collect_ident_refs(arg, refs);
                }
            }
            ast::Expression::Paren(p) => Self::collect_ident_refs(&p.expr, refs),
            ast::Expression::Selector(sel) => Self::collect_ident_refs(&sel.x, refs),
            ast::Expression::Index(idx) => {
                if let Some(ref l) = idx.left {
                    Self::collect_ident_refs(l, refs);
                }
                Self::collect_ident_refs(&idx.index, refs);
            }
            ast::Expression::CompositeLit(comp) => {
                Self::collect_ident_refs(&comp.typ, refs);
                for kv in &comp.val.values {
                    if let ast::Element::Expr(e) = &kv.val {
                        Self::collect_ident_refs(e, refs);
                    }
                }
            }
            ast::Expression::Star(star) => Self::collect_ident_refs(&star.right, refs),
            ast::Expression::Slice(sl) => {
                Self::collect_ident_refs(&sl.left, refs);
                for idx_opt in &sl.index {
                    if let Some(e) = idx_opt {
                        Self::collect_ident_refs(e, refs);
                    }
                }
            }
            ast::Expression::TypeAssert(ta) => Self::collect_ident_refs(&ta.left, refs),
            ast::Expression::FuncLit(fl) => {
                for stmt in &fl.body.list {
                    Self::collect_stmt_ident_refs(stmt, refs);
                }
            }
            ast::Expression::List(exprs) => {
                for e in exprs {
                    Self::collect_ident_refs(e, refs);
                }
            }
            ast::Expression::Invar(inv) => Self::collect_ident_refs(&inv.expr, refs),
            _ => {}
        }
    }

    fn collect_stmt_ident_refs(stmt: &ast::Statement, refs: &mut Vec<String>) {
        match stmt {
            ast::Statement::Expr(es) => Self::collect_ident_refs(&es.expr, refs),
            ast::Statement::Return(ret) => {
                for e in &ret.ret {
                    Self::collect_ident_refs(e, refs);
                }
            }
            ast::Statement::Assign(a) => {
                for e in &a.right {
                    Self::collect_ident_refs(e, refs);
                }
            }
            ast::Statement::Block(block) => {
                for s in &block.list {
                    Self::collect_stmt_ident_refs(s, refs);
                }
            }
            ast::Statement::If(if_stmt) => {
                Self::collect_ident_refs(&if_stmt.cond, refs);
            }
            _ => {}
        }
    }

    fn sort_declarations_by_deps(decls: &[ast::Declaration]) -> Vec<ast::Declaration> {
        let mut var_decl_indices: Vec<usize> = Vec::new();
        let mut const_decl_indices: Vec<usize> = Vec::new();
        let mut other_indices: Vec<usize> = Vec::new();

        // Collect names defined by each var/const decl
        let mut all_var_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut all_const_names: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (i, decl) in decls.iter().enumerate() {
            match decl {
                ast::Declaration::Variable(var_decl) => {
                    var_decl_indices.push(i);
                    for spec in &var_decl.specs {
                        for name in &spec.name {
                            all_var_names.insert(name.name.clone());
                        }
                    }
                }
                ast::Declaration::Const(const_decl) => {
                    const_decl_indices.push(i);
                    for spec in &const_decl.specs {
                        for name in &spec.name {
                            all_const_names.insert(name.name.clone());
                        }
                    }
                }
                _ => {
                    other_indices.push(i);
                }
            }
        }

        // If no variable declarations or only one, no sorting needed
        if var_decl_indices.len() <= 1 {
            return decls.to_vec();
        }

        // Build dependency graph for variable declarations
        let global_names: std::collections::HashSet<&String> =
            all_var_names.iter().chain(all_const_names.iter()).collect();

        let mut var_deps: Vec<(usize, Vec<usize>)> = Vec::new();
        let mut idx_map: HashMap<String, usize> = HashMap::new();
        for (order, &di) in var_decl_indices.iter().enumerate() {
            if let ast::Declaration::Variable(var_decl) = &decls[di] {
                for spec in &var_decl.specs {
                    for name in &spec.name {
                        idx_map.insert(name.name.clone(), order);
                    }
                }
            }
        }

        for (order, &di) in var_decl_indices.iter().enumerate() {
            let mut deps = Vec::new();
            if let ast::Declaration::Variable(var_decl) = &decls[di] {
                for spec in &var_decl.specs {
                    for val in &spec.values {
                        let mut refs = Vec::new();
                        Self::collect_ident_refs(val, &mut refs);
                        for r in &refs {
                            if global_names.contains(r) {
                                if let Some(&dep_order) = idx_map.get(r) {
                                    if dep_order != order {
                                        deps.push(dep_order);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            var_deps.push((order, deps));
        }

        // Topological sort (Kahn's algorithm)
        let n = var_decl_indices.len();
        let mut in_degree = vec![0usize; n];
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (node, deps) in &var_deps {
            for &dep in deps {
                adj[dep].push(*node);
                in_degree[*node] += 1;
            }
        }

        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for i in 0..n {
            if in_degree[i] == 0 {
                queue.push_back(i);
            }
        }

        let mut sorted_var_order: Vec<usize> = Vec::new();
        while let Some(node) = queue.pop_front() {
            sorted_var_order.push(node);
            for &next in &adj[node] {
                in_degree[next] -= 1;
                if in_degree[next] == 0 {
                    queue.push_back(next);
                }
            }
        }

        // If there's a cycle, fall back to original order
        if sorted_var_order.len() != n {
            return decls.to_vec();
        }

        // Build the result: types/consts first (in original order), then sorted vars, then functions
        let mut result: Vec<ast::Declaration> = Vec::with_capacity(decls.len());

        // First: type and const declarations in original order
        for &i in &other_indices {
            if matches!(&decls[i], ast::Declaration::Type(_)) {
                result.push(decls[i].clone());
            }
        }
        for &i in &const_decl_indices {
            result.push(decls[i].clone());
        }

        // Then: sorted variable declarations
        for &order in &sorted_var_order {
            result.push(decls[var_decl_indices[order]].clone());
        }

        // Finally: function declarations in original order
        for &i in &other_indices {
            if matches!(&decls[i], ast::Declaration::Function(_)) {
                result.push(decls[i].clone());
            }
        }

        result
    }

    fn compile_declaration(&mut self, decl: &ast::Declaration) -> Result<(), Error> {
        match decl {
            ast::Declaration::Function(func_decl) => self.compile_func_decl(func_decl, false),
            ast::Declaration::Variable(var_decl) => {
                for spec in &var_decl.specs {
                    self.compile_global_var(spec)?;
                }
                Ok(())
            }
            ast::Declaration::Const(const_decl) => {
                let mut last_exprs: Vec<ast::Expression> = Vec::new();
                for (iota_val, spec) in const_decl.specs.iter().enumerate() {
                    self.current_iota = Some(iota_val as i64);
                    self.compile_global_const(spec, &mut last_exprs)?;
                }
                self.current_iota = None;
                Ok(())
            }
            ast::Declaration::Type(type_decl) => {
                for spec in &type_decl.specs {
                    Self::reject_unsupported_type(&spec.typ)?;
                    // Generic type definitions: store as template for later monomorphization
                    if !spec.params.list.is_empty() {
                        self.generic_types.insert(spec.name.name.clone(), spec.clone());
                        continue;
                    }
                    if let ast::Expression::TypeStruct(struct_type) = &spec.typ {
                        let struct_def = self.compute_struct_def(&struct_type.fields);
                        self.struct_defs
                            .insert(spec.name.name.clone(), struct_def);
                        self.register_struct_field_map_types(&spec.name.name, &struct_type.fields);
                    } else if let ast::Expression::Ident(base_type) = &spec.typ {
                        self.type_aliases.insert(
                            spec.name.name.clone(),
                            (base_type.name.clone(), spec.alias),
                        );
                    } else if matches!(&spec.typ, ast::Expression::TypeSlice(_) | ast::Expression::TypeMap(_) | ast::Expression::TypeArray(_)) {
                        self.named_composite_types.insert(
                            spec.name.name.clone(),
                            spec.typ.clone(),
                        );
                    }
                    // Interface types are handled during prescan
                }
                Ok(())
            }
        }
    }

    fn register_struct_field_map_types(&mut self, struct_name: &str, fields: &[ast::Field]) {
        for field in fields {
            if let ast::Expression::TypeMap(map_type) = &field.typ {
                let (kv, ks, vv, vs, sk, sv, vst) = self.map_key_val_types(map_type);
                let nested = self.build_nested_map_type_info(map_type);
                let mti = MapTypeInfo {
                    key_vt: kv, val_vt: vv, key_size: ks, val_size: vs,
                    is_string_key: sk, is_string_val: sv,
                    val_struct_type: vst, nested_map_val_type: nested,
                };
                for name_ident in &field.name {
                    self.struct_field_map_types.insert(
                        (struct_name.to_string(), name_ident.name.clone()),
                        mti.clone(),
                    );
                }
            }
        }
    }

    fn compute_struct_def(&self, fields: &[ast::Field]) -> StructDef {
        let mut result_fields = Vec::new();
        let mut offset: u32 = 0;
        let mut embedded_types = Vec::new();

        for field in fields {
            // Handle embedded (anonymous) fields: promote inner struct fields
            if field.name.is_empty() {
                if let ast::Expression::Ident(type_ident) = &field.typ {
                    let embed_name = type_ident.name.clone();
                    if let Some(inner_def) = self.struct_defs.get(&embed_name) {
                        let embed_offset = offset;
                        embedded_types.push((embed_name.clone(), embed_offset));
                        result_fields.push(StructFieldDef {
                            name: embed_name,
                            wasm_type: WasmType::I32,
                            offset: embed_offset,
                            go_type_tag: None,
                        });
                        for inner_field in &inner_def.fields {
                            let abs_offset = embed_offset + inner_field.offset;
                            result_fields.push(StructFieldDef {
                                name: inner_field.name.clone(),
                                wasm_type: inner_field.wasm_type,
                                offset: abs_offset,
                                go_type_tag: inner_field.go_type_tag.clone(),
                            });
                        }
                        offset += inner_def.total_size;
                        continue;
                    }
                }
            }

            let wasm_types = self.field_to_wasm_types(field);
            let names: Vec<String> = if field.name.is_empty() {
                vec!["".to_string()]
            } else {
                field.name.iter().map(|n| n.name.clone()).collect()
            };

            let go_type_tag = match &field.typ {
                ast::Expression::TypeSlice(_) => Some("__slice".to_string()),
                ast::Expression::TypeMap(_) => Some("__map".to_string()),
                ast::Expression::Ident(id) if id.name == "string" => Some("__string".to_string()),
                ast::Expression::TypePointer(_) => Some("__ptr".to_string()),
                ast::Expression::Ident(id) if self.struct_defs.contains_key(&id.name) => {
                    Some(id.name.clone())
                }
                _ => None,
            };

            for name in &names {
                if wasm_types.len() == 1 {
                    let wt = wasm_types[0];
                    let size = wt.byte_size();
                    let align = size;
                    offset = (offset + align - 1) & !(align - 1);
                    result_fields.push(StructFieldDef {
                        name: name.clone(),
                        wasm_type: wt,
                        offset,
                        go_type_tag: go_type_tag.clone(),
                    });
                    offset += size;
                } else {
                    for (i, &wt) in wasm_types.iter().enumerate() {
                        let size = wt.byte_size();
                        let align = size;
                        offset = (offset + align - 1) & !(align - 1);
                        result_fields.push(StructFieldDef {
                            name: if i == 0 {
                                name.clone()
                            } else {
                                format!("{}_{}", name, i)
                            },
                            wasm_type: wt,
                            offset,
                            go_type_tag: if i == 0 { go_type_tag.clone() } else { None },
                        });
                        offset += size;
                    }
                }
            }
        }

        let total_size = if offset == 0 {
            // Empty struct: zero size per Go spec.
            // alloc(0) may return the same pointer for distinct values;
            // the spec allows this for zero-size types.
            0
        } else {
            let align = 8u32;
            (offset + align - 1) & !(align - 1)
        };

        StructDef {
            fields: result_fields,
            total_size,
            embedded_types,
        }
    }

    fn compile_global_var(&mut self, spec: &ast::VarSpec) -> Result<(), Error> {
        // Check for composite types (struct, slice, map) that need heap allocation
        let is_composite_type = spec.typ.as_ref().map_or(false, |t| {
            matches!(t, ast::Expression::TypeSlice(_) | ast::Expression::TypeMap(_) | ast::Expression::TypeArray(_))
                || matches!(t, ast::Expression::Ident(id) if {
                    self.struct_defs.contains_key(&id.name)
                    || self.named_composite_types.contains_key(&id.name)
                })
        }) || spec.values.first().map_or(false, |v| {
            matches!(v, ast::Expression::CompositeLit(_))
        });

        if is_composite_type {
            // Composite globals are I32 pointers to heap-allocated data
            for name in &spec.name {
                let global_idx = self.next_global_idx;
                self.global_section.global(
                    GlobalType { val_type: ValType::I32, mutable: true, shared: false },
                    &ConstExpr::i32_const(0),
                );
                self.next_global_idx += 1;
                self.global_vars.insert(name.name.clone(), (global_idx, ValType::I32));

                // Track the struct type for selector access
                if let Some(type_expr) = &spec.typ {
                    if let ast::Expression::Ident(type_id) = type_expr {
                        if self.struct_defs.contains_key(&type_id.name) {
                            // Will be tracked at usage site via global_var_struct_types
                        }
                    }
                }

                if let Some(val) = spec.values.first() {
                    self.global_var_inits.push((name.name.clone(), val.clone(), ValType::I32));
                }
            }
            return Ok(());
        }

        let is_string_type = spec.typ.as_ref().map_or(false, |t| {
            matches!(t, ast::Expression::Ident(id) if id.name == "string")
        }) || spec.values.first().map_or(false, |v| {
            matches!(v, ast::Expression::BasicLit(lit) if lit.kind == LitKind::String)
                || matches!(v, ast::Expression::Operation(op) if {
                    let is_str_const = self.try_eval_const_expr(v);
                    matches!(is_str_const, Some(ConstValue::Str(_)))
                })
        });

        if is_string_type {
            // Strings are (ptr, len) pairs; use two i32 globals and defer initialization
            for name in &spec.name {
                let ptr_idx = self.next_global_idx;
                self.global_section.global(
                    GlobalType { val_type: ValType::I32, mutable: true, shared: false },
                    &ConstExpr::i32_const(0),
                );
                self.next_global_idx += 1;

                let len_idx = self.next_global_idx;
                self.global_section.global(
                    GlobalType { val_type: ValType::I32, mutable: true, shared: false },
                    &ConstExpr::i32_const(0),
                );
                self.next_global_idx += 1;

                self.global_vars.insert(name.name.clone(), (ptr_idx, ValType::I32));
                self.global_vars.insert(format!("{}_1", name.name), (len_idx, ValType::I32));

                if let Some(val) = spec.values.first() {
                    self.global_var_inits.push((name.name.clone(), val.clone(), ValType::I32));
                }
            }
            return Ok(());
        }

        let vt = if let Some(type_expr) = &spec.typ {
            self.expr_to_val_type(type_expr)
        } else if let Some(val) = spec.values.first() {
            self.infer_val_type_no_locals(val)
        } else {
            ValType::I64
        };

        let const_init = if let Some(val) = spec.values.first() {
            self.try_eval_const_expr(val)
        } else {
            None
        };

        let has_non_const_init = spec.values.first().is_some() && const_init.is_none();

        for name in &spec.name {
            let global_idx = self.next_global_idx;
            self.global_section.global(
                GlobalType {
                    val_type: vt,
                    mutable: true,
                    shared: false,
                },
                &match (&const_init, vt) {
                    (Some(ConstValue::I64(v)), ValType::I64) => ConstExpr::i64_const(*v),
                    (Some(ConstValue::I64(v)), ValType::I32) => ConstExpr::i32_const(*v as i32),
                    (Some(ConstValue::F64(v)), ValType::F64) => ConstExpr::f64_const(*v),
                    (Some(ConstValue::F64(v)), ValType::F32) => ConstExpr::f32_const(*v as f32),
                    (Some(ConstValue::Bool(v)), _) => ConstExpr::i32_const(*v as i32),
                    (_, ValType::I64) => ConstExpr::i64_const(0),
                    (_, ValType::I32) => ConstExpr::i32_const(0),
                    (_, ValType::F64) => ConstExpr::f64_const(0.0),
                    (_, ValType::F32) => ConstExpr::f32_const(0.0),
                    _ => ConstExpr::i64_const(0),
                },
            );
            self.next_global_idx += 1;
            self.global_vars.insert(name.name.clone(), (global_idx, vt));

            if has_non_const_init {
                if let Some(val) = spec.values.first() {
                    if let ast::Expression::Ident(func_ident) = val {
                        if let Some(fi) = self.functions.iter().find(|f| f.name == func_ident.name) {
                            self.global_func_vars.insert(name.name.clone(), fi.wasm_func_idx);
                            continue;
                        }
                    }
                    self.global_var_inits.push((name.name.clone(), val.clone(), vt));
                }
            }
        }

        Ok(())
    }

    fn compile_global_const(
        &mut self,
        spec: &ast::ConstSpec,
        last_exprs: &mut Vec<ast::Expression>,
    ) -> Result<(), Error> {
        let exprs_to_use = if spec.values.is_empty() {
            last_exprs.as_slice()
        } else {
            *last_exprs = spec.values.clone();
            spec.values.as_slice()
        };

        for (i, name) in spec.name.iter().enumerate() {
            let val_expr = exprs_to_use.get(i).or(exprs_to_use.first());

            if let Some(expr) = val_expr {
                let cv = self.eval_const_expr(expr, &name.name)?;
                self.constants.insert(name.name.clone(), cv);
            } else {
                return Err(Error::InternalError(format!(
                    "constant '{}' must have a value",
                    name.name
                )));
            }
        }
        Ok(())
    }

    fn eval_const_expr(&self, expr: &ast::Expression, name: &str) -> Result<ConstValue, Error> {
        self.try_eval_const_expr(expr).ok_or_else(|| {
            if self.is_const_overflow(expr) {
                Error::SyntaxError(format!("constant {} overflows integer", name))
            } else {
                Error::InternalError(format!(
                    "constant '{}' has a non-constant initializer",
                    name
                ))
            }
        })
    }

    fn is_const_overflow(&self, expr: &ast::Expression) -> bool {
        if let ast::Expression::Operation(op) = expr {
            if let Some(ref y) = op.y {
                let lhs = self.try_eval_const_expr(&op.x);
                let rhs = self.try_eval_const_expr(y);
                if let (Some(ConstValue::I64(a)), Some(ConstValue::I64(b))) = (&lhs, &rhs) {
                    return match op.op {
                        Operator::Add => a.checked_add(*b).is_none(),
                        Operator::Sub => a.checked_sub(*b).is_none(),
                        Operator::Star => a.checked_mul(*b).is_none(),
                        Operator::Shl => *b < 0 || *b > 63 || a.checked_shl(*b as u32).is_none(),
                        _ => false,
                    };
                }
            }
        }
        false
    }

    fn try_eval_const_expr(&self, expr: &ast::Expression) -> Option<ConstValue> {
        match expr {
            ast::Expression::BasicLit(lit) => match lit.kind {
                LitKind::Integer => Self::parse_go_int(&lit.value).ok().map(ConstValue::I64),
                LitKind::Float => Self::parse_go_float(&lit.value).ok().map(ConstValue::F64),
                LitKind::String => {
                    Self::extract_string_content(&lit.value).map(ConstValue::Str)
                }
                LitKind::Char => {
                    let s = lit.value.trim_matches('\'');
                    Self::unescape_go_char(s).ok().map(|c| ConstValue::I64(c as i64))
                }
                LitKind::Imag => {
                    let num_str = lit.value.trim_end_matches('i');
                    num_str.parse::<f64>().ok().map(|v| ConstValue::Complex128(0.0, v))
                }
                _ => None,
            },
            ast::Expression::Ident(ident) => match ident.name.as_str() {
                "true" => Some(ConstValue::Bool(true)),
                "false" => Some(ConstValue::Bool(false)),
                "iota" => self.current_iota.map(ConstValue::I64),
                _ => self.constants.get(&ident.name).cloned(),
            },
            ast::Expression::Operation(op) => {
                if op.y.is_none() {
                    let inner = self.try_eval_const_expr(&op.x)?;
                    return match op.op {
                        Operator::Sub => match inner {
                            ConstValue::I64(v) => v.checked_neg().map(ConstValue::I64),
                            ConstValue::F64(v) => Some(ConstValue::F64(-v)),
                            ConstValue::Complex128(r, i) => Some(ConstValue::Complex128(-r, -i)),
                            _ => None,
                        },
                        Operator::Add => Some(inner),
                        Operator::Xor => match inner {
                            ConstValue::I64(v) => Some(ConstValue::I64(!v)),
                            _ => None,
                        },
                        Operator::Not => match inner {
                            ConstValue::Bool(v) => Some(ConstValue::Bool(!v)),
                            _ => None,
                        },
                        _ => None,
                    };
                }
                let lhs = self.try_eval_const_expr(&op.x)?;
                let rhs = self.try_eval_const_expr(op.y.as_ref().unwrap())?;
                match (&lhs, &rhs) {
                    (ConstValue::I64(a), ConstValue::I64(b)) => {
                        match op.op {
                            Operator::Add => Some(ConstValue::I64(a.checked_add(*b)?)),
                            Operator::Sub => Some(ConstValue::I64(a.checked_sub(*b)?)),
                            Operator::Star => Some(ConstValue::I64(a.checked_mul(*b)?)),
                            Operator::Quo => Some(ConstValue::I64(a.checked_div(*b)?)),
                            Operator::Rem => Some(ConstValue::I64(a.checked_rem(*b)?)),
                            Operator::Shl => {
                                if *b < 0 { return None; }
                                if *b >= 64 { return Some(ConstValue::I64(0)); }
                                Some(ConstValue::I64(a.checked_shl(*b as u32)?))
                            }
                            Operator::Shr => {
                                if *b < 0 { return None; }
                                if *b >= 64 {
                                    return Some(ConstValue::I64(if *a < 0 { -1 } else { 0 }));
                                }
                                Some(ConstValue::I64(a.checked_shr(*b as u32)?))
                            }
                            Operator::And => Some(ConstValue::I64(a & b)),
                            Operator::Or => Some(ConstValue::I64(a | b)),
                            Operator::Xor => Some(ConstValue::I64(a ^ b)),
                            Operator::AndNot => Some(ConstValue::I64(a & !b)),
                            Operator::Equal => Some(ConstValue::Bool(a == b)),
                            Operator::NotEqual => Some(ConstValue::Bool(a != b)),
                            Operator::Less => Some(ConstValue::Bool(a < b)),
                            Operator::Greater => Some(ConstValue::Bool(a > b)),
                            Operator::LessEqual => Some(ConstValue::Bool(a <= b)),
                            Operator::GreaterEqual => Some(ConstValue::Bool(a >= b)),
                            _ => None,
                        }
                    }
                    (ConstValue::F64(a), ConstValue::F64(b)) => {
                        match op.op {
                            Operator::Add => Some(ConstValue::F64(a + b)),
                            Operator::Sub => Some(ConstValue::F64(a - b)),
                            Operator::Star => Some(ConstValue::F64(a * b)),
                            Operator::Quo => Some(ConstValue::F64(a / b)),
                            Operator::Equal => Some(ConstValue::Bool(a == b)),
                            Operator::NotEqual => Some(ConstValue::Bool(a != b)),
                            Operator::Less => Some(ConstValue::Bool(a < b)),
                            Operator::Greater => Some(ConstValue::Bool(a > b)),
                            Operator::LessEqual => Some(ConstValue::Bool(a <= b)),
                            Operator::GreaterEqual => Some(ConstValue::Bool(a >= b)),
                            _ => None,
                        }
                    }
                    (ConstValue::I64(a), ConstValue::F64(b)) => {
                        let a = *a as f64;
                        match op.op {
                            Operator::Add => Some(ConstValue::F64(a + b)),
                            Operator::Sub => Some(ConstValue::F64(a - b)),
                            Operator::Star => Some(ConstValue::F64(a * b)),
                            Operator::Quo => Some(ConstValue::F64(a / b)),
                            Operator::Equal => Some(ConstValue::Bool(a == *b)),
                            Operator::NotEqual => Some(ConstValue::Bool(a != *b)),
                            Operator::Less => Some(ConstValue::Bool(a < *b)),
                            Operator::Greater => Some(ConstValue::Bool(a > *b)),
                            Operator::LessEqual => Some(ConstValue::Bool(a <= *b)),
                            Operator::GreaterEqual => Some(ConstValue::Bool(a >= *b)),
                            _ => None,
                        }
                    }
                    (ConstValue::F64(a), ConstValue::I64(b)) => {
                        let b = *b as f64;
                        match op.op {
                            Operator::Add => Some(ConstValue::F64(a + b)),
                            Operator::Sub => Some(ConstValue::F64(a - b)),
                            Operator::Star => Some(ConstValue::F64(a * b)),
                            Operator::Quo => Some(ConstValue::F64(a / b)),
                            Operator::Equal => Some(ConstValue::Bool(*a == b)),
                            Operator::NotEqual => Some(ConstValue::Bool(*a != b)),
                            Operator::Less => Some(ConstValue::Bool(*a < b)),
                            Operator::Greater => Some(ConstValue::Bool(*a > b)),
                            Operator::LessEqual => Some(ConstValue::Bool(*a <= b)),
                            Operator::GreaterEqual => Some(ConstValue::Bool(*a >= b)),
                            _ => None,
                        }
                    }
                    (ConstValue::Str(a), ConstValue::Str(b)) => {
                        match op.op {
                            Operator::Add => Some(ConstValue::Str(format!("{}{}", a, b))),
                            Operator::Equal => Some(ConstValue::Bool(a == b)),
                            Operator::NotEqual => Some(ConstValue::Bool(a != b)),
                            Operator::Less => Some(ConstValue::Bool(a < b)),
                            Operator::Greater => Some(ConstValue::Bool(a > b)),
                            Operator::LessEqual => Some(ConstValue::Bool(a <= b)),
                            Operator::GreaterEqual => Some(ConstValue::Bool(a >= b)),
                            _ => None,
                        }
                    }
                    (ConstValue::Bool(a), ConstValue::Bool(b)) => {
                        match op.op {
                            Operator::AndAnd => Some(ConstValue::Bool(*a && *b)),
                            Operator::OrOr => Some(ConstValue::Bool(*a || *b)),
                            Operator::Equal => Some(ConstValue::Bool(a == b)),
                            Operator::NotEqual => Some(ConstValue::Bool(a != b)),
                            _ => None,
                        }
                    }
                    // Complex constant arithmetic: 1 + 2i, complex + complex, etc.
                    (ConstValue::Complex128(ar, ai), ConstValue::Complex128(br, bi)) => {
                        match op.op {
                            Operator::Add => Some(ConstValue::Complex128(ar + br, ai + bi)),
                            Operator::Sub => Some(ConstValue::Complex128(ar - br, ai - bi)),
                            Operator::Star => Some(ConstValue::Complex128(ar * br - ai * bi, ar * bi + ai * br)),
                            Operator::Quo => {
                                let denom = br * br + bi * bi;
                                Some(ConstValue::Complex128(
                                    (ar * br + ai * bi) / denom,
                                    (ai * br - ar * bi) / denom,
                                ))
                            }
                            Operator::Equal => Some(ConstValue::Bool(ar == br && ai == bi)),
                            Operator::NotEqual => Some(ConstValue::Bool(ar != br || ai != bi)),
                            _ => None,
                        }
                    }
                    (ConstValue::I64(a), ConstValue::Complex128(br, bi)) => {
                        let ar = *a as f64;
                        match op.op {
                            Operator::Add => Some(ConstValue::Complex128(ar + br, *bi)),
                            Operator::Sub => Some(ConstValue::Complex128(ar - br, -bi)),
                            Operator::Star => Some(ConstValue::Complex128(ar * br, ar * bi)),
                            Operator::Quo => {
                                let denom = br * br + bi * bi;
                                Some(ConstValue::Complex128(
                                    (ar * br) / denom,
                                    (-ar * bi) / denom,
                                ))
                            }
                            _ => None,
                        }
                    }
                    (ConstValue::Complex128(ar, ai), ConstValue::I64(b)) => {
                        let br = *b as f64;
                        match op.op {
                            Operator::Add => Some(ConstValue::Complex128(ar + br, *ai)),
                            Operator::Sub => Some(ConstValue::Complex128(ar - br, *ai)),
                            Operator::Star => Some(ConstValue::Complex128(ar * br, ai * br)),
                            Operator::Quo => {
                                let denom = br * br;
                                Some(ConstValue::Complex128(ar * br / denom, ai * br / denom))
                            }
                            _ => None,
                        }
                    }
                    (ConstValue::F64(a), ConstValue::Complex128(br, bi)) => {
                        match op.op {
                            Operator::Add => Some(ConstValue::Complex128(a + br, *bi)),
                            Operator::Sub => Some(ConstValue::Complex128(a - br, -bi)),
                            Operator::Star => Some(ConstValue::Complex128(a * br, a * bi)),
                            Operator::Quo => {
                                let denom = br * br + bi * bi;
                                Some(ConstValue::Complex128(
                                    (a * br) / denom,
                                    (-a * bi) / denom,
                                ))
                            }
                            _ => None,
                        }
                    }
                    (ConstValue::Complex128(ar, ai), ConstValue::F64(b)) => {
                        match op.op {
                            Operator::Add => Some(ConstValue::Complex128(ar + b, *ai)),
                            Operator::Sub => Some(ConstValue::Complex128(ar - b, *ai)),
                            Operator::Star => Some(ConstValue::Complex128(ar * b, ai * b)),
                            Operator::Quo => Some(ConstValue::Complex128(ar / b, ai / b)),
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            ast::Expression::Paren(p) => self.try_eval_const_expr(&p.expr),
            _ => None,
        }
    }

    fn infer_val_type_no_locals(&self, expr: &ast::Expression) -> ValType {
        match expr {
            ast::Expression::BasicLit(lit) => match lit.kind {
                LitKind::Integer => ValType::I64,
                LitKind::Float => ValType::F64,
                LitKind::String => ValType::I32,
                LitKind::Imag => ValType::I32,
                _ => ValType::I64,
            },
            ast::Expression::Ident(ident) => match ident.name.as_str() {
                "true" | "false" => ValType::I32,
                _ => {
                    if let Some(cv) = self.constants.get(&ident.name) {
                        return match cv {
                            ConstValue::I64(_) => ValType::I64,
                            ConstValue::F64(_) => ValType::F64,
                            ConstValue::Bool(_) => ValType::I32,
                            ConstValue::Str(_) => ValType::I32,
                            ConstValue::Complex128(_, _) => ValType::I32,
                        };
                    }
                    ValType::I64
                }
            },
            ast::Expression::Operation(op) if op.y.is_some() => {
                let lhs = self.infer_val_type_no_locals(&op.x);
                let rhs = self.infer_val_type_no_locals(op.y.as_ref().unwrap());
                if lhs == ValType::F64 || rhs == ValType::F64 {
                    ValType::F64
                } else {
                    lhs
                }
            }
            _ => ValType::I64,
        }
    }

    fn extract_recv_type_name(&self, recv: &ast::FieldList) -> Option<String> {
        recv.list.first().and_then(|field| match &field.typ {
            ast::Expression::TypePointer(p) => {
                if let ast::Expression::Ident(id) = p.typ.as_ref() {
                    Some(id.name.clone())
                } else {
                    None
                }
            }
            ast::Expression::Ident(id) => Some(id.name.clone()),
            ast::Expression::Index(idx) => {
                if let Some(ast::Expression::Ident(id)) = idx.left.as_deref() {
                    Some(id.name.clone())
                } else {
                    None
                }
            }
            ast::Expression::IndexList(idx_list) => {
                if let ast::Expression::Ident(id) = idx_list.left.as_ref() {
                    Some(id.name.clone())
                } else {
                    None
                }
            }
            _ => None,
        })
    }

    fn is_generic_recv(&self, recv: &ast::FieldList) -> bool {
        recv.list.first().map_or(false, |field| {
            matches!(&field.typ,
                ast::Expression::Index(_) | ast::Expression::IndexList(_))
        })
    }

    fn compile_func_decl(&mut self, decl: &ast::FuncDecl, defer_code: bool) -> Result<(), Error> {
        let name = &decl.name.name;

        // Generic functions: store for later monomorphization instead of compiling now
        if !decl.typ.typ_params.list.is_empty() {
            self.generic_funcs.insert(name.clone(), decl.clone());
            return Ok(());
        }

        // Methods on generic types: store for later monomorphization
        if let Some(recv) = &decl.recv {
            if self.is_generic_recv(recv) {
                let recv_type_name = self.extract_recv_type_name(recv);
                if let Some(type_name) = recv_type_name {
                    let key = format!("{}.{}", type_name, name);
                    self.generic_funcs.insert(key, decl.clone());
                    return Ok(());
                }
            }
        }

        let is_method = decl.recv.is_some();

        let recv_type_name = if let Some(recv) = &decl.recv {
            self.extract_recv_type_name(recv)
        } else {
            None
        };

        let internal_name = if let Some(ref rtn) = recv_type_name {
            format!("{}.{}", rtn, name)
        } else {
            name.clone()
        };

        let is_exported =
            name.chars().next().map_or(false, |c| c.is_uppercase()) && !is_method;

        let mut param_types: Vec<ValType> = Vec::new();
        let mut param_names: Vec<String> = Vec::new();

        if let Some(recv) = &decl.recv {
            for field in &recv.list {
                let recv_vt = match &field.typ {
                    ast::Expression::TypePointer(_) => ValType::I32,
                    ast::Expression::Ident(id) => {
                        if self.struct_defs.contains_key(&id.name) {
                            ValType::I32
                        } else {
                            let resolved = self.resolve_type_name(&id.name);
                            match resolved {
                                "int" | "int64" | "uint" | "uint64" => ValType::I64,
                                "float64" => ValType::F64,
                                "float32" => ValType::F32,
                                "int32" | "uint32" | "byte" | "bool" | "rune" => ValType::I32,
                                _ => ValType::I32,
                            }
                        }
                    }
                    _ => ValType::I32,
                };
                param_types.push(recv_vt);
                let recv_name = field.name.first().map_or("self", |id| &id.name);
                param_names.push(recv_name.to_string());
            }
        }

        let mut is_variadic = false;
        let mut variadic_elem_vt: Option<ValType> = None;
        let mut variadic_param_name: Option<String> = None;

        for field in &decl.typ.params.list {
            // Detect variadic parameter: ...T
            if let ast::Expression::Ellipsis(ellipsis) = &field.typ {
                is_variadic = true;
                let elem_vt = if let Some(ref elt) = ellipsis.elt {
                    Self::infer_array_elem_vt(elt)
                } else {
                    ValType::I64
                };
                variadic_elem_vt = Some(elem_vt);
                param_types.push(ValType::I32); // slice header pointer
                let vname = field.name.first().map_or(
                    format!("_param{}", param_names.len()),
                    |id| id.name.clone()
                );
                variadic_param_name = Some(vname.clone());
                param_names.push(vname);
                continue;
            }

            let is_iface_param = match &field.typ {
                ast::Expression::Ident(id) => {
                    self.iface_defs.contains_key(&id.name) || id.name == "error" || id.name == "any"
                }
                ast::Expression::TypeInterface(_) => true,
                _ => false,
            };

            let is_string_param = matches!(&field.typ, ast::Expression::Ident(id) if id.name == "string");

            let field_wasm_types = self.field_to_wasm_types(field);
            if field.name.is_empty() {
                for wt in &field_wasm_types {
                    param_types.push(wt.to_val_type());
                    param_names.push(format!("_param{}", param_names.len()));
                }
            } else {
                for ident in field.name.iter() {
                    if is_string_param {
                        param_types.push(ValType::I32);
                        param_names.push(ident.name.clone());
                        param_types.push(ValType::I32);
                        param_names.push(format!("{}__str_len", ident.name));
                    } else {
                        if !field_wasm_types.is_empty() {
                            param_types.push(field_wasm_types[0].to_val_type());
                        } else {
                            param_types.push(ValType::I32);
                        }
                        param_names.push(ident.name.clone());
                    }
                    if is_iface_param {
                        param_types.push(ValType::I32);
                        param_names.push(format!("{}__type_id", ident.name));
                    }
                }
            }
        }

        let mut result_types: Vec<ValType> = Vec::new();
        let mut result_go_types: Vec<String> = Vec::new();
        for field in &decl.typ.result.list {
            let field_wasm_types = self.field_to_wasm_types(field);
            let go_type_name = self.expr_type_name(&field.typ);
            for wt in &field_wasm_types {
                result_types.push(wt.to_val_type());
                result_go_types.push(go_type_name.clone());
            }
        }

        let type_idx = self.next_type_idx;
        self.type_section
            .ty()
            .function(param_types.clone(), result_types.clone());
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let is_init = name == "init"
            && !is_method
            && decl.typ.params.list.is_empty()
            && decl.typ.result.list.is_empty();

        if is_init {
            self.init_func_indices.push(func_idx);
        }

        if is_exported {
            self.export_section
                .export(name, ExportKind::Func, func_idx);
        }

        let wasm_params: Vec<(String, WasmType)> = param_names
            .iter()
            .zip(param_types.iter())
            .map(|(n, vt)| {
                (
                    n.clone(),
                    match vt {
                        ValType::I32 => WasmType::I32,
                        ValType::I64 => WasmType::I64,
                        ValType::F32 => WasmType::F32,
                        ValType::F64 => WasmType::F64,
                        _ => WasmType::I32,
                    },
                )
            })
            .collect();

        let wasm_results: Vec<WasmType> = result_types
            .iter()
            .map(|vt| match vt {
                ValType::I32 => WasmType::I32,
                ValType::I64 => WasmType::I64,
                ValType::F32 => WasmType::F32,
                ValType::F64 => WasmType::F64,
                _ => WasmType::I32,
            })
            .collect();

        let recv_count = if decl.recv.is_some() { 1 } else { 0 };
        let mut iface_param_indices: Vec<usize> = Vec::new();
        {
            let mut go_arg_idx: usize = 0;
            for field in &decl.typ.params.list {
                let is_iface_field = match &field.typ {
                    ast::Expression::Ident(id) => {
                        self.iface_defs.contains_key(&id.name) || id.name == "error" || id.name == "any"
                    }
                    ast::Expression::TypeInterface(_) => true,
                    _ => false,
                };
                let name_count = if field.name.is_empty() { 1 } else { field.name.len() };
                for _ in 0..name_count {
                    if is_iface_field {
                        iface_param_indices.push(go_arg_idx);
                    }
                    go_arg_idx += 1;
                }
            }
        }
        let _ = recv_count;

        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: internal_name,
            params: wasm_params,
            results: wasm_results,
            result_go_types: result_go_types.clone(),
            is_exported,
            recv_type: recv_type_name.clone(),
            is_variadic,
            variadic_elem_vt,
            iface_param_indices,
        });

        let param_entries: Vec<(String, ValType)> = param_names
            .iter()
            .zip(param_types.iter())
            .map(|(n, vt)| (n.clone(), *vt))
            .collect();
        let mut locals = LocalAlloc::new(param_entries);

        // Track variadic parameter as slice
        if let Some(ref vp_name) = variadic_param_name {
            locals.set_var_struct_type(vp_name, "__slice");
            if let Some(evtype) = variadic_elem_vt {
                locals.slice_elem_types.insert(vp_name.clone(), evtype);
            }
        }

        // Track struct type for receiver
        if let Some(recv) = &decl.recv {
            for field in &recv.list {
                let recv_name = field.name.first().map_or("self", |id| &id.name);
                if let Some(ref rtn) = recv_type_name {
                    locals.set_var_struct_type(recv_name, rtn);
                }
            }
        }

        // Track struct types and signedness for parameters
        for field in &decl.typ.params.list {
            if let ast::Expression::TypeSlice(slice_type) = &field.typ {
                let elem_vt = Self::infer_array_elem_vt(&slice_type.typ);
                for name_ident in &field.name {
                    locals.set_var_struct_type(&name_ident.name, "__slice");
                    locals.slice_elem_types.insert(name_ident.name.clone(), elem_vt);
                    if let ast::Expression::TypeSlice(inner_st) = slice_type.typ.as_ref() {
                        let inner_vt = Self::infer_array_elem_vt(&inner_st.typ);
                        locals.nested_slice_inner_elem_types.insert(name_ident.name.clone(), inner_vt);
                    }
                    if let ast::Expression::Ident(el_id) = slice_type.typ.as_ref() {
                        if self.struct_defs.contains_key(&el_id.name) {
                            locals.slice_elem_struct_types.insert(name_ident.name.clone(), el_id.name.clone());
                        }
                    }
                }
            }
            if let ast::Expression::Ident(type_ident) = &field.typ {
                if self.struct_defs.contains_key(&type_ident.name) {
                    for name_ident in &field.name {
                        locals.set_var_struct_type(&name_ident.name, &type_ident.name);
                    }
                }
                if type_ident.name == "Context" {
                    for name_ident in &field.name {
                        locals.set_var_struct_type(&name_ident.name, "__context");
                    }
                }
                if type_ident.name == "string" {
                    for name_ident in &field.name {
                        locals.set_var_struct_type(&name_ident.name, "__string");
                        let ptr_idx = locals.find(&name_ident.name).unwrap_or(0);
                        let len_name = format!("{}__str_len", name_ident.name);
                        let len_idx = locals.find(&len_name).unwrap_or_else(|| {
                            locals.add_local(&len_name, ValType::I32)
                        });
                        locals.string_locals.insert(name_ident.name.clone(), (ptr_idx, len_idx));
                    }
                }
                if Self::is_unsigned_type_name(&type_ident.name) {
                    for name_ident in &field.name {
                        locals.unsigned_vars.insert(name_ident.name.clone());
                    }
                }
                if self.iface_defs.contains_key(&type_ident.name)
                    || type_ident.name == "error"
                    || type_ident.name == "any"
                {
                    for name_ident in &field.name {
                        let iface_tag = format!("__iface_{}", type_ident.name);
                        locals.set_var_struct_type(&name_ident.name, &iface_tag);
                        let tid_param_name = format!("{}__type_id", name_ident.name);
                        let tid_local = locals.find(&tid_param_name).unwrap_or_else(|| {
                            locals.add_local(&tid_param_name, ValType::I32)
                        });
                        self.iface_var_type_ids.insert(name_ident.name.clone(), tid_local);
                    }
                }
            }
            if let ast::Expression::TypePointer(ptr) = &field.typ {
                if let ast::Expression::Ident(type_ident) = ptr.typ.as_ref() {
                    if self.struct_defs.contains_key(&type_ident.name) {
                        for name_ident in &field.name {
                            locals.set_var_struct_type(&name_ident.name, &type_ident.name);
                        }
                    } else {
                        let ptr_tag = match type_ident.name.as_str() {
                            "int" | "int64" | "uint" | "uint64" => "__ptr_i64",
                            "float32" => "__ptr_f32",
                            "float64" => "__ptr_f64",
                            _ => "__ptr_i32",
                        };
                        for name_ident in &field.name {
                            locals.set_var_struct_type(&name_ident.name, ptr_tag);
                        }
                    }
                }
            }
            if let ast::Expression::Selector(sel) = &field.typ {
                if let ast::Expression::Ident(pkg) = sel.x.as_ref() {
                    if pkg.name == "context" && sel.sel.name == "Context" {
                        for name_ident in &field.name {
                            locals.set_var_struct_type(&name_ident.name, "__context");
                        }
                    }
                }
            }
        }

        // Collect named return variables
        let mut named_returns: Vec<(String, ValType)> = Vec::new();
        for field in &decl.typ.result.list {
            let field_wasm_types = self.field_to_wasm_types(field);
            for (i, ident) in field.name.iter().enumerate() {
                let vt = if i < field_wasm_types.len() {
                    field_wasm_types[i].to_val_type()
                } else if !field_wasm_types.is_empty() {
                    field_wasm_types[0].to_val_type()
                } else {
                    ValType::I64
                };
                let _local_idx = locals.add_local(&ident.name, vt);
                named_returns.push((ident.name.clone(), vt));
            }
        }

        self.deferred_calls.push(Vec::new());

        let mut func_body: Vec<Instruction<'static>> = Vec::new();

        if let Some(body) = &decl.body {
            self.named_returns = named_returns.clone();
            self.current_result_types = result_types.clone();
            self.current_result_go_types = result_go_types.clone();
            self.compile_block(&body, &mut func_body, &mut locals, &result_types)?;
            self.named_returns = Vec::new();
            self.current_result_types = Vec::new();
            self.current_result_go_types = Vec::new();
        }

        self.emit_deferred_calls(&mut func_body);
        self.deferred_calls.pop();

        let body_always_returns = decl
            .body
            .as_ref()
            .map_or(false, |b| Self::block_always_returns(&b.list));

        if body_always_returns {
            if !result_types.is_empty()
                && func_body
                    .last()
                    .map_or(true, |i| !matches!(i, Instruction::Return))
            {
                func_body.push(Instruction::Unreachable);
            }
        } else if result_types.is_empty()
            || func_body
                .last()
                .map_or(true, |i| !matches!(i, Instruction::Return))
        {
            if !named_returns.is_empty() {
                for (name, _vt) in &named_returns {
                    if let Some(idx) = locals.find(name) {
                        func_body.push(Instruction::LocalGet(idx));
                    }
                }
            } else if !result_types.is_empty() {
                return Err(Error::SyntaxError(format!(
                    "missing return at end of function '{}'",
                    decl.name.name
                )));
            }
        }

        func_body.push(Instruction::End);

        let mut func = Function::new(locals.local_types());
        for instr in &func_body {
            func.instruction(instr);
        }

        if defer_code {
            self.pending_closures.push(func);
        } else {
            self.code_section.function(&func);
            for closure_func in self.pending_closures.drain(..) {
                self.code_section.function(&closure_func);
            }
        }

        Ok(())
    }

    fn compile_block(
        &mut self,
        block: &ast::BlockStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        locals.push_scope();
        for stmt in &block.list {
            self.compile_statement(stmt, out, locals, result_types)?;
        }
        locals.pop_scope();
        Ok(())
    }

    fn compile_statement(
        &mut self,
        stmt: &ast::Statement,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        match stmt {
            ast::Statement::Return(ret) => self.compile_return(ret, out, locals, result_types),
            ast::Statement::Expr(expr_stmt) => {
                if let ast::Expression::Call(call) = &expr_stmt.expr {
                    if let ast::Expression::Ident(ident) = call.func.as_ref() {
                        match ident.name.as_str() {
                            "append" | "cap" | "complex" | "imag" | "len" | "make" | "new" | "real" => {
                                return Err(Error::SyntaxError(format!(
                                    "{}() not used (value is discarded); not permitted in statement context",
                                    ident.name
                                )));
                            }
                            _ => {}
                        }
                    }
                }
                self.compile_expression(&expr_stmt.expr, out, locals)?;
                let wasm_types = self.expression_result_count(&expr_stmt.expr, Some(locals));
                for _ in 0..wasm_types {
                    out.push(Instruction::Drop);
                }
                Ok(())
            }
            ast::Statement::Assign(assign) => self.compile_assign(assign, out, locals),
            ast::Statement::If(if_stmt) => {
                self.compile_if(if_stmt, out, locals, result_types)
            }
            ast::Statement::For(for_stmt) => {
                self.compile_for(for_stmt, out, locals, result_types)
            }
            ast::Statement::Block(block) => {
                self.compile_block(block, out, locals, result_types)
            }
            ast::Statement::IncDec(incdec) => self.compile_incdec(incdec, out, locals),
            ast::Statement::Switch(switch) => {
                self.compile_switch(switch, out, locals, result_types)
            }
            ast::Statement::Branch(branch) => self.compile_branch(branch, out),
            ast::Statement::Declaration(decl_stmt) => {
                self.compile_decl_stmt(decl_stmt, out, locals)
            }
            ast::Statement::Defer(defer) => {
                // Compile arguments eagerly at the defer site
                let mut arg_locals = Vec::new();
                for arg in &defer.call.args {
                    let vt = self.infer_val_type(arg, locals);
                    self.compile_expression(arg, out, locals)?;
                    let temp = locals.add_local(
                        &format!("__defer_arg_{}", locals.locals.len()),
                        vt,
                    );
                    out.push(Instruction::LocalSet(temp));
                    arg_locals.push((temp, vt));
                }

                if let ast::Expression::FuncLit(_) = defer.call.func.as_ref() {
                    // Compile the closure literal — this adds it as a function
                    self.compile_expression(&defer.call.func, out, locals)?;
                    // compile_func_lit pushes func_idx onto the WASM stack; drop it
                    // since we read it from last_closure_func_idx instead.
                    out.push(Instruction::Drop);
                    let closure_func_idx = self.last_closure_func_idx
                        .ok_or_else(|| Error::InternalError(
                            "defer: closure function index not set".to_string(),
                        ))?;

                    let env_local = self.last_closure_env;

                    // Build arg_locals for env_ptr + explicit args
                    let mut closure_arg_locals = Vec::new();
                    if let Some(el) = env_local {
                        closure_arg_locals.push((el, ValType::I32));
                    } else {
                        let zero_env = locals.add_local(
                            &format!("__defer_env_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::LocalSet(zero_env));
                        closure_arg_locals.push((zero_env, ValType::I32));
                    }
                    closure_arg_locals.extend(arg_locals);

                    let mut named_return_captures = Vec::new();
                    if let Some(el) = env_local {
                        for cap in &self.last_closure_captures {
                            if let Some((_, vt)) = self.named_returns.iter().find(|(n, _)| *n == cap.name) {
                                if let Some(nr_local) = locals.find(&cap.name) {
                                    named_return_captures.push(NamedReturnCapture {
                                        env_local: el,
                                        env_offset: cap.env_offset,
                                        named_return_local: nr_local,
                                        val_type: *vt,
                                    });
                                }
                            }
                        }
                    }

                    if let Some(deferred) = self.deferred_calls.last_mut() {
                        deferred.push(DeferredCall {
                            func_idx: closure_func_idx,
                            arg_locals: closure_arg_locals,
                            named_return_captures,
                        });
                    }
                    return Ok(());
                }

                let func_idx =
                    if let ast::Expression::Ident(ident) = defer.call.func.as_ref() {
                        self.functions
                            .iter()
                            .find(|f| f.name == ident.name)
                            .map(|f| f.wasm_func_idx)
                    } else if let ast::Expression::Selector(sel) =
                        defer.call.func.as_ref()
                    {
                        if let ast::Expression::Ident(recv_ident) = sel.x.as_ref() {
                            let qname = format!("{}.{}", recv_ident.name, sel.sel.name);
                            let pkg_func = self.functions
                                .iter()
                                .find(|f| f.name == qname)
                                .map(|f| f.wasm_func_idx);

                            if pkg_func.is_some() {
                                pkg_func
                            } else if let Some(type_name) = locals.get_var_struct_type(&recv_ident.name).map(|s| s.to_string()) {
                                let method_qname = format!("{}.{}", type_name, sel.sel.name);
                                let method_func = self.functions
                                    .iter()
                                    .find(|f| f.name == method_qname)
                                    .map(|f| f.wasm_func_idx);
                                if let Some(idx) = method_func {
                                    self.compile_expression(sel.x.as_ref(), out, locals)?;
                                    let recv_temp = locals.add_local(
                                        &format!("__defer_recv_{}", locals.locals.len()),
                                        ValType::I32,
                                    );
                                    out.push(Instruction::LocalSet(recv_temp));
                                    arg_locals.insert(0, (recv_temp, ValType::I32));
                                    Some(idx)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            // Chained selector: e.g., obj.field.Method()
                            // Compile the full receiver expression and resolve the method
                            let recv_type = self.infer_selector_struct_type(sel.x.as_ref(), locals);
                            if let Some(type_name) = recv_type {
                                let method_qname = format!("{}.{}", type_name, sel.sel.name);
                                let method_func = self.functions
                                    .iter()
                                    .find(|f| f.name == method_qname)
                                    .map(|f| f.wasm_func_idx);
                                if let Some(idx) = method_func {
                                    self.compile_expression(sel.x.as_ref(), out, locals)?;
                                    let recv_temp = locals.add_local(
                                        &format!("__defer_recv_{}", locals.locals.len()),
                                        ValType::I32,
                                    );
                                    out.push(Instruction::LocalSet(recv_temp));
                                    arg_locals.insert(0, (recv_temp, ValType::I32));
                                    Some(idx)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                    } else {
                        None
                    };

                match func_idx {
                    Some(idx) => {
                        if let Some(deferred) = self.deferred_calls.last_mut() {
                            deferred.push(DeferredCall {
                                func_idx: idx,
                                arg_locals,
                                named_return_captures: Vec::new(),
                            });
                        }
                        Ok(())
                    }
                    None => Err(Error::InternalError(
                        "defer: could not resolve function".to_string(),
                    )),
                }
            }
            ast::Statement::Range(range) => {
                self.compile_range(range, out, locals, result_types)
            }
            ast::Statement::Empty(_) => Ok(()),
            ast::Statement::Label(labeled) => {
                match labeled.stmt.as_ref() {
                    ast::Statement::For(for_stmt) => {
                        self.compile_for_labeled(
                            for_stmt, out, locals, result_types,
                            Some(labeled.name.name.clone()),
                        )
                    }
                    ast::Statement::Range(range) => {
                        self.compile_range_labeled(
                            range, out, locals, result_types,
                            Some(labeled.name.name.clone()),
                        )
                    }
                    ast::Statement::Switch(sw) => {
                        self.compile_switch_labeled(
                            sw, out, locals, result_types,
                            Some(labeled.name.name.clone()),
                        )
                    }
                    ast::Statement::TypeSwitch(ts) => {
                        self.compile_type_switch_labeled(
                            ts, out, locals, result_types,
                            Some(labeled.name.name.clone()),
                        )
                    }
                    _ => self.compile_statement(&labeled.stmt, out, locals, result_types),
                }
            }
            ast::Statement::Go(_) => Err(Error::InternalError(
                "goroutines not supported in WASM UDFs".to_string(),
            )),
            ast::Statement::Send(_) => Err(Error::InternalError(
                "channel send not supported in WASM UDFs".to_string(),
            )),
            ast::Statement::Select(_) => Err(Error::InternalError(
                "select not supported in WASM UDFs".to_string(),
            )),
            ast::Statement::TypeSwitch(ts) => {
                self.compile_type_switch(ts, out, locals, result_types)
            }
        }
    }

    fn is_iface_go_type(&self, go_type: &str) -> bool {
        go_type == "error" || go_type == "any" || self.iface_defs.contains_key(go_type)
    }

    fn emit_return_iface_box(
        &mut self,
        expr: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        self.compile_expression(expr, out, locals)?;
        let rhs_vt = self.infer_val_type(expr, locals);
        let rhs_type_name = self.infer_concrete_type_name(expr, locals);
        let type_id = self.get_or_create_type_id(&rhs_type_name);
        let (elem_size, _) = Self::elem_size_and_align(rhs_vt);

        let val_tmp = locals.add_local(&format!("__ret_ibox_v_{}", locals.locals.len()), rhs_vt);
        out.push(Instruction::LocalSet(val_tmp));

        let alloc_size = (elem_size as i32).max(8);
        out.push(Instruction::I32Const(alloc_size));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let data_ptr = locals.add_local(&format!("__ret_ibox_d_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalSet(data_ptr));
        out.push(Instruction::LocalGet(data_ptr));
        out.push(Instruction::LocalGet(val_tmp));
        let (_, align) = Self::elem_size_and_align(rhs_vt);
        Self::emit_typed_store(rhs_vt, 0, align, out);

        out.push(Instruction::I32Const(8));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let wrapper = locals.add_local(&format!("__ret_ibox_w_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalSet(wrapper));
        out.push(Instruction::LocalGet(wrapper));
        out.push(Instruction::I32Const(type_id as i32));
        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalGet(wrapper));
        out.push(Instruction::LocalGet(data_ptr));
        out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalGet(wrapper));
        Ok(())
    }

    fn compile_return(
        &mut self,
        ret: &ast::ReturnStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        let has_deferred_named_return_captures = self.deferred_calls.last()
            .map_or(false, |scope| scope.iter().any(|dc| !dc.named_return_captures.is_empty()));

        let result_go_types = self.current_result_go_types.clone();

        // Detect `return f()` where f() returns multiple values
        let is_multi_return_forward = ret.ret.len() == 1
            && result_types.len() > 1
            && matches!(&ret.ret[0], ast::Expression::Call(_))
            && {
                let call_ret = self.call_return_val_types(&ret.ret[0], locals);
                call_ret.len() == result_types.len()
            };

        if ret.ret.is_empty() && !self.named_returns.is_empty() {
            self.emit_deferred_calls(out);
            for (name, _vt) in &self.named_returns.clone() {
                if let Some(idx) = locals.find(name) {
                    out.push(Instruction::LocalGet(idx));
                }
            }
        } else if !self.named_returns.is_empty() && has_deferred_named_return_captures {
            let named_returns_clone = self.named_returns.clone();
            if is_multi_return_forward {
                // `return f()` where f returns multiple values and we have named returns with defer captures.
                // Compile the call, then store each result into named return locals (in reverse stack order).
                self.compile_expression(&ret.ret[0], out, locals)?;
                for (name, _vt) in named_returns_clone.iter().rev() {
                    if let Some(idx) = locals.find(name) {
                        out.push(Instruction::LocalSet(idx));
                    }
                }
            } else {
                for (i, expr) in ret.ret.iter().enumerate() {
                    self.compile_expression(expr, out, locals)?;
                    if let Some(&expected_vt) = result_types.get(i) {
                        let actual_vt = self.infer_val_type(expr, locals);
                        if actual_vt != expected_vt {
                            Self::emit_typed_coerce(actual_vt, expected_vt, out)?;
                        }
                    }
                    if let Some((name, _vt)) = named_returns_clone.get(i) {
                        if let Some(idx) = locals.find(name) {
                            out.push(Instruction::LocalSet(idx));
                        }
                    }
                }
            }
            self.emit_deferred_calls(out);
            for (name, _vt) in &named_returns_clone {
                if let Some(idx) = locals.find(name) {
                    out.push(Instruction::LocalGet(idx));
                }
            }
        } else {
            // When returning a single slice variable but the function signature
            // expects 3 values (ptr, len, cap), expand the header pointer.
            if ret.ret.len() == 1 && result_types.len() == 3 {
                if let ast::Expression::Ident(ident) = &ret.ret[0] {
                    if locals.get_var_struct_type(&ident.name) == Some("__slice") {
                        self.compile_expression(&ret.ret[0], out, locals)?;
                        let hdr = locals.add_local(
                            &format!("__ret_slice_hdr_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::LocalSet(hdr));
                        out.push(Instruction::LocalGet(hdr));
                        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        out.push(Instruction::LocalGet(hdr));
                        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                        out.push(Instruction::LocalGet(hdr));
                        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
                        self.emit_deferred_calls(out);
                        out.push(Instruction::Return);
                        return Ok(());
                    }
                }
            }

            for (i, expr) in ret.ret.iter().enumerate() {
                let is_iface_return = result_go_types.get(i)
                    .map_or(false, |gt| self.is_iface_go_type(gt));
                if is_iface_return && !self.is_interface_var_expr(expr, locals) {
                    self.emit_return_iface_box(expr, out, locals)?;
                } else {
                    self.compile_expression(expr, out, locals)?;
                    if let Some(&expected_vt) = result_types.get(i) {
                        let actual_vt = self.infer_val_type(expr, locals);
                        if actual_vt != expected_vt {
                            Self::emit_typed_coerce(actual_vt, expected_vt, out)?;
                        }
                    }
                }
            }
            self.emit_deferred_calls(out);
        }
        out.push(Instruction::Return);
        Ok(())
    }

    fn compile_assign(
        &mut self,
        assign: &ast::AssignStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let is_define = assign.op == Operator::Define;

        if is_define {
            // Validate short variable declaration rules
            let mut seen_names: Vec<&str> = Vec::new();
            let mut has_new = false;
            for left in &assign.left {
                if let ast::Expression::Ident(ident) = left {
                    if ident.name == "_" {
                        continue;
                    }
                    if seen_names.contains(&ident.name.as_str()) {
                        return Err(Error::SyntaxError(format!(
                            "{} repeated on left side of :=",
                            ident.name
                        )));
                    }
                    seen_names.push(&ident.name);
                    if locals.find_at_current_scope(&ident.name).is_none() {
                        has_new = true;
                    }
                }
            }
            if !seen_names.is_empty() && !has_new {
                return Err(Error::SyntaxError(
                    "no new variables on left side of :=".to_string(),
                ));
            }

            // Map comma-ok: v, ok := m[key]
            if assign.right.len() == 1 && assign.left.len() == 2 {
                if let ast::Expression::Index(idx_expr) = &assign.right[0] {
                    if let Some(left_expr) = idx_expr.left.as_deref() {
                        if let ast::Expression::Ident(map_ident) = left_expr {
                            if locals.get_var_struct_type(&map_ident.name) == Some("__map") {
                                let val_var = if let ast::Expression::Ident(id) = &assign.left[0] {
                                    id.name.clone()
                                } else {
                                    "__comma_ok_val".to_string()
                                };
                                let ok_var = if let ast::Expression::Ident(id) = &assign.left[1] {
                                    id.name.clone()
                                } else {
                                    "__comma_ok_flag".to_string()
                                };
                                let map_name = map_ident.name.clone();
                                let key_expr = idx_expr.index.clone();
                                return self.compile_map_get_ok(&map_name, &key_expr, &val_var, &ok_var, out, locals);
                            }
                        }
                    }
                }
            }

            // Type assertion comma-ok: v, ok := x.(T)
            if assign.right.len() == 1 && assign.left.len() == 2 {
                if let ast::Expression::TypeAssert(ta) = &assign.right[0] {
                    if ta.right.is_some() {
                        let val_var = if let ast::Expression::Ident(id) = &assign.left[0] {
                            id.name.clone()
                        } else {
                            "__ta_val".to_string()
                        };
                        let ok_var = if let ast::Expression::Ident(id) = &assign.left[1] {
                            id.name.clone()
                        } else {
                            "__ta_ok".to_string()
                        };
                        return self.compile_type_assert_ok(ta, &val_var, &ok_var, out, locals);
                    }
                }
            }

            // Multi-return: single function call on the right, multiple vars on the left
            if assign.right.len() == 1 && assign.left.len() > 1 {
                if let ast::Expression::Call(_) = &assign.right[0] {
                    let ret_types = self.call_return_val_types(&assign.right[0], locals);
                    if ret_types.is_empty() {
                        return Err(Error::InternalError(format!(
                            "assignment mismatch: {} variables but function returns no values",
                            assign.left.len(),
                        )));
                    }
                    if ret_types.len() != assign.left.len() {
                        return Err(Error::InternalError(format!(
                            "assignment mismatch: {} variables but function returns {} values",
                            assign.left.len(),
                            ret_types.len()
                        )));
                    }
                    return self.compile_multi_return_define(assign, &ret_types, out, locals);
                }
            }

            for (i, left) in assign.left.iter().enumerate() {
                if let ast::Expression::Ident(ident) = left {
                    if ident.name == "_" {
                        if i < assign.right.len() {
                            self.compile_expression(&assign.right[i], out, locals)?;
                            out.push(Instruction::Drop);
                        }
                        continue;
                    }

                    let vt = if i < assign.right.len() {
                        self.infer_val_type(&assign.right[i], locals)
                    } else {
                        ValType::I32
                    };

                    let local_idx = if let Some(existing) = locals.find_at_current_scope(&ident.name) {
                        existing
                    } else {
                        locals.add_local(&ident.name, vt)
                    };

                    if i < assign.right.len() {
                        // Track struct type from composite literals
                        if let ast::Expression::CompositeLit(comp) = &assign.right[i] {
                            if let ast::Expression::Ident(type_ident) = comp.typ.as_ref()
                            {
                                if let Some(underlying) = self.named_composite_types.get(&type_ident.name).cloned() {
                                    self.setup_named_composite_var(&ident.name, &underlying, locals);
                                } else {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        &type_ident.name,
                                    );
                                }
                            }
                        }

                        // Track type from &expr (address-of)
                        if let ast::Expression::Operation(addr_op) = &assign.right[i] {
                            if addr_op.op == Operator::And && addr_op.y.is_none() {
                                if let ast::Expression::CompositeLit(comp) = &*addr_op.x {
                                    if let ast::Expression::Ident(type_ident) = comp.typ.as_ref() {
                                        locals.set_var_struct_type(
                                            &ident.name,
                                            &type_ident.name,
                                        );
                                    }
                                } else if let ast::Expression::Ident(ref_ident) = &*addr_op.x {
                                    let ptr_tag = if let Some(st) = locals.get_var_struct_type(&ref_ident.name) {
                                        if self.struct_defs.contains_key(st) {
                                            Some(st.to_string())
                                        } else {
                                            None
                                        }
                                    } else {
                                        let vt = self.infer_val_type(&addr_op.x, locals);
                                        Some(match vt {
                                            ValType::I64 => "__ptr_i64".to_string(),
                                            ValType::F32 => "__ptr_f32".to_string(),
                                            ValType::F64 => "__ptr_f64".to_string(),
                                            _ => "__ptr_i32".to_string(),
                                        })
                                    };
                                    if let Some(tag) = ptr_tag {
                                        locals.set_var_struct_type(&ident.name, &tag);
                                    }
                                }
                            }
                        }

                        if self.is_unsigned_expr(&assign.right[i], locals) {
                            locals.unsigned_vars.insert(ident.name.clone());
                        }

                        // Track string variables
                        let is_string = self.is_string_expr(&assign.right[i], locals);
                        if is_string {
                            locals.set_var_struct_type(
                                &ident.name,
                                "__string",
                            );
                            let len_local = locals.add_local(
                                &format!("{}__str_len", ident.name),
                                ValType::I32,
                            );
                            locals.string_locals.insert(
                                ident.name.clone(),
                                (local_idx, len_local),
                            );
                        }

                        // Track slice variables from make/append calls
                        if let ast::Expression::Call(call_expr) = &assign.right[i] {
                            if let ast::Expression::Ident(fn_ident) =
                                call_expr.func.as_ref()
                            {
                                if fn_ident.name == "make" {
                                    if let Some(ast::Expression::TypeMap(map_type)) = call_expr.args.first() {
                                        locals.set_var_struct_type(
                                            &ident.name,
                                            "__map",
                                        );
                                        let (kv, ks, vv, vs, sk, sv, vst) = self.map_key_val_types(map_type);
                                        let nested = self.build_nested_map_type_info(map_type);
                                        locals.map_types.insert(
                                            ident.name.clone(),
                                            MapTypeInfo { key_vt: kv, val_vt: vv, key_size: ks, val_size: vs, is_string_key: sk, is_string_val: sv, val_struct_type: vst, nested_map_val_type: nested },
                                        );
                                    } else {
                                        locals.set_var_struct_type(
                                            &ident.name,
                                            "__slice",
                                        );
                                        let elem_vt =
                                            Self::infer_slice_elem_type(
                                                call_expr.args.first(),
                                            );
                                        locals.slice_elem_types.insert(
                                            ident.name.clone(),
                                            elem_vt,
                                        );
                                        if let Some(ast::Expression::TypeSlice(slice_type)) = call_expr.args.first() {
                                            if let ast::Expression::TypeSlice(inner_st) = slice_type.typ.as_ref() {
                                                let inner_vt = Self::infer_array_elem_vt(&inner_st.typ);
                                                locals.nested_slice_inner_elem_types.insert(ident.name.clone(), inner_vt);
                                            }
                                            if let ast::Expression::Ident(el_id) = slice_type.typ.as_ref() {
                                                if self.struct_defs.contains_key(&el_id.name) {
                                                    locals.slice_elem_struct_types.insert(ident.name.clone(), el_id.name.clone());
                                                }
                                            }
                                        }
                                    }
                                } else if fn_ident.name == "append" {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        "__slice",
                                    );
                                } else if fn_ident.name == "new" {
                                    if let Some(type_arg) = call_expr.args.first() {
                                        if let ast::Expression::Ident(ti) = type_arg {
                                            let ptr_tag = match ti.name.as_str() {
                                                "int" | "int64" | "uint" | "uint64" => "__ptr_i64",
                                                "float32" => "__ptr_f32",
                                                "float64" => "__ptr_f64",
                                                _ => {
                                                    if self.struct_defs.contains_key(&ti.name) {
                                                        &ti.name
                                                    } else {
                                                        "__ptr_i32"
                                                    }
                                                }
                                            };
                                            locals.set_var_struct_type(
                                                &ident.name,
                                                ptr_tag,
                                            );
                                        }
                                    }
                                } else if fn_ident.name == "complex" {
                                    let is_c64 = call_expr.args.first().map_or(false, |a| {
                                        self.infer_val_type(a, locals) == ValType::F32
                                    });
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        if is_c64 { "__complex64" } else { "__complex128" },
                                    );
                                }
                            }

                            // Track []byte(s) and []rune(s) type conversions as slice variables
                            if let ast::Expression::TypeSlice(slice_type) = call_expr.func.as_ref() {
                                locals.set_var_struct_type(&ident.name, "__slice");
                                let elem_vt = Self::infer_array_elem_vt(&slice_type.typ);
                                locals.slice_elem_types.insert(ident.name.clone(), elem_vt);
                                if let ast::Expression::Ident(el_id) = slice_type.typ.as_ref() {
                                    if el_id.name == "rune" || el_id.name == "int32" {
                                        locals.rune_slices.insert(ident.name.clone());
                                    }
                                }
                            }

                            // Track [N]T(slice) slice-to-array conversions
                            if let ast::Expression::TypeArray(arr_type) = call_expr.func.as_ref() {
                                let arr_len = if let ast::Expression::BasicLit(lit) = arr_type.len.as_ref() {
                                    Self::parse_go_int(&lit.value)
                                        .map_err(|e| Error::SyntaxError(e))? as u32
                                } else {
                                    0
                                };
                                let elem_vt = Self::infer_array_elem_vt(&arr_type.typ);
                                locals.set_var_struct_type(&ident.name, "__array");
                                locals.array_info.insert(ident.name.clone(), (elem_vt, arr_len));
                            }
                        }

                        // Track imaginary literal assignments
                        if let ast::Expression::BasicLit(lit) = &assign.right[i] {
                            if lit.kind == LitKind::Imag {
                                locals.set_var_struct_type(
                                    &ident.name,
                                    "__complex128",
                                );
                            }
                        }

                        // Track composite literal assignments
                        if let ast::Expression::CompositeLit(comp) = &assign.right[i] {
                            if let ast::Expression::TypeArray(arr_type) = comp.typ.as_ref() {
                                let arr_len = if let ast::Expression::BasicLit(lit) = arr_type.len.as_ref() {
                                    Self::parse_go_int(&lit.value)
                                        .map_err(|e| Error::SyntaxError(e))? as u32
                                } else if matches!(arr_type.len.as_ref(), ast::Expression::Ellipsis(_)) {
                                    comp.val.values.len() as u32
                                } else { 0 };
                                let elem_vt = Self::infer_array_elem_vt(&arr_type.typ);
                                locals.set_var_struct_type(&ident.name, "__array");
                                locals.array_info.insert(ident.name.clone(), (elem_vt, arr_len));
                            } else if let ast::Expression::TypeSlice(slice_type) = comp.typ.as_ref() {
                                locals.set_var_struct_type(&ident.name, "__slice");
                                let elem_vt = Self::infer_array_elem_vt(&slice_type.typ);
                                locals.slice_elem_types.insert(ident.name.clone(), elem_vt);
                                if let ast::Expression::TypeSlice(inner_st) = slice_type.typ.as_ref() {
                                    let inner_vt = Self::infer_array_elem_vt(&inner_st.typ);
                                    locals.nested_slice_inner_elem_types.insert(ident.name.clone(), inner_vt);
                                }
                                if let ast::Expression::Ident(el_id) = slice_type.typ.as_ref() {
                                    if el_id.name == "rune" || el_id.name == "int32" {
                                        locals.rune_slices.insert(ident.name.clone());
                                    }
                                    if self.struct_defs.contains_key(&el_id.name) {
                                        locals.slice_elem_struct_types.insert(ident.name.clone(), el_id.name.clone());
                                    }
                                }
                            } else if let ast::Expression::TypeMap(map_type) = comp.typ.as_ref() {
                                locals.set_var_struct_type(&ident.name, "__map");
                                let (kv, ks, vv, vs, sk, sv, vst) = self.map_key_val_types(map_type);
                                let nested = self.build_nested_map_type_info(map_type);
                                locals.map_types.insert(
                                    ident.name.clone(),
                                    MapTypeInfo { key_vt: kv, val_vt: vv, key_size: ks, val_size: vs, is_string_key: sk, is_string_val: sv, val_struct_type: vst, nested_map_val_type: nested },
                                );
                            } else if let ast::Expression::Ident(type_ident) = comp.typ.as_ref() {
                                if let Some(underlying) = self.named_composite_types.get(&type_ident.name).cloned() {
                                    self.setup_named_composite_var(&ident.name, &underlying, locals);
                                } else if self.struct_defs.contains_key(&type_ident.name) {
                                    locals.set_var_struct_type(&ident.name, &type_ident.name);
                                }
                            } else if let ast::Expression::Index(idx) = comp.typ.as_ref() {
                                // Generic type instantiation: Pair[int]{...}
                                if let Some(ast::Expression::Ident(type_ident)) = idx.left.as_ref().map(|l| l.as_ref()) {
                                    if self.generic_types.contains_key(&type_ident.name) {
                                        let type_arg = Self::type_expr_to_go_string(&idx.index);
                                        if let Ok(mono_name) = self.monomorphize_generic_type(&type_ident.name, &[type_arg]) {
                                            locals.set_var_struct_type(&ident.name, &mono_name);
                                        }
                                    }
                                }
                            }
                        }

                        // Track reslice assignments: t := s[lo:hi] or t := s[lo:hi:max]
                        if let ast::Expression::Slice(sl) = &assign.right[i] {
                            if !self.is_string_expr(&assign.right[i], locals) {
                                locals.set_var_struct_type(&ident.name, "__slice");
                                if let ast::Expression::Ident(src_ident) = &*sl.left {
                                    if let Some(&evtype) = locals.slice_elem_types.get(&src_ident.name) {
                                        locals.slice_elem_types.insert(ident.name.clone(), evtype);
                                    }
                                    if let Some(&inner_vt) = locals.nested_slice_inner_elem_types.get(&src_ident.name) {
                                        locals.nested_slice_inner_elem_types.insert(ident.name.clone(), inner_vt);
                                    }
                                    if let Some(st) = locals.slice_elem_struct_types.get(&src_ident.name).cloned() {
                                        locals.slice_elem_struct_types.insert(ident.name.clone(), st);
                                    }
                                }
                            }
                        }

                        // Track type assertion results as struct types
                        if let ast::Expression::TypeAssert(ta) = &assign.right[i] {
                            if let Some(ref target_type) = ta.right {
                                if let ast::Expression::Ident(type_id) = target_type.as_ref() {
                                    if self.struct_defs.contains_key(&type_id.name) {
                                        locals.set_var_struct_type(&ident.name, &type_id.name);
                                    }
                                }
                            }
                        }

                        // Track struct type from map or slice index result
                        if let ast::Expression::Index(idx_expr) = &assign.right[i] {
                            if let Some(left_expr) = idx_expr.left.as_deref() {
                                if let ast::Expression::Ident(src_ident) = left_expr {
                                    let map_val_struct = locals.map_types.get(&src_ident.name)
                                        .and_then(|mti| mti.val_struct_type.clone());
                                    if let Some(st) = map_val_struct {
                                        locals.set_var_struct_type(&ident.name, &st);
                                    } else if let Some(st) = locals.slice_elem_struct_types.get(&src_ident.name).cloned() {
                                        locals.set_var_struct_type(&ident.name, &st);
                                    }
                                }
                            }
                        }

                        // Track interface returns from function calls
                        let mut is_iface_from_call = false;
                        if let ast::Expression::Call(call_expr) = &assign.right[i] {
                            if let ast::Expression::Ident(fn_ident) = call_expr.func.as_ref() {
                                if let Some(fi) = self.functions.iter().find(|f| f.name == fn_ident.name).cloned() {
                                    if let Some(go_type) = fi.result_go_types.first() {
                                        if self.is_iface_go_type(go_type) {
                                            is_iface_from_call = true;
                                            let iface_tag = format!("__iface_{}", go_type);
                                            locals.set_var_struct_type(&ident.name, &iface_tag);
                                            let tid_local = locals.add_local(
                                                &format!("{}__type_id", ident.name),
                                                ValType::I32,
                                            );
                                            self.iface_var_type_ids.insert(ident.name.clone(), tid_local);
                                        }
                                    }
                                }
                            } else if let ast::Expression::Selector(sel) = call_expr.func.as_ref() {
                                let method_name = format!("{}.{}", self.infer_concrete_type_name(&sel.x, locals), sel.sel.name);
                                if let Some(fi) = self.functions.iter().find(|f| f.name == method_name).cloned() {
                                    if let Some(go_type) = fi.result_go_types.first() {
                                        if self.is_iface_go_type(go_type) {
                                            is_iface_from_call = true;
                                            let iface_tag = format!("__iface_{}", go_type);
                                            locals.set_var_struct_type(&ident.name, &iface_tag);
                                            let tid_local = locals.add_local(
                                                &format!("{}__type_id", ident.name),
                                                ValType::I32,
                                            );
                                            self.iface_var_type_ids.insert(ident.name.clone(), tid_local);
                                        }
                                    }
                                }
                            }
                        }

                        if let ast::Expression::FuncLit(_) = &assign.right[i] {
                            locals.closure_info.insert(
                                ident.name.clone(),
                                (self.next_func_idx, u32::MAX),
                            );
                        }

                        self.compile_expression(&assign.right[i], out, locals)?;
                        if is_iface_from_call {
                            let wrapper = locals.add_local(
                                &format!("__iface_wrap_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            out.push(Instruction::LocalSet(wrapper));
                            let tid_local = *self.iface_var_type_ids.get(&ident.name).ok_or_else(|| {
                                Error::InternalError(format!(
                                    "interface type-id local not found for '{}'", ident.name
                                ))
                            })?;
                            out.push(Instruction::LocalGet(wrapper));
                            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalSet(tid_local));
                            out.push(Instruction::LocalGet(wrapper));
                            out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalSet(local_idx));
                        } else if is_string {
                            let (ptr_local, len_local) = locals.string_locals[&ident.name];
                            out.push(Instruction::LocalSet(len_local));
                            out.push(Instruction::LocalSet(ptr_local));
                        } else {
                            out.push(Instruction::LocalSet(local_idx));
                        }

                        if let Some(func_idx) = self.last_closure_func_idx.take() {
                            let env_local =
                                self.last_closure_env.take().unwrap_or(u32::MAX);
                            locals
                                .closure_info
                                .insert(ident.name.clone(), (func_idx, env_local));
                            let cap_info: Vec<(String, u32, ValType)> = self.last_closure_captures.iter()
                                .map(|c| (c.name.clone(), c.env_offset, c.val_type))
                                .collect();
                            locals.closure_env_captures.insert(ident.name.clone(), cap_info);
                        }
                        if let Some(func_idx) = self.last_func_value_idx.take() {
                            locals.closure_info.insert(ident.name.clone(), (func_idx, u32::MAX));
                            if self.last_is_method_expr {
                                locals.method_expr_vars.insert(ident.name.clone());
                                self.last_is_method_expr = false;
                            }
                        }
                    }
                }
            }
        } else {
            // Multi-return with =: a, b = func()
            if assign.right.len() == 1 && assign.left.len() > 1 {
                if let ast::Expression::Call(_) = &assign.right[0] {
                    let ret_types = self.call_return_val_types(&assign.right[0], locals);
                    if ret_types.len() == assign.left.len() {
                        return self.compile_multi_return_define(assign, &ret_types, out, locals);
                    }
                }
            }

            // Map comma-ok with =: v, ok = m[key]
            if assign.right.len() == 1 && assign.left.len() == 2 {
                if let ast::Expression::Index(idx_expr) = &assign.right[0] {
                    if let Some(left_expr) = idx_expr.left.as_deref() {
                        if let ast::Expression::Ident(map_ident) = left_expr {
                            if locals.get_var_struct_type(&map_ident.name) == Some("__map") {
                                let val_var = if let ast::Expression::Ident(id) = &assign.left[0] {
                                    id.name.clone()
                                } else {
                                    "__comma_ok_val".to_string()
                                };
                                let ok_var = if let ast::Expression::Ident(id) = &assign.left[1] {
                                    id.name.clone()
                                } else {
                                    "__comma_ok_flag".to_string()
                                };
                                let map_name = map_ident.name.clone();
                                let key_expr = idx_expr.index.clone();
                                return self.compile_map_get_ok(&map_name, &key_expr, &val_var, &ok_var, out, locals);
                            }
                        }
                    }
                }
            }

            let needs_parallel = assign.left.len() > 1
                && assign.right.len() > 1
                && assign.op == Operator::Assign;

            let mut par_temps: Vec<(u32, Option<u32>)> = Vec::new();
            if needs_parallel {
                for right in &assign.right {
                    let is_str = self.is_string_expr(right, locals);
                    self.compile_expression(right, out, locals)?;
                    if is_str {
                        let len_tmp = locals.add_local(
                            &format!("__par_l_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        let ptr_tmp = locals.add_local(
                            &format!("__par_p_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::LocalSet(len_tmp));
                        out.push(Instruction::LocalSet(ptr_tmp));
                        par_temps.push((ptr_tmp, Some(len_tmp)));
                    } else {
                        let vt = self.infer_val_type(right, locals);
                        let tmp = locals.add_local(
                            &format!("__par_{}", locals.locals.len()),
                            vt,
                        );
                        out.push(Instruction::LocalSet(tmp));
                        par_temps.push((tmp, None));
                    }
                }
            }

            for (i, left) in assign.left.iter().enumerate() {
                if needs_parallel {
                    if i < par_temps.len() {
                        let (val_tmp, len_tmp_opt) = par_temps[i];
                        out.push(Instruction::LocalGet(val_tmp));
                        if let Some(len_tmp) = len_tmp_opt {
                            out.push(Instruction::LocalGet(len_tmp));
                        }
                    }
                } else if i < assign.right.len() {
                    if let ast::Expression::Ident(lhs_ident) = left {
                        if let ast::Expression::FuncLit(_) = &assign.right[i] {
                            locals.closure_info.insert(
                                lhs_ident.name.clone(),
                                (self.next_func_idx, u32::MAX),
                            );
                        }
                    }
                    self.compile_expression(&assign.right[i], out, locals)?;
                }

                match left {
                    ast::Expression::Ident(ident) => {
                        if ident.name == "_" {
                            out.push(Instruction::Drop);
                            continue;
                        }
                        // Interface variable assignment: box value and set type_id
                        if self.is_interface_var(&ident.name, locals) {
                            if let Some(tid_local) = self.get_iface_type_id_local(&ident.name) {
                                let data_local = locals.find(&ident.name).ok_or_else(|| {
                                    Error::InternalError(format!("interface var '{}' not found", ident.name))
                                })?;
                                let rhs_vt = if i < assign.right.len() {
                                    self.infer_val_type(&assign.right[i], locals)
                                } else {
                                    ValType::I64
                                };
                                let rhs_type_name = if i < assign.right.len() {
                                    self.infer_concrete_type_name(&assign.right[i], locals)
                                } else {
                                    "int".to_string()
                                };
                                if let Some(st) = locals.get_var_struct_type(&ident.name) {
                                    if let Some(iface_name) = st.strip_prefix("__iface_") {
                                        let iface_name = iface_name.to_string();
                                        if !self.is_interface_var_expr(&assign.right[i], locals) {
                                            self.check_iface_satisfaction(&iface_name, &rhs_type_name)?;
                                        }
                                    }
                                }
                                let type_id = self.get_or_create_type_id(&rhs_type_name);
                                let (elem_size, _) = Self::elem_size_and_align(rhs_vt);
                                self.emit_box_value(rhs_vt, elem_size, type_id, tid_local, data_local, out, locals)?;
                                continue;
                            }
                        }
                        if let Some(&(ptr_local, len_local)) = locals.string_locals.get(&ident.name) {
                            if assign.op == Operator::Assign {
                                out.push(Instruction::LocalSet(len_local));
                                out.push(Instruction::LocalSet(ptr_local));
                            } else if assign.op == Operator::AddAssign {
                                // s += expr: RHS is already compiled on stack as (ptr, len)
                                // We need to concatenate current s with the RHS
                                let rhs_len = locals.add_local(&format!("__sadd_rlen_{}", locals.locals.len()), ValType::I32);
                                let rhs_ptr = locals.add_local(&format!("__sadd_rptr_{}", locals.locals.len()), ValType::I32);
                                out.push(Instruction::LocalSet(rhs_len));
                                out.push(Instruction::LocalSet(rhs_ptr));

                                let total = locals.add_local(&format!("__sadd_total_{}", locals.locals.len()), ValType::I32);
                                out.push(Instruction::LocalGet(len_local));
                                out.push(Instruction::LocalGet(rhs_len));
                                out.push(Instruction::I32Add);
                                out.push(Instruction::LocalSet(total));

                                out.push(Instruction::LocalGet(total));
                                out.push(Instruction::Call(self.alloc_func_idx()?));
                                let new_ptr = locals.add_local(&format!("__sadd_np_{}", locals.locals.len()), ValType::I32);
                                out.push(Instruction::LocalSet(new_ptr));

                                // Copy old string
                                out.push(Instruction::LocalGet(new_ptr));
                                out.push(Instruction::LocalGet(ptr_local));
                                out.push(Instruction::LocalGet(len_local));
                                out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

                                // Copy rhs string
                                out.push(Instruction::LocalGet(new_ptr));
                                out.push(Instruction::LocalGet(len_local));
                                out.push(Instruction::I32Add);
                                out.push(Instruction::LocalGet(rhs_ptr));
                                out.push(Instruction::LocalGet(rhs_len));
                                out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

                                // Update string locals
                                out.push(Instruction::LocalGet(new_ptr));
                                out.push(Instruction::LocalSet(ptr_local));
                                out.push(Instruction::LocalGet(total));
                                out.push(Instruction::LocalSet(len_local));
                            } else {
                                return Err(Error::TypeError(format!(
                                    "operator {:?} not defined for string (only += is valid for string concatenation)",
                                    assign.op
                                )));
                            }
                            continue;
                        }
                        if let Some(cap_info) = self.find_or_add_capture(&ident.name) {
                            let env_offset = cap_info.0 as u64;
                            let vt = cap_info.1;
                            match assign.op {
                                Operator::Assign => {
                                    let tmp = locals.add_local(
                                        &format!("__cap_w_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(0)); // env_ptr
                                    out.push(Instruction::LocalGet(tmp));
                                    match vt {
                                        ValType::I64 => out.push(Instruction::I64Store(MemArg {
                                            offset: env_offset, align: 3, memory_index: 0,
                                        })),
                                        ValType::F64 => out.push(Instruction::F64Store(MemArg {
                                            offset: env_offset, align: 3, memory_index: 0,
                                        })),
                                        ValType::F32 => out.push(Instruction::F32Store(MemArg {
                                            offset: env_offset, align: 2, memory_index: 0,
                                        })),
                                        _ => out.push(Instruction::I32Store(MemArg {
                                            offset: env_offset, align: 2, memory_index: 0,
                                        })),
                                    }
                                }
                                _ => {
                                    let rhs_tmp = locals.add_local(
                                        &format!("__cap_rhs_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(rhs_tmp));
                                    out.push(Instruction::LocalGet(0));
                                    match vt {
                                        ValType::I64 => out.push(Instruction::I64Load(MemArg {
                                            offset: env_offset, align: 3, memory_index: 0,
                                        })),
                                        ValType::F64 => out.push(Instruction::F64Load(MemArg {
                                            offset: env_offset, align: 3, memory_index: 0,
                                        })),
                                        ValType::F32 => out.push(Instruction::F32Load(MemArg {
                                            offset: env_offset, align: 2, memory_index: 0,
                                        })),
                                        _ => out.push(Instruction::I32Load(MemArg {
                                            offset: env_offset, align: 2, memory_index: 0,
                                        })),
                                    }
                                    out.push(Instruction::LocalGet(rhs_tmp));
                                    let arith = match assign.op {
                                        Operator::AddAssign => Self::typed_add(vt),
                                        Operator::SubAssign => Self::typed_sub(vt),
                                        Operator::MulAssign => Self::typed_mul(vt),
                                        _ => Self::typed_add(vt),
                                    };
                                    out.push(arith);
                                    let result_tmp = locals.add_local(
                                        &format!("__cap_res_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(result_tmp));
                                    out.push(Instruction::LocalGet(0));
                                    out.push(Instruction::LocalGet(result_tmp));
                                    match vt {
                                        ValType::I64 => out.push(Instruction::I64Store(MemArg {
                                            offset: env_offset, align: 3, memory_index: 0,
                                        })),
                                        ValType::F64 => out.push(Instruction::F64Store(MemArg {
                                            offset: env_offset, align: 3, memory_index: 0,
                                        })),
                                        ValType::F32 => out.push(Instruction::F32Store(MemArg {
                                            offset: env_offset, align: 2, memory_index: 0,
                                        })),
                                        _ => out.push(Instruction::I32Store(MemArg {
                                            offset: env_offset, align: 2, memory_index: 0,
                                        })),
                                    }
                                }
                            }
                            continue;
                        }
                        if let Some(idx) = locals.find(&ident.name) {
                            let vt = locals
                                .find_type(&ident.name)
                                .unwrap_or(ValType::I64);
                            match assign.op {
                                Operator::Assign => {
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::AddAssign => {
                                    let tmp = locals.add_local(
                                        &format!("__ca_tmp_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(idx));
                                    out.push(Instruction::LocalGet(tmp));
                                    out.push(Self::typed_add(vt));
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::SubAssign => {
                                    let tmp = locals.add_local(
                                        &format!("__ca_tmp_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(idx));
                                    out.push(Instruction::LocalGet(tmp));
                                    out.push(Self::typed_sub(vt));
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::MulAssign => {
                                    let tmp = locals.add_local(
                                        &format!("__ca_tmp_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(idx));
                                    out.push(Instruction::LocalGet(tmp));
                                    out.push(Self::typed_mul(vt));
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::QuoAssign => {
                                    let is_unsigned = locals.unsigned_vars.contains(&ident.name);
                                    let tmp = locals.add_local(
                                        &format!("__ca_tmp_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(idx));
                                    out.push(Instruction::LocalGet(tmp));
                                    out.push(Self::typed_div(vt, is_unsigned));
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::RemAssign
                                | Operator::AndAssign
                                | Operator::OrAssign
                                | Operator::XorAssign
                                | Operator::ShlAssign
                                | Operator::ShrAssign
                                | Operator::AndNotAssign => {
                                    let is_unsigned = locals.unsigned_vars.contains(&ident.name);
                                    let tmp = locals.add_local(
                                        &format!("__ca_tmp_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(idx));
                                    out.push(Instruction::LocalGet(tmp));
                                    self.emit_compound_op(&assign.op, vt, is_unsigned, out)?;
                                    out.push(Instruction::LocalSet(idx));
                                }
                                _ => {
                                    out.push(Instruction::LocalSet(idx));
                                }
                            }
                        } else if let Some(&(global_idx, vt)) = self.global_vars.get(&ident.name) {
                            let len_key = format!("{}_1", ident.name);
                            if let Some(&(len_global_idx, _)) = self.global_vars.get(&len_key) {
                                // String global: stack has (ptr, len)
                                let len_tmp = locals.add_local(
                                    &format!("__gsa_len_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                let ptr_tmp = locals.add_local(
                                    &format!("__gsa_ptr_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(len_tmp));
                                out.push(Instruction::LocalSet(ptr_tmp));
                                out.push(Instruction::LocalGet(ptr_tmp));
                                out.push(Instruction::GlobalSet(global_idx));
                                out.push(Instruction::LocalGet(len_tmp));
                                out.push(Instruction::GlobalSet(len_global_idx));
                            } else {
                                match assign.op {
                                    Operator::Assign => {
                                        out.push(Instruction::GlobalSet(global_idx));
                                    }
                                    _ => {
                                        let is_unsigned = locals.unsigned_vars.contains(&ident.name);
                                        let tmp = locals.add_local(
                                            &format!("__gca_tmp_{}", locals.locals.len()),
                                            vt,
                                        );
                                        out.push(Instruction::LocalSet(tmp));
                                        out.push(Instruction::GlobalGet(global_idx));
                                        out.push(Instruction::LocalGet(tmp));
                                        self.emit_compound_op(&assign.op, vt, is_unsigned, out)?;
                                        out.push(Instruction::GlobalSet(global_idx));
                                    }
                                }
                            }
                        }
                    }
                    ast::Expression::Index(idx_expr) => {
                        // Check if this is a map index assignment
                        if let Some(left_expr) = idx_expr.left.as_deref() {
                            if let ast::Expression::Ident(map_ident) = left_expr {
                                if locals.get_var_struct_type(&map_ident.name) == Some("__map") {
                                    let mti_is_string_val = locals.map_types.get(&map_ident.name)
                                        .map_or(false, |mti| mti.is_string_val);
                                    let rhs_vt = if i < assign.right.len() {
                                        self.infer_val_type(&assign.right[i], locals)
                                    } else {
                                        ValType::I64
                                    };
                                    let val_len_tmp = if mti_is_string_val {
                                        let vl = locals.add_local(
                                            &format!("__midx_rlen_{}", locals.locals.len()),
                                            ValType::I32,
                                        );
                                        out.push(Instruction::LocalSet(vl));
                                        Some(vl)
                                    } else {
                                        None
                                    };
                                    let rhs_tmp = locals.add_local(
                                        &format!("__midx_rhs_{}", locals.locals.len()),
                                        rhs_vt,
                                    );
                                    out.push(Instruction::LocalSet(rhs_tmp));
                                    let map_name = map_ident.name.clone();
                                    let key_expr = idx_expr.index.clone();
                                    self.compile_map_set(&map_name, &key_expr, rhs_tmp, val_len_tmp, rhs_vt, out, locals)?;
                                    continue;
                                }
                            }
                        }

                        let rhs_vt = if i < assign.right.len() {
                            self.infer_val_type(&assign.right[i], locals)
                        } else {
                            ValType::I64
                        };
                        let rhs_tmp = locals.add_local(
                            &format!("__idx_rhs_{}", locals.locals.len()),
                            rhs_vt,
                        );
                        out.push(Instruction::LocalSet(rhs_tmp));

                        let (elem_vt, align) =
                            self.compile_index_store_addr(idx_expr, out, locals)?;

                        let addr_tmp = locals.add_local(
                            &format!("__idx_addr_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::LocalSet(addr_tmp));

                        match assign.op {
                            Operator::Assign => {
                                out.push(Instruction::LocalGet(addr_tmp));
                                out.push(Instruction::LocalGet(rhs_tmp));
                                Self::emit_typed_coerce(rhs_vt, elem_vt, out)?;
                                Self::emit_typed_store(elem_vt, 0, align, out);
                            }
                            _ => {
                                out.push(Instruction::LocalGet(addr_tmp));
                                Self::emit_typed_load(elem_vt, 0, align, out);

                                out.push(Instruction::LocalGet(rhs_tmp));
                                Self::emit_typed_coerce(rhs_vt, elem_vt, out)?;

                                self.emit_compound_op(&assign.op, elem_vt, false, out)?;

                                let result_tmp = locals.add_local(
                                    &format!("__idx_res_{}", locals.locals.len()),
                                    elem_vt,
                                );
                                out.push(Instruction::LocalSet(result_tmp));
                                out.push(Instruction::LocalGet(addr_tmp));
                                out.push(Instruction::LocalGet(result_tmp));
                                Self::emit_typed_store(elem_vt, 0, align, out);
                            }
                        }
                    }
                    ast::Expression::Selector(sel) => {
                        let rhs_vt = if i < assign.right.len() {
                            self.infer_val_type(&assign.right[i], locals)
                        } else {
                            ValType::I64
                        };
                        let rhs_tmp = locals.add_local(
                            &format!("__sel_rhs_{}", locals.locals.len()),
                            rhs_vt,
                        );
                        out.push(Instruction::LocalSet(rhs_tmp));

                        let (offset, field_vt) =
                            self.compile_selector_store_addr(sel, out, locals)?;
                        let (_, align) = Self::elem_size_and_align(field_vt);

                        let addr_tmp = locals.add_local(
                            &format!("__sel_addr_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::LocalSet(addr_tmp));

                        match assign.op {
                            Operator::Assign => {
                                out.push(Instruction::LocalGet(addr_tmp));
                                out.push(Instruction::LocalGet(rhs_tmp));
                                Self::emit_typed_coerce(rhs_vt, field_vt, out)?;
                                Self::emit_typed_store(field_vt, offset, align, out);
                            }
                            _ => {
                                out.push(Instruction::LocalGet(addr_tmp));
                                Self::emit_typed_load(field_vt, offset, align, out);

                                out.push(Instruction::LocalGet(rhs_tmp));
                                Self::emit_typed_coerce(rhs_vt, field_vt, out)?;

                                self.emit_compound_op(&assign.op, field_vt, false, out)?;

                                let result_tmp = locals.add_local(
                                    &format!("__sel_res_{}", locals.locals.len()),
                                    field_vt,
                                );
                                out.push(Instruction::LocalSet(result_tmp));
                                out.push(Instruction::LocalGet(addr_tmp));
                                out.push(Instruction::LocalGet(result_tmp));
                                Self::emit_typed_store(field_vt, offset, align, out);
                            }
                        }
                    }
                    ast::Expression::Star(star) => {
                        let rhs_vt = if i < assign.right.len() {
                            self.infer_val_type(&assign.right[i], locals)
                        } else {
                            ValType::I64
                        };
                        let rhs_tmp = locals.add_local(
                            &format!("__star_rhs_{}", locals.locals.len()),
                            rhs_vt,
                        );
                        out.push(Instruction::LocalSet(rhs_tmp));

                        self.compile_expression(&star.right, out, locals)?;
                        let deref_vt = self.infer_deref_type(&star.right, locals);
                        let (_, align) = Self::elem_size_and_align(deref_vt);

                        out.push(Instruction::LocalGet(rhs_tmp));
                        Self::emit_typed_coerce(rhs_vt, deref_vt, out)?;
                        Self::emit_typed_store(deref_vt, 0, align, out);
                    }
                    ast::Expression::Operation(op) if op.op == Operator::Star && op.y.is_none() => {
                        let rhs_vt = if i < assign.right.len() {
                            self.infer_val_type(&assign.right[i], locals)
                        } else {
                            ValType::I64
                        };
                        let rhs_tmp = locals.add_local(
                            &format!("__deref_rhs_{}", locals.locals.len()),
                            rhs_vt,
                        );
                        out.push(Instruction::LocalSet(rhs_tmp));

                        self.compile_expression(&op.x, out, locals)?;
                        let deref_vt = self.infer_deref_type(&op.x, locals);
                        let (_, align) = Self::elem_size_and_align(deref_vt);

                        out.push(Instruction::LocalGet(rhs_tmp));
                        Self::emit_typed_coerce(rhs_vt, deref_vt, out)?;
                        Self::emit_typed_store(deref_vt, 0, align, out);
                    }
                    _ => {
                        return Err(Error::InternalError(
                            "assignment to unsupported target expression"
                                .to_string(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn typed_add(vt: ValType) -> Instruction<'static> {
        match vt {
            ValType::I32 => Instruction::I32Add,
            ValType::F32 => Instruction::F32Add,
            ValType::F64 => Instruction::F64Add,
            _ => Instruction::I64Add,
        }
    }

    fn typed_sub(vt: ValType) -> Instruction<'static> {
        match vt {
            ValType::I32 => Instruction::I32Sub,
            ValType::F32 => Instruction::F32Sub,
            ValType::F64 => Instruction::F64Sub,
            _ => Instruction::I64Sub,
        }
    }

    fn typed_mul(vt: ValType) -> Instruction<'static> {
        match vt {
            ValType::I32 => Instruction::I32Mul,
            ValType::F32 => Instruction::F32Mul,
            ValType::F64 => Instruction::F64Mul,
            _ => Instruction::I64Mul,
        }
    }

    fn typed_div(vt: ValType, is_unsigned: bool) -> Instruction<'static> {
        match vt {
            ValType::I32 => if is_unsigned { Instruction::I32DivU } else { Instruction::I32DivS },
            ValType::F32 => Instruction::F32Div,
            ValType::F64 => Instruction::F64Div,
            _ => if is_unsigned { Instruction::I64DivU } else { Instruction::I64DivS },
        }
    }

    fn compile_if(
        &mut self,
        if_stmt: &ast::IfStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        let has_init = if_stmt.init.is_some();
        if has_init {
            locals.push_scope();
        }

        if let Some(init) = &if_stmt.init {
            self.compile_statement(init, out, locals, result_types)?;
        }

        self.compile_expression(&if_stmt.cond, out, locals)?;

        out.push(Instruction::If(BlockType::Empty));
        if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
            *depth += 1;
        }
        self.compile_block(&if_stmt.body, out, locals, result_types)?;

        if let Some(else_) = &if_stmt.else_ {
            out.push(Instruction::Else);
            self.compile_statement(else_, out, locals, result_types)?;
        }

        if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
            *depth -= 1;
        }
        out.push(Instruction::End);

        if has_init {
            locals.pop_scope();
        }
        Ok(())
    }

    fn compile_for(
        &mut self,
        for_stmt: &ast::ForStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        self.compile_for_labeled(for_stmt, out, locals, result_types, None)
    }

    fn compile_for_labeled(
        &mut self,
        for_stmt: &ast::ForStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
        label: Option<String>,
    ) -> Result<(), Error> {
        let has_init = for_stmt.init.is_some();
        if has_init {
            locals.push_scope();
        }

        if let Some(init) = &for_stmt.init {
            self.compile_statement(init, out, locals, result_types)?;
        }

        let has_post = for_stmt.post.is_some();

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));

        self.loop_depth.push((label, 0, has_post, true));

        if let Some(cond) = &for_stmt.cond {
            if let ast::Statement::Expr(expr_stmt) = cond.as_ref() {
                self.compile_expression(&expr_stmt.expr, out, locals)?;
                out.push(Instruction::I32Eqz);
                out.push(Instruction::BrIf(1));
            } else {
                return Err(Error::InternalError(format!(
                    "unsupported for-loop condition statement: {:?}",
                    cond
                )));
            }
        }

        if has_post {
            out.push(Instruction::Block(BlockType::Empty));
        }

        self.compile_block(&for_stmt.body, out, locals, result_types)?;

        if has_post {
            out.push(Instruction::End);
        }

        if let Some(post) = &for_stmt.post {
            self.compile_statement(post, out, locals, result_types)?;
        }

        out.push(Instruction::Br(0));
        out.push(Instruction::End);
        out.push(Instruction::End);

        self.loop_depth.pop();

        if has_init {
            locals.pop_scope();
        }

        Ok(())
    }

    fn compile_range(
        &mut self,
        range: &ast::RangeStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        self.compile_range_labeled(range, out, locals, result_types, None)
    }

    fn compile_range_labeled(
        &mut self,
        range: &ast::RangeStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
        label: Option<String>,
    ) -> Result<(), Error> {
        // Reject range over a bare function name (Go 1.23+ iterator protocol).
        // NOTE: this only catches local functions by name; qualified iterators
        // (e.g. iter.Seq) are not detected here because Go 1.23 range-over-func
        // is out of scope for this compiler.
        if let ast::Expression::Ident(fn_ident) = &range.expr {
            if self.functions.iter().any(|f| f.name == fn_ident.name && f.recv_type.is_none()) {
                return Err(Error::SyntaxError(format!(
                    "range over function ('for range {}') is not supported; range-over-function iterators (Go 1.23+) are not implemented",
                    fn_ident.name
                )));
            }
        }

        // Check for map range
        if let ast::Expression::Ident(map_ident) = &range.expr {
            if locals.get_var_struct_type(&map_ident.name) == Some("__map") {
                return self.compile_range_map(range, &map_ident.name.clone(), out, locals, result_types, label);
            }
        }
        if let ast::Expression::Selector(sel) = &range.expr {
            if self.is_selector_map_field(sel, locals) {
                if let Some(parent_type) = self.infer_struct_type_from_expr(sel.x.as_ref(), locals) {
                    if let Some(mti) = self.struct_field_map_types.get(&(parent_type, sel.sel.name.clone())).cloned() {
                        self.compile_expression(&range.expr, out, locals)?;
                        let tmp_name = format!("__range_map_tmp_{}", locals.locals.len());
                        let tmp_local = locals.add_local(&tmp_name, ValType::I32);
                        out.push(Instruction::LocalSet(tmp_local));
                        locals.set_var_struct_type(&tmp_name, "__map");
                        locals.map_types.insert(tmp_name.clone(), mti);
                        return self.compile_range_map(range, &tmp_name, out, locals, result_types, label);
                    }
                }
            }
        }

        let idx_local = locals.add_local("__range_idx", ValType::I32);
        let len_local = locals.add_local("__range_len", ValType::I32);
        let base_ptr_local = locals.add_local("__range_base", ValType::I32);

        // Check if the range expression is a slice header variable
        let is_slice_header = if let ast::Expression::Ident(ident) = &range.expr {
            locals.get_var_struct_type(&ident.name) == Some("__slice")
        } else if let ast::Expression::Selector(sel) = &range.expr {
            self.is_selector_slice_field(sel, locals)
        } else {
            false
        };

        // Check if range is over an array variable
        let array_info = if let ast::Expression::Ident(ident) = &range.expr {
            locals.array_info.get(&ident.name).copied()
        } else {
            None
        };

        let is_string_range_early = self.is_string_expr(&range.expr, locals);

        self.compile_expression(&range.expr, out, locals)?;

        if let Some((_arr_elem_vt, arr_len)) = array_info {
            // Array variable: pushes data pointer (1 value), length is compile-time
            out.push(Instruction::LocalSet(base_ptr_local));
            out.push(Instruction::I32Const(arr_len as i32));
            out.push(Instruction::LocalSet(len_local));
        } else if is_slice_header {
            // Slice header pointer: load data_ptr and len from header
            let hdr_tmp = locals.add_local(
                &format!("__range_hdr_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalSet(hdr_tmp));
            out.push(Instruction::LocalGet(hdr_tmp));
            out.push(Instruction::I32Load(MemArg {
                offset: 4,
                align: 2,
                memory_index: 0,
            }));
            out.push(Instruction::LocalSet(len_local));
            out.push(Instruction::LocalGet(hdr_tmp));
            out.push(Instruction::I32Load(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
            out.push(Instruction::LocalSet(base_ptr_local));
        } else if is_string_range_early {
            // String variable pushes (ptr, len) – always 2 values
            out.push(Instruction::LocalSet(len_local));
            out.push(Instruction::LocalSet(base_ptr_local));
        } else {
            // Check how many values the expression pushed
            let expr_count = self.expression_result_count(&range.expr, Some(locals));
            if expr_count >= 3 {
                // Slice: (ptr, len, cap) -> store cap, len, ptr
                out.push(Instruction::Drop); // cap
                out.push(Instruction::LocalSet(len_local));
                out.push(Instruction::LocalSet(base_ptr_local));
            } else if expr_count == 2 {
                // (ptr, len) or (something, something)
                out.push(Instruction::LocalSet(len_local));
                out.push(Instruction::LocalSet(base_ptr_local));
            } else {
                // Single value: assume it's a count (integer range)
                // Wrap to i32 if the expression pushed i64
                let range_vt = self.infer_val_type(&range.expr, locals);
                if range_vt == ValType::I64 {
                    out.push(Instruction::I32WrapI64);
                }
                // Clamp negative values to 0 so the loop runs zero iterations
                let raw_count = locals.add_local(
                    &format!("__range_raw_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalSet(raw_count));
                out.push(Instruction::LocalGet(raw_count));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalGet(raw_count));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::I32GtS);
                out.push(Instruction::Select);
                out.push(Instruction::LocalSet(len_local));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(base_ptr_local));
            }
        }

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(idx_local));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));

        self.loop_depth.push((label, 0, false, true));

        out.push(Instruction::LocalGet(idx_local));
        out.push(Instruction::LocalGet(len_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        // For string ranges, decode UTF-8 runes and compute rune width
        let mut rune_width_local: Option<u32> = None;
        let mut rune_val_local: Option<u32> = None;
        if is_string_range_early {
            let byte0 = locals.add_local("__utf8_byte0", ValType::I32);
            let addr = locals.add_local("__utf8_addr", ValType::I32);
            let rw = locals.add_local("__rune_width", ValType::I32);
            let rv = locals.add_local("__rune_val", ValType::I32);
            rune_width_local = Some(rw);
            rune_val_local = Some(rv);

            let mem0 = MemArg { offset: 0, align: 0, memory_index: 0 };

            // addr = base_ptr + idx
            out.push(Instruction::LocalGet(base_ptr_local));
            out.push(Instruction::LocalGet(idx_local));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(addr));

            // byte0 = mem[addr]
            out.push(Instruction::LocalGet(addr));
            out.push(Instruction::I32Load8U(mem0));
            out.push(Instruction::LocalSet(byte0));

            // Default: width=1, rune=byte0
            out.push(Instruction::I32Const(1));
            out.push(Instruction::LocalSet(rw));
            out.push(Instruction::LocalGet(byte0));
            out.push(Instruction::LocalSet(rv));

            // if byte0 >= 0x80 (multi-byte)
            out.push(Instruction::LocalGet(byte0));
            out.push(Instruction::I32Const(0x80));
            out.push(Instruction::I32GeU);
            out.push(Instruction::If(BlockType::Empty));
            {
                // 2-byte: (byte0 & 0xE0) == 0xC0
                out.push(Instruction::LocalGet(byte0));
                out.push(Instruction::I32Const(0xE0));
                out.push(Instruction::I32And);
                out.push(Instruction::I32Const(0xC0));
                out.push(Instruction::I32Eq);
                out.push(Instruction::If(BlockType::Empty));
                {
                    // Boundary check: need idx + 2 <= len
                    out.push(Instruction::LocalGet(idx_local));
                    out.push(Instruction::I32Const(2));
                    out.push(Instruction::I32Add);
                    out.push(Instruction::LocalGet(len_local));
                    out.push(Instruction::I32LeU);
                    out.push(Instruction::If(BlockType::Empty));
                    {
                        out.push(Instruction::I32Const(2));
                        out.push(Instruction::LocalSet(rw));
                        out.push(Instruction::LocalGet(byte0));
                        out.push(Instruction::I32Const(0x1F));
                        out.push(Instruction::I32And);
                        out.push(Instruction::I32Const(6));
                        out.push(Instruction::I32Shl);
                        out.push(Instruction::LocalGet(addr));
                        out.push(Instruction::I32Load8U(MemArg { offset: 1, align: 0, memory_index: 0 }));
                        out.push(Instruction::I32Const(0x3F));
                        out.push(Instruction::I32And);
                        out.push(Instruction::I32Or);
                        out.push(Instruction::LocalSet(rv));
                    }
                    out.push(Instruction::Else);
                    {
                        out.push(Instruction::I32Const(0xFFFD));
                        out.push(Instruction::LocalSet(rv));
                    }
                    out.push(Instruction::End);
                }
                out.push(Instruction::Else);
                {
                    // 3-byte: (byte0 & 0xF0) == 0xE0
                    out.push(Instruction::LocalGet(byte0));
                    out.push(Instruction::I32Const(0xF0));
                    out.push(Instruction::I32And);
                    out.push(Instruction::I32Const(0xE0));
                    out.push(Instruction::I32Eq);
                    out.push(Instruction::If(BlockType::Empty));
                    {
                        // Boundary check: need idx + 3 <= len
                        out.push(Instruction::LocalGet(idx_local));
                        out.push(Instruction::I32Const(3));
                        out.push(Instruction::I32Add);
                        out.push(Instruction::LocalGet(len_local));
                        out.push(Instruction::I32LeU);
                        out.push(Instruction::If(BlockType::Empty));
                        {
                            out.push(Instruction::I32Const(3));
                            out.push(Instruction::LocalSet(rw));
                            out.push(Instruction::LocalGet(byte0));
                            out.push(Instruction::I32Const(0x0F));
                            out.push(Instruction::I32And);
                            out.push(Instruction::I32Const(12));
                            out.push(Instruction::I32Shl);
                            out.push(Instruction::LocalGet(addr));
                            out.push(Instruction::I32Load8U(MemArg { offset: 1, align: 0, memory_index: 0 }));
                            out.push(Instruction::I32Const(0x3F));
                            out.push(Instruction::I32And);
                            out.push(Instruction::I32Const(6));
                            out.push(Instruction::I32Shl);
                            out.push(Instruction::I32Or);
                            out.push(Instruction::LocalGet(addr));
                            out.push(Instruction::I32Load8U(MemArg { offset: 2, align: 0, memory_index: 0 }));
                            out.push(Instruction::I32Const(0x3F));
                            out.push(Instruction::I32And);
                            out.push(Instruction::I32Or);
                            out.push(Instruction::LocalSet(rv));
                        }
                        out.push(Instruction::Else);
                        {
                            out.push(Instruction::I32Const(0xFFFD));
                            out.push(Instruction::LocalSet(rv));
                        }
                        out.push(Instruction::End);
                    }
                    out.push(Instruction::Else);
                    {
                        // 4-byte: (byte0 & 0xF8) == 0xF0
                        out.push(Instruction::LocalGet(byte0));
                        out.push(Instruction::I32Const(0xF8));
                        out.push(Instruction::I32And);
                        out.push(Instruction::I32Const(0xF0));
                        out.push(Instruction::I32Eq);
                        out.push(Instruction::If(BlockType::Empty));
                        {
                            // Boundary check: need idx + 4 <= len
                            out.push(Instruction::LocalGet(idx_local));
                            out.push(Instruction::I32Const(4));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalGet(len_local));
                            out.push(Instruction::I32LeU);
                            out.push(Instruction::If(BlockType::Empty));
                            {
                                out.push(Instruction::I32Const(4));
                                out.push(Instruction::LocalSet(rw));
                                out.push(Instruction::LocalGet(byte0));
                                out.push(Instruction::I32Const(0x07));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Const(18));
                                out.push(Instruction::I32Shl);
                                out.push(Instruction::LocalGet(addr));
                                out.push(Instruction::I32Load8U(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Const(12));
                                out.push(Instruction::I32Shl);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::LocalGet(addr));
                                out.push(Instruction::I32Load8U(MemArg { offset: 2, align: 0, memory_index: 0 }));
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Const(6));
                                out.push(Instruction::I32Shl);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::LocalGet(addr));
                                out.push(Instruction::I32Load8U(MemArg { offset: 3, align: 0, memory_index: 0 }));
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::LocalSet(rv));
                            }
                            out.push(Instruction::Else);
                            {
                                out.push(Instruction::I32Const(0xFFFD));
                                out.push(Instruction::LocalSet(rv));
                            }
                            out.push(Instruction::End);
                        }
                        out.push(Instruction::Else);
                        {
                            // Invalid leading byte: U+FFFD replacement, width=1
                            out.push(Instruction::I32Const(1));
                            out.push(Instruction::LocalSet(rw));
                            out.push(Instruction::I32Const(0xFFFD));
                            out.push(Instruction::LocalSet(rv));
                        }
                        out.push(Instruction::End); // end 4-byte check
                    }
                    out.push(Instruction::End); // end 3-byte check
                }
                out.push(Instruction::End); // end 2-byte check
            }
            out.push(Instruction::End); // end multi-byte check
        }

        if let Some(key) = &range.key {
            if let ast::Expression::Ident(ident) = key {
                if ident.name != "_" {
                    let key_local = if range
                        .op
                        .as_ref()
                        .map_or(false, |(_, op)| *op == Operator::Define)
                    {
                        locals.add_local(&ident.name, ValType::I64)
                    } else {
                        locals
                            .find(&ident.name)
                            .unwrap_or_else(|| locals.add_local(&ident.name, ValType::I64))
                    };
                    out.push(Instruction::LocalGet(idx_local));
                    out.push(Instruction::I64ExtendI32S);
                    out.push(Instruction::LocalSet(key_local));
                }
            }
        }

        if let Some(value) = &range.value {
            if let ast::Expression::Ident(ident) = value {
                if ident.name != "_" {
                    if is_string_range_early {
                        // String range: value is the decoded rune
                        let value_local = if range.op.as_ref().map_or(false, |(_, op)| *op == Operator::Define) {
                            locals.add_local(&ident.name, ValType::I32)
                        } else {
                            locals.find(&ident.name).unwrap_or_else(|| locals.add_local(&ident.name, ValType::I32))
                        };
                        if let Some(rv) = rune_val_local {
                            out.push(Instruction::LocalGet(rv));
                            out.push(Instruction::LocalSet(value_local));
                        }
                    } else {
                        let elem_vt = if let ast::Expression::Ident(range_ident) = &range.expr {
                            if let Some(&(arr_evtype, _)) = locals.array_info.get(&range_ident.name) {
                                arr_evtype
                            } else {
                                locals
                                    .slice_elem_types
                                    .get(&range_ident.name)
                                    .copied()
                                    .unwrap_or(ValType::I64)
                            }
                        } else {
                            ValType::I64
                        };

                        let value_local = if range
                            .op
                            .as_ref()
                            .map_or(false, |(_, op)| *op == Operator::Define)
                        {
                            locals.add_local(&ident.name, elem_vt)
                        } else {
                            locals.find(&ident.name).unwrap_or_else(|| {
                                locals.add_local(&ident.name, elem_vt)
                            })
                        };

                        let (elem_size, align) = match elem_vt {
                            ValType::I32 | ValType::F32 => (4i32, 2u32),
                            _ => (8i32, 3u32),
                        };

                        out.push(Instruction::LocalGet(base_ptr_local));
                        out.push(Instruction::LocalGet(idx_local));
                        out.push(Instruction::I32Const(elem_size));
                        out.push(Instruction::I32Mul);
                        out.push(Instruction::I32Add);
                        match elem_vt {
                            ValType::I32 => out.push(Instruction::I32Load(MemArg {
                                offset: 0,
                                align,
                                memory_index: 0,
                            })),
                            ValType::F32 => out.push(Instruction::F32Load(MemArg {
                                offset: 0,
                                align,
                                memory_index: 0,
                            })),
                            ValType::F64 => out.push(Instruction::F64Load(MemArg {
                                offset: 0,
                                align,
                                memory_index: 0,
                            })),
                            _ => out.push(Instruction::I64Load(MemArg {
                                offset: 0,
                                align,
                                memory_index: 0,
                            })),
                        }
                        out.push(Instruction::LocalSet(value_local));
                    }
                }
            }
        }

        // Increment index BEFORE body so that `continue` (Br(0) to Loop start)
        // doesn't skip the increment and cause an infinite loop.
        if let Some(rw) = rune_width_local {
            // String range: advance by rune byte-width
            out.push(Instruction::LocalGet(idx_local));
            out.push(Instruction::LocalGet(rw));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(idx_local));
        } else {
            out.push(Instruction::LocalGet(idx_local));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(idx_local));
        }

        self.compile_block(&range.body, out, locals, result_types)?;

        out.push(Instruction::Br(0));
        out.push(Instruction::End);
        out.push(Instruction::End);

        self.loop_depth.pop();

        Ok(())
    }

    fn infer_concrete_type_name(&self, expr: &ast::Expression, locals: &LocalAlloc) -> String {
        match expr {
            ast::Expression::BasicLit(lit) => {
                use crate::parser::token::LitKind;
                match lit.kind {
                    LitKind::Integer => "int".to_string(),
                    LitKind::Float => "float64".to_string(),
                    LitKind::String => "string".to_string(),
                    LitKind::Imag => "complex128".to_string(),
                    _ => "int".to_string(),
                }
            }
            ast::Expression::Ident(ident) => {
                if let Some(st) = locals.get_var_struct_type(&ident.name) {
                    if st == "__string" {
                        return "string".to_string();
                    }
                    if st.starts_with("__") {
                        return "int".to_string();
                    }
                    return st.to_string();
                }
                if let Some(vt) = locals.find_type(&ident.name) {
                    return Self::type_id_for_val_type(vt).to_string();
                }
                "int".to_string()
            }
            ast::Expression::CompositeLit(comp) => {
                if let ast::Expression::Ident(type_ident) = comp.typ.as_ref() {
                    type_ident.name.clone()
                } else {
                    "int".to_string()
                }
            }
            ast::Expression::Operation(op) if op.y.is_none() && op.op == Operator::And => {
                self.infer_concrete_type_name(&op.x, locals)
            }
            _ => {
                let vt = self.infer_val_type(expr, locals);
                Self::type_id_for_val_type(vt).to_string()
            }
        }
    }

    fn emit_box_value(
        &mut self,
        val_vt: ValType,
        elem_size: i32,
        type_id: u32,
        tid_local: u32,
        data_local: u32,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        // Value is on top of the stack; save it to a temp
        let tmp = locals.add_local(&format!("__box_tmp_{}", locals.locals.len()), val_vt);
        out.push(Instruction::LocalSet(tmp));

        // Allocate memory for the value
        let alloc_size = (elem_size as i32).max(8);
        out.push(Instruction::I32Const(alloc_size));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        out.push(Instruction::LocalSet(data_local));

        // Store value at allocated address
        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::LocalGet(tmp));
        let (_, align) = Self::elem_size_and_align(val_vt);
        Self::emit_typed_store(val_vt, 0, align, out);

        // Set type_id
        out.push(Instruction::I32Const(type_id as i32));
        out.push(Instruction::LocalSet(tid_local));

        Ok(())
    }

    fn check_interface_nil_cmp(
        &self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        locals: &LocalAlloc,
    ) -> (String, bool) {
        // Check: lhs is interface var, rhs is nil (or vice versa)
        if let ast::Expression::Ident(id) = lhs {
            if self.is_interface_var(&id.name, locals) {
                if let ast::Expression::Ident(rhs_id) = rhs {
                    if rhs_id.name == "nil" {
                        return (id.name.clone(), true);
                    }
                }
            }
        }
        if let ast::Expression::Ident(id) = rhs {
            if self.is_interface_var(&id.name, locals) {
                if let ast::Expression::Ident(lhs_id) = lhs {
                    if lhs_id.name == "nil" {
                        return (id.name.clone(), true);
                    }
                }
            }
        }
        (String::new(), false)
    }

    fn is_interface_var(&self, name: &str, locals: &LocalAlloc) -> bool {
        if let Some(st) = locals.get_var_struct_type(name) {
            st == "__interface" || st.starts_with("__iface_")
        } else {
            false
        }
    }

    fn is_interface_var_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> bool {
        if let ast::Expression::Ident(id) = expr {
            self.is_interface_var(&id.name, locals)
        } else {
            false
        }
    }

    fn get_iface_type_id_local(&self, name: &str) -> Option<u32> {
        self.iface_var_type_ids.get(name).copied()
    }

    fn types_implementing_interface(&self, iface_name: &str) -> Vec<u32> {
        let iface_methods = match self.iface_defs.get(iface_name) {
            Some(methods) => methods,
            None => return Vec::new(),
        };
        let iface_sigs = self.iface_method_sigs.get(iface_name);

        let mut result = Vec::new();
        for (type_name, &type_id) in &self.type_registry {
            if type_name == "nil" || self.iface_defs.contains_key(type_name) {
                continue;
            }
            let has_all_methods = iface_methods.iter().all(|method| {
                let qualified = format!("{}.{}", type_name, method);
                if let Some(f) = self.functions.iter().find(|f| f.name == qualified) {
                    if let Some(sigs) = iface_sigs {
                        if let Some((expected_params, expected_results)) = sigs.get(method) {
                            let impl_params: Vec<WasmType> = f.params.iter()
                                .skip(1) // skip receiver
                                .map(|(_, wt)| *wt)
                                .collect();
                            let impl_results: Vec<WasmType> = f.results.clone();
                            return impl_params == *expected_params && impl_results == *expected_results;
                        }
                    }
                    true
                } else {
                    false
                }
            });
            if has_all_methods {
                result.push(type_id);
            }
        }
        result
    }

    fn check_iface_satisfaction(&self, iface_name: &str, concrete_type: &str) -> Result<(), Error> {
        if iface_name == "any" || concrete_type == "nil" {
            return Ok(());
        }
        let iface_methods = match self.iface_defs.get(iface_name) {
            Some(methods) => methods,
            None => return Ok(()),
        };
        let iface_sigs = self.iface_method_sigs.get(iface_name);
        for method in iface_methods {
            let qualified = format!("{}.{}", concrete_type, method);
            if let Some(f) = self.functions.iter().find(|f| f.name == qualified) {
                if let Some(sigs) = iface_sigs {
                    if let Some((expected_params, expected_results)) = sigs.get(method) {
                        let impl_params: Vec<WasmType> = f.params.iter()
                            .skip(1)
                            .map(|(_, wt)| *wt)
                            .collect();
                        if impl_params != *expected_params || f.results != *expected_results {
                            return Err(Error::TypeError(format!(
                                "type '{}' does not implement interface '{}': method '{}' has wrong signature",
                                concrete_type, iface_name, method
                            )));
                        }
                    }
                }
            } else {
                return Err(Error::TypeError(format!(
                    "type '{}' does not implement interface '{}': missing method '{}'",
                    concrete_type, iface_name, method
                )));
            }
        }
        Ok(())
    }

    fn compile_type_assert(
        &mut self,
        ta: &ast::TypeAssertion,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        // x.(T) -- single-value form (panics on mismatch)
        let target_type = ta.right.as_ref().ok_or_else(|| {
            Error::InternalError("type assertion without target type".to_string())
        })?;

        let target_type_name = Self::extract_type_name_from_expr(target_type).ok_or_else(|| {
            Error::InternalError("type assertion target must be a named type".to_string())
        })?;

        // Get the interface variable -- either from a named var or by compiling the expression
        let (tid_local, data_local) = if let ast::Expression::Ident(ident) = ta.left.as_ref() {
            let tid = self.get_iface_type_id_local(&ident.name).ok_or_else(|| {
                Error::InternalError(format!("'{}' is not an interface variable", ident.name))
            })?;
            let data = locals.find(&ident.name).ok_or_else(|| {
                Error::InternalError(format!("variable '{}' not found", ident.name))
            })?;
            (tid, data)
        } else {
            self.compile_expression(&ta.left, out, locals)?;
            let wrapper = locals.add_local(
                &format!("__ta_wrap_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalSet(wrapper));

            let tid = locals.add_local(
                &format!("__ta_tid_{}", locals.locals.len()),
                ValType::I32,
            );
            let data = locals.add_local(
                &format!("__ta_data_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalGet(wrapper));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(tid));
            out.push(Instruction::LocalGet(wrapper));
            out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(data));
            (tid, data)
        };

        let lookup_type_name = target_type_name.strip_prefix('*').unwrap_or(&target_type_name).to_string();

        // Check if target is an interface type
        if self.iface_defs.contains_key(&lookup_type_name)
            || lookup_type_name == "any"
            || lookup_type_name == "error"
        {
            if lookup_type_name == "any" {
                // any always succeeds: pass through the interface value
                out.push(Instruction::LocalGet(data_local));
                return Ok(());
            }

            let valid_type_ids = self.types_implementing_interface(&lookup_type_name);
            if valid_type_ids.is_empty() {
                out.push(Instruction::Unreachable);
                return Ok(());
            }

            // Check if type_id matches any implementing type
            let match_local = locals.add_local(
                &format!("__ta_imatch_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(match_local));

            for tid in &valid_type_ids {
                out.push(Instruction::LocalGet(tid_local));
                out.push(Instruction::I32Const(*tid as i32));
                out.push(Instruction::I32Eq);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(match_local));
                out.push(Instruction::End);
            }

            out.push(Instruction::LocalGet(match_local));
            out.push(Instruction::I32Eqz);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);

            // Pass through the data pointer (interface value stays as-is)
            out.push(Instruction::LocalGet(data_local));
            return Ok(());
        }

        let target_id = self.get_or_create_type_id(&lookup_type_name);
        let target_vt = Self::val_type_for_type_name(&target_type_name);

        // Check type_id matches target
        out.push(Instruction::LocalGet(tid_local));
        out.push(Instruction::I32Const(target_id as i32));
        out.push(Instruction::I32Ne);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::Unreachable); // panic on mismatch
        out.push(Instruction::End);

        // Load value from data_ptr
        out.push(Instruction::LocalGet(data_local));
        let (_, align) = Self::elem_size_and_align(target_vt);
        Self::emit_typed_load(target_vt, 0, align, out);

        Ok(())
    }

    fn compile_type_assert_ok(
        &mut self,
        ta: &ast::TypeAssertion,
        val_var: &str,
        ok_var: &str,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let target_type = ta.right.as_ref().ok_or_else(|| {
            Error::InternalError("type assertion without target type".to_string())
        })?;

        let target_type_name = Self::extract_type_name_from_expr(target_type).ok_or_else(|| {
            Error::InternalError("type assertion target must be a named type".to_string())
        })?;

        let (tid_local, data_local) = if let ast::Expression::Ident(ident) = ta.left.as_ref() {
            let tid = self.get_iface_type_id_local(&ident.name).ok_or_else(|| {
                Error::InternalError(format!("'{}' is not an interface variable", ident.name))
            })?;
            let data = locals.find(&ident.name).ok_or_else(|| {
                Error::InternalError(format!("variable '{}' not found", ident.name))
            })?;
            (tid, data)
        } else {
            self.compile_expression(&ta.left, out, locals)?;
            let wrapper = locals.add_local(
                &format!("__taok_wrap_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalSet(wrapper));

            let tid = locals.add_local(
                &format!("__taok_tid_{}", locals.locals.len()),
                ValType::I32,
            );
            let data = locals.add_local(
                &format!("__taok_data_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalGet(wrapper));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(tid));
            out.push(Instruction::LocalGet(wrapper));
            out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(data));
            (tid, data)
        };

        let lookup_type_name = target_type_name.strip_prefix('*').unwrap_or(&target_type_name).to_string();

        // Check if target is an interface type
        if self.iface_defs.contains_key(&lookup_type_name)
            || lookup_type_name == "any"
            || lookup_type_name == "error"
        {
            let val_local = locals.add_local(val_var, ValType::I32);
            let ok_local = locals.add_local(ok_var, ValType::I32);
            locals.set_var_struct_type(val_var, "__interface");

            // Create a type_id local for the result interface variable
            let val_tid_local = locals.add_local(
                &format!("{}__iface_tid", val_var),
                ValType::I32,
            );

            if lookup_type_name == "any" {
                // any always succeeds
                out.push(Instruction::LocalGet(data_local));
                out.push(Instruction::LocalSet(val_local));
                out.push(Instruction::LocalGet(tid_local));
                out.push(Instruction::LocalSet(val_tid_local));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(ok_local));
                self.iface_var_type_ids.insert(val_var.to_string(), val_tid_local);
                return Ok(());
            }

            let valid_type_ids = self.types_implementing_interface(&lookup_type_name);

            let match_local = locals.add_local(
                &format!("__taok_imatch_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(match_local));

            for tid in &valid_type_ids {
                out.push(Instruction::LocalGet(tid_local));
                out.push(Instruction::I32Const(*tid as i32));
                out.push(Instruction::I32Eq);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(match_local));
                out.push(Instruction::End);
            }

            out.push(Instruction::LocalGet(match_local));
            out.push(Instruction::If(BlockType::Empty));
            {
                out.push(Instruction::LocalGet(data_local));
                out.push(Instruction::LocalSet(val_local));
                out.push(Instruction::LocalGet(tid_local));
                out.push(Instruction::LocalSet(val_tid_local));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(ok_local));
            }
            out.push(Instruction::End);

            self.iface_var_type_ids.insert(val_var.to_string(), val_tid_local);

            return Ok(());
        }

        let target_id = self.get_or_create_type_id(&lookup_type_name);
        let target_vt = Self::val_type_for_type_name(&target_type_name);

        let val_local = locals.add_local(val_var, target_vt);
        let ok_local = locals.add_local(ok_var, ValType::I32);

        // Check type_id matches target
        out.push(Instruction::LocalGet(tid_local));
        out.push(Instruction::I32Const(target_id as i32));
        out.push(Instruction::I32Eq);
        out.push(Instruction::If(BlockType::Empty));
        {
            // Match: load value and set ok=1
            out.push(Instruction::LocalGet(data_local));
            let (_, align) = Self::elem_size_and_align(target_vt);
            Self::emit_typed_load(target_vt, 0, align, out);
            out.push(Instruction::LocalSet(val_local));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::LocalSet(ok_local));
        }
        out.push(Instruction::End);
        // If no match, val_local stays zero-initialized, ok_local stays 0

        Ok(())
    }

    fn compile_type_switch_labeled(
        &mut self,
        ts: &ast::TypeSwitchStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
        label: Option<String>,
    ) -> Result<(), Error> {
        if label.is_some() {
            out.push(Instruction::Block(BlockType::Empty));
            self.loop_depth.push((label, 0, false, false));
        }
        let r = self.compile_type_switch(ts, out, locals, result_types);
        if self.loop_depth.last().map_or(false, |e| !e.3 && e.0.is_some()) {
            self.loop_depth.pop();
            out.push(Instruction::End);
        }
        r
    }

    fn compile_type_switch(
        &mut self,
        ts: &ast::TypeSwitchStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        for clause in &ts.block.body {
            if clause.body.iter().any(|s| Self::contains_fallthrough(s)) {
                return Err(Error::SyntaxError(
                    "cannot fallthrough in type switch".to_string(),
                ));
            }
        }

        let has_init = ts.init.is_some();
        if has_init {
            locals.push_scope();
        }

        // Compile init statement if present
        if let Some(ref init) = ts.init {
            self.compile_statement(init, out, locals, result_types)?;
        }

        // Extract interface variable and optional binding name from tag
        // tag is: v := x.(type)  OR  x.(type)
        // The tag is stored as a Statement (an assignment or expression statement)
        let (iface_var_name, bind_name) = self.extract_type_switch_guard(ts)?;

        let tid_local = self.get_iface_type_id_local(&iface_var_name).ok_or_else(|| {
            Error::InternalError(format!("'{}' is not an interface variable", iface_var_name))
        })?;
        let data_local = locals.find(&iface_var_name).ok_or_else(|| {
            Error::InternalError(format!("variable '{}' not found", iface_var_name))
        })?;

        // Store type_id in a temp
        let type_id_tmp = locals.add_local(
            &format!("__tsw_tid_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(tid_local));
        out.push(Instruction::LocalSet(type_id_tmp));

        let matched_local = locals.add_local(
            &format!("__tsw_matched_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(matched_local));

        let mut default_body: Option<&Vec<ast::Statement>> = None;

        for clause in &ts.block.body {
            if clause.list.is_empty() {
                // Default case
                default_body = Some(clause.body.as_ref());
                continue;
            }

            // case T1, T2, ...:
            // Check if type_id matches any of the case types
            out.push(Instruction::LocalGet(matched_local));
            out.push(Instruction::I32Eqz);
            out.push(Instruction::If(BlockType::Empty));
            {
                // Build OR of type matches
                let mut first = true;
                for case_expr in &clause.list {
                    let type_name = Self::extract_type_name_from_expr(case_expr);
                    if let Some(ref tn) = type_name {
                        if tn == "nil" {
                            out.push(Instruction::LocalGet(type_id_tmp));
                            out.push(Instruction::I32Eqz);
                        } else {
                            let lookup_name = tn.strip_prefix('*').unwrap_or(tn);
                            let case_type_id = self.get_or_create_type_id(lookup_name);
                            out.push(Instruction::LocalGet(type_id_tmp));
                            out.push(Instruction::I32Const(case_type_id as i32));
                            out.push(Instruction::I32Eq);
                        }
                        if !first {
                            out.push(Instruction::I32Or);
                        }
                        first = false;
                    }
                }

                if first {
                    out.push(Instruction::I32Const(0));
                }

                out.push(Instruction::If(BlockType::Empty));
                {
                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::LocalSet(matched_local));

                    if let Some(ref bind) = bind_name {
                        if clause.list.len() == 1 {
                            let case_type_name = Self::extract_type_name_from_expr(&clause.list[0]);
                            if let Some(ref tn) = case_type_name {
                                if tn != "nil" {
                                    let bind_vt = Self::val_type_for_type_name(tn);
                                    let bind_local = locals.add_local(bind, bind_vt);
                                    out.push(Instruction::LocalGet(data_local));
                                    let (_, align) = Self::elem_size_and_align(bind_vt);
                                    Self::emit_typed_load(bind_vt, 0, align, out);
                                    out.push(Instruction::LocalSet(bind_local));
                                }
                            }
                        } else {
                            // Multiple types: bind as the interface value (I32 pointer)
                            let bind_local = locals.add_local(bind, ValType::I32);
                            out.push(Instruction::LocalGet(data_local));
                            out.push(Instruction::LocalSet(bind_local));
                        }
                    }

                    self.compile_block_stmts(clause.body.as_ref(), out, locals, result_types)?;
                }
                out.push(Instruction::End);
            }
            out.push(Instruction::End);
        }

        // Default case
        if let Some(default) = default_body {
            out.push(Instruction::LocalGet(matched_local));
            out.push(Instruction::I32Eqz);
            out.push(Instruction::If(BlockType::Empty));
            {
                self.compile_block_stmts(default, out, locals, result_types)?;
            }
            out.push(Instruction::End);
        }

        if has_init {
            locals.pop_scope();
        }

        Ok(())
    }

    fn extract_type_name_from_expr(expr: &ast::Expression) -> Option<String> {
        match expr {
            ast::Expression::Ident(ident) => Some(ident.name.clone()),
            ast::Expression::Star(star) => {
                let inner = Self::extract_type_name_from_expr(&star.right)?;
                Some(format!("*{}", inner))
            }
            ast::Expression::TypePointer(ptr) => {
                let inner = Self::extract_type_name_from_expr(&ptr.typ)?;
                Some(format!("*{}", inner))
            }
            ast::Expression::Selector(sel) => {
                if let ast::Expression::Ident(pkg) = sel.x.as_ref() {
                    Some(format!("{}.{}", pkg.name, sel.sel.name))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn extract_type_switch_guard(
        &self,
        ts: &ast::TypeSwitchStmt,
    ) -> Result<(String, Option<String>), Error> {
        if let Some(ref tag) = ts.tag {
            match tag.as_ref() {
                ast::Statement::Assign(assign) => {
                    // v := x.(type)
                    let bind_name = if let Some(ast::Expression::Ident(id)) = assign.left.first() {
                        Some(id.name.clone())
                    } else {
                        None
                    };
                    if let Some(ast::Expression::TypeAssert(ta)) = assign.right.first() {
                        if let ast::Expression::Ident(ident) = ta.left.as_ref() {
                            return Ok((ident.name.clone(), bind_name));
                        }
                    }
                    Err(Error::InternalError(
                        "type switch guard must be a type assertion".to_string(),
                    ))
                }
                ast::Statement::Expr(expr_stmt) => {
                    // x.(type)
                    if let ast::Expression::TypeAssert(ta) = &expr_stmt.expr {
                        if let ast::Expression::Ident(ident) = ta.left.as_ref() {
                            return Ok((ident.name.clone(), None));
                        }
                    }
                    Err(Error::InternalError(
                        "type switch guard must be a type assertion".to_string(),
                    ))
                }
                _ => Err(Error::InternalError(
                    "type switch guard must be an assignment or expression".to_string(),
                )),
            }
        } else {
            Err(Error::InternalError(
                "type switch statement missing guard".to_string(),
            ))
        }
    }

    fn compile_block_stmts(
        &mut self,
        stmts: &[ast::Statement],
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        for stmt in stmts {
            self.compile_statement(stmt, out, locals, result_types)?;
        }
        Ok(())
    }

    fn compile_interface_method_call(
        &mut self,
        iface_var: &str,
        method_name: &str,
        args: &[ast::Expression],
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let tid_local = self.get_iface_type_id_local(iface_var).ok_or_else(|| {
            Error::InternalError(format!("'{}' is not an interface variable", iface_var))
        })?;
        let data_local = locals.find(iface_var).ok_or_else(|| {
            Error::InternalError(format!("variable '{}' not found", iface_var))
        })?;

        // Nil check: if type_id == 0, panic with a descriptive message
        out.push(Instruction::LocalGet(tid_local));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        {
            let msg = format!("runtime error: nil pointer dereference (calling method {} on nil interface)", method_name);
            let msg_bytes = msg.as_bytes();
            let msg_len = msg_bytes.len() as i32;
            out.push(Instruction::I32Const(msg_len));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            let msg_ptr_local = locals.add_local(
                &format!("__nil_panic_ptr_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalSet(msg_ptr_local));
            for (j, &byte) in msg_bytes.iter().enumerate() {
                out.push(Instruction::LocalGet(msg_ptr_local));
                out.push(Instruction::I32Const(byte as i32));
                out.push(Instruction::I32Store8(MemArg {
                    offset: j as u64,
                    align: 0,
                    memory_index: 0,
                }));
            }
            out.push(Instruction::LocalGet(msg_ptr_local));
            out.push(Instruction::GlobalSet(self.panic_value_ptr_global));
            out.push(Instruction::I32Const(msg_len));
            out.push(Instruction::GlobalSet(self.panic_value_len_global));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::GlobalSet(self.panicking_global));
            out.push(Instruction::Unreachable);
        }
        out.push(Instruction::End);

        // Compile arguments to temp locals
        let mut arg_locals = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            self.compile_expression(arg, out, locals)?;
            let arg_vt = self.infer_val_type(arg, locals);
            let arg_local = locals.add_local(
                &format!("__imc_arg_{}_{}", i, locals.locals.len()),
                arg_vt,
            );
            out.push(Instruction::LocalSet(arg_local));
            arg_locals.push((arg_local, arg_vt));
        }

        // Find all concrete types that implement this method
        let candidates: Vec<(u32, u32, Vec<ValType>)> = self
            .functions
            .iter()
            .filter_map(|f| {
                let type_name = f.recv_type.as_ref()?;
                if !f.name.ends_with(&format!(".{}", method_name)) {
                    return None;
                }
                let type_id = self.type_registry.get(type_name).copied().unwrap_or(0);
                let result_types: Vec<ValType> = f.results.iter().map(|r| r.to_val_type()).collect();
                Some((type_id, f.wasm_func_idx, result_types))
            })
            .collect();

        if candidates.is_empty() {
            return Err(Error::InternalError(format!(
                "no concrete implementations found for interface method '{}'",
                method_name
            )));
        }

        // Determine result types from first candidate
        let result_vts: Vec<ValType> = candidates[0].2.clone();

        // Create locals for each return value
        let mut result_locals: Vec<(u32, ValType)> = Vec::new();
        for (j, &vt) in result_vts.iter().enumerate() {
            let rl = locals.add_local(
                &format!("__imc_res_{}_{}", j, locals.locals.len()),
                vt,
            );
            result_locals.push((rl, vt));
        }

        // Emit if/else chain on type_id
        for (type_id, func_idx, _result_types) in candidates.iter() {
            out.push(Instruction::LocalGet(tid_local));
            out.push(Instruction::I32Const(*type_id as i32));
            out.push(Instruction::I32Eq);
            out.push(Instruction::If(BlockType::Empty));
            {
                // Load the concrete value from the box (data_ptr is a pointer to the boxed value)
                out.push(Instruction::LocalGet(data_local));
                out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                // Push arguments
                for (arg_local, _) in &arg_locals {
                    out.push(Instruction::LocalGet(*arg_local));
                }
                out.push(Instruction::Call(*func_idx));
                // Store results in reverse order (last result first on stack)
                for (rl, _) in result_locals.iter().rev() {
                    out.push(Instruction::LocalSet(*rl));
                }
            }
            out.push(Instruction::End);
        }

        // Push results
        for (rl, _) in &result_locals {
            out.push(Instruction::LocalGet(*rl));
        }

        Ok(())
    }

    fn compile_range_map(
        &mut self,
        range: &ast::RangeStmt,
        map_name: &str,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
        label: Option<String>,
    ) -> Result<(), Error> {
        let mti = locals.map_types.get(map_name).ok_or_else(|| {
            Error::InternalError(format!("map type info not found for '{}'", map_name))
        })?.clone();

        let map_local = locals.find(map_name).ok_or_else(|| {
            Error::InternalError(format!("map variable '{}' not found", map_name))
        })?;

        let entry_size = Self::map_entry_size(mti.key_size, mti.val_size) as i32;

        let idx_local = locals.add_local("__rm_idx", ValType::I32);
        let count_local = locals.add_local("__rm_count", ValType::I32);
        let cap_local = locals.add_local("__rm_cap", ValType::I32);
        let data_local = locals.add_local("__rm_data", ValType::I32);
        let entry_local = locals.add_local("__rm_entry", ValType::I32);
        let slot_local = locals.add_local("__rm_slot", ValType::I32);

        // Load capacity and data_ptr from map header
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(cap_local));
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(data_local));

        // Randomize starting index: start = counter % cap (avoid div-by-zero)
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::I32GtU);
        out.push(Instruction::If(BlockType::Result(ValType::I32)));
        {
            out.push(Instruction::GlobalGet(self.map_iter_counter_global));
            out.push(Instruction::LocalGet(cap_local));
            out.push(Instruction::I32RemU);
        }
        out.push(Instruction::Else);
        out.push(Instruction::I32Const(0));
        out.push(Instruction::End);
        out.push(Instruction::LocalSet(idx_local));

        // Bump the global counter for next map iteration
        out.push(Instruction::GlobalGet(self.map_iter_counter_global));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::GlobalSet(self.map_iter_counter_global));

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(count_local));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));

        self.loop_depth.push((label, 0, false, true));

        // Check count < cap
        out.push(Instruction::LocalGet(count_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        // slot = idx % cap (wrap around)
        out.push(Instruction::LocalGet(idx_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32RemU);
        out.push(Instruction::LocalSet(slot_local));

        // entry = data + slot * entry_size
        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::LocalGet(slot_local));
        out.push(Instruction::I32Const(entry_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(entry_local));

        // Skip non-occupied entries (tag != 1)
        out.push(Instruction::LocalGet(entry_local));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Ne);
        out.push(Instruction::If(BlockType::Empty));
        {
            // Increment idx and count, continue
            out.push(Instruction::LocalGet(idx_local));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(idx_local));
            out.push(Instruction::LocalGet(count_local));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(count_local));
            out.push(Instruction::Br(1)); // back to loop
        }
        out.push(Instruction::End);

        // Extract key
        if let Some(key) = &range.key {
            if let ast::Expression::Ident(ident) = key {
                if ident.name != "_" {
                    let key_local = if range.op.as_ref().map_or(false, |(_, op)| *op == Operator::Define) {
                        locals.add_local(&ident.name, mti.key_vt)
                    } else {
                        locals.find(&ident.name).unwrap_or_else(|| locals.add_local(&ident.name, mti.key_vt))
                    };
                    out.push(Instruction::LocalGet(entry_local));
                    let (_, key_align) = Self::elem_size_and_align(mti.key_vt);
                    Self::emit_typed_load(mti.key_vt, 4, key_align, out);
                    out.push(Instruction::LocalSet(key_local));
                }
            }
        }

        // Extract value
        if let Some(value) = &range.value {
            if let ast::Expression::Ident(ident) = value {
                if ident.name != "_" {
                    let val_local = if range.op.as_ref().map_or(false, |(_, op)| *op == Operator::Define) {
                        locals.add_local(&ident.name, mti.val_vt)
                    } else {
                        locals.find(&ident.name).unwrap_or_else(|| locals.add_local(&ident.name, mti.val_vt))
                    };
                    let val_offset = 4u64 + mti.key_size as u64;
                    let (_, val_align) = Self::elem_size_and_align(mti.val_vt);
                    out.push(Instruction::LocalGet(entry_local));
                    Self::emit_typed_load(mti.val_vt, val_offset, val_align, out);
                    out.push(Instruction::LocalSet(val_local));
                }
            }
        }

        // Increment idx and count before body
        out.push(Instruction::LocalGet(idx_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(idx_local));
        out.push(Instruction::LocalGet(count_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(count_local));

        self.compile_block(&range.body, out, locals, result_types)?;

        out.push(Instruction::Br(0));
        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block

        self.loop_depth.pop();

        Ok(())
    }

    fn emit_string_eq_from_locals(
        tag_ptr: u32,
        tag_len: u32,
        case_ptr: u32,
        case_len: u32,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) {
        let result = locals.add_local(
            &format!("__sw_seq_r_{}", locals.locals.len()),
            ValType::I32,
        );
        let idx = locals.add_local(
            &format!("__sw_seq_i_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::I32Const(1));
        out.push(Instruction::LocalSet(result));
        out.push(Instruction::LocalGet(tag_len));
        out.push(Instruction::LocalGet(case_len));
        out.push(Instruction::I32Ne);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(result));
        out.push(Instruction::Else);
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(idx));
        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::LocalGet(tag_len));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));
        out.push(Instruction::LocalGet(tag_ptr));
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Add);
        out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        out.push(Instruction::LocalGet(case_ptr));
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Add);
        out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        out.push(Instruction::I32Ne);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(result));
        out.push(Instruction::Br(2));
        out.push(Instruction::End);
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(idx));
        out.push(Instruction::Br(0));
        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block
        out.push(Instruction::End); // else
        out.push(Instruction::LocalGet(result));
    }

    fn compile_switch_labeled(
        &mut self,
        switch: &ast::SwitchStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
        label: Option<String>,
    ) -> Result<(), Error> {
        self.compile_switch_inner(switch, out, locals, result_types, label)
    }

    fn compile_switch(
        &mut self,
        switch: &ast::SwitchStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        self.compile_switch_inner(switch, out, locals, result_types, None)
    }

    fn compile_switch_inner(
        &mut self,
        switch: &ast::SwitchStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
        label: Option<String>,
    ) -> Result<(), Error> {
        let has_init = switch.init.is_some();
        if has_init {
            locals.push_scope();
        }

        if let Some(init) = &switch.init {
            self.compile_statement(init, out, locals, result_types)?;
        }

        let is_string_tag = switch.tag.as_ref().map_or(false, |t| self.is_string_expr(t, locals));

        let tag_str_ptr: Option<u32>;
        let tag_str_len: Option<u32>;

        let (tag_local, tag_vt) = if let Some(tag) = &switch.tag {
            if is_string_tag {
                let ptr_l = locals.add_local("__switch_tag_ptr", ValType::I32);
                let len_l = locals.add_local("__switch_tag_len", ValType::I32);
                self.compile_expression(tag, out, locals)?;
                out.push(Instruction::LocalSet(len_l));
                out.push(Instruction::LocalSet(ptr_l));
                tag_str_ptr = Some(ptr_l);
                tag_str_len = Some(len_l);
                (None, ValType::I32)
            } else {
                tag_str_ptr = None;
                tag_str_len = None;
                let vt = self.infer_val_type(tag, locals);
                let local = locals.add_local("__switch_tag", vt);
                self.compile_expression(tag, out, locals)?;
                out.push(Instruction::LocalSet(local));
                (Some(local), vt)
            }
        } else {
            tag_str_ptr = None;
            tag_str_len = None;
            (None, ValType::I32)
        };

        let mut non_default_cases: Vec<&ast::CaseClause> = Vec::new();
        let mut default_case: Option<&ast::CaseClause> = None;

        for case in &switch.block.body {
            if case.tok == Keyword::Default {
                default_case = Some(case);
            } else {
                non_default_cases.push(case);
            }
        }

        // Validate fallthrough: cannot appear in the last case clause when there is no
        // subsequent clause to fall into.
        let all_clauses = &switch.block.body;
        if let Some(last_clause) = all_clauses.last() {
            if last_clause.body.iter().any(|s| Self::is_fallthrough_stmt(s)) {
                return Err(Error::SyntaxError(
                    "cannot fallthrough final case of switch".to_string(),
                ));
            }
        }

        if non_default_cases.is_empty() {
            if let Some(def) = default_case {
                for stmt in def.body.iter() {
                    self.compile_statement(stmt, out, locals, result_types)?;
                }
            }
            if has_init {
                locals.pop_scope();
            }
            return Ok(());
        }

        let eq_instr = match tag_vt {
            ValType::I32 => Instruction::I32Eq,
            ValType::F32 => Instruction::F32Eq,
            ValType::F64 => Instruction::F64Eq,
            _ => Instruction::I64Eq,
        };

        let num_cases = non_default_cases.len();

        let has_fallthrough = non_default_cases.iter().any(|c| {
            c.body.iter().any(|s| Self::is_fallthrough_stmt(s))
        });

        out.push(Instruction::Block(BlockType::Empty));
        self.loop_depth.push((label, 0, false, false));

        if has_fallthrough {
            let matched = locals.add_local("__sw_matched", ValType::I32);
            let ft = locals.add_local("__sw_ft", ValType::I32);
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(matched));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(ft));

            for case in non_default_cases.iter() {
                let mut first = true;
                for expr in &case.list {
                    if is_string_tag {
                        let cp = locals.add_local(
                            &format!("__sw_cp_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        let cl = locals.add_local(
                            &format!("__sw_cl_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        self.compile_expression(expr, out, locals)?;
                        out.push(Instruction::LocalSet(cl));
                        out.push(Instruction::LocalSet(cp));
                        let sptr = tag_str_ptr.ok_or_else(|| Error::InternalError(
                            "switch string tag pointer local missing".to_string(),
                        ))?;
                        let slen = tag_str_len.ok_or_else(|| Error::InternalError(
                            "switch string tag length local missing".to_string(),
                        ))?;
                        Self::emit_string_eq_from_locals(
                            sptr, slen,
                            cp, cl, out, locals,
                        );
                    } else if let Some(tag_l) = tag_local {
                        out.push(Instruction::LocalGet(tag_l));
                        self.compile_expression(expr, out, locals)?;
                        let case_vt = self.infer_val_type(expr, locals);
                        Self::emit_typed_coerce(case_vt, tag_vt, out)?;
                        out.push(eq_instr.clone());
                    } else {
                        self.compile_expression(expr, out, locals)?;
                    }
                    if !first {
                        out.push(Instruction::I32Or);
                    }
                    first = false;
                }
                out.push(Instruction::LocalGet(ft));
                out.push(Instruction::I32Or);

                out.push(Instruction::If(BlockType::Empty));
                if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                    *depth += 1;
                }

                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(ft));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(matched));

                for stmt in case.body.iter() {
                    if Self::is_fallthrough_stmt(stmt) {
                        out.push(Instruction::I32Const(1));
                        out.push(Instruction::LocalSet(ft));
                        continue;
                    }
                    self.compile_statement(stmt, out, locals, result_types)?;
                }

                if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                    *depth -= 1;
                }
                out.push(Instruction::End); // end if
            }

            if let Some(def) = default_case {
                out.push(Instruction::LocalGet(matched));
                out.push(Instruction::I32Eqz);
                out.push(Instruction::LocalGet(ft));
                out.push(Instruction::I32Or);
                out.push(Instruction::If(BlockType::Empty));
                if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                    *depth += 1;
                }
                for stmt in def.body.iter() {
                    self.compile_statement(stmt, out, locals, result_types)?;
                }
                if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                    *depth -= 1;
                }
                out.push(Instruction::End);
            }
        } else {
            for (i, case) in non_default_cases.iter().enumerate() {
                let mut first = true;
                for expr in &case.list {
                    if is_string_tag {
                        let cp = locals.add_local(
                            &format!("__sw_cp_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        let cl = locals.add_local(
                            &format!("__sw_cl_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        self.compile_expression(expr, out, locals)?;
                        out.push(Instruction::LocalSet(cl));
                        out.push(Instruction::LocalSet(cp));
                        let sptr = tag_str_ptr.ok_or_else(|| Error::InternalError(
                            "switch string tag pointer local missing".to_string(),
                        ))?;
                        let slen = tag_str_len.ok_or_else(|| Error::InternalError(
                            "switch string tag length local missing".to_string(),
                        ))?;
                        Self::emit_string_eq_from_locals(
                            sptr, slen,
                            cp, cl, out, locals,
                        );
                    } else if let Some(tag_l) = tag_local {
                        out.push(Instruction::LocalGet(tag_l));
                        self.compile_expression(expr, out, locals)?;
                        let case_vt = self.infer_val_type(expr, locals);
                        Self::emit_typed_coerce(case_vt, tag_vt, out)?;
                        out.push(eq_instr.clone());
                    } else {
                        self.compile_expression(expr, out, locals)?;
                    }
                    if !first {
                        out.push(Instruction::I32Or);
                    }
                    first = false;
                }

                out.push(Instruction::If(BlockType::Empty));
                if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                    *depth += 1;
                }

                for stmt in case.body.iter() {
                    self.compile_statement(stmt, out, locals, result_types)?;
                }

                let is_last = i == num_cases - 1;
                if !is_last || default_case.is_some() {
                    out.push(Instruction::Else);
                } else {
                    if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                        *depth -= 1;
                    }
                    out.push(Instruction::End);
                }
            }

            if let Some(def) = default_case {
                for stmt in def.body.iter() {
                    self.compile_statement(stmt, out, locals, result_types)?;
                }
            }

            let blocks_to_close = if default_case.is_some() {
                num_cases
            } else {
                num_cases.saturating_sub(1)
            };

            for _ in 0..blocks_to_close {
                if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                    *depth -= 1;
                }
                out.push(Instruction::End);
            }
        }

        self.loop_depth.pop();
        out.push(Instruction::End);

        if has_init {
            locals.pop_scope();
        }

        Ok(())
    }

    fn compile_branch(
        &mut self,
        branch: &ast::BranchStmt,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        if branch.key == Keyword::Goto {
            // goto is intentionally excluded: WASM uses structured control flow (block/loop/if)
            // which cannot directly represent arbitrary jumps. While goto could be emulated with
            // a loop+switch dispatch, it is rarely used in practice and not worth the complexity
            // for UDF workloads.
            return Err(Error::InternalError(
                "goto is not supported in WASM UDFs".to_string(),
            ));
        }
        if branch.key == Keyword::FallThrough {
            // Fallthrough is handled by compile_switch; if we reach here,
            // it means fallthrough was used outside a switch — which is a Go error.
            return Err(Error::InternalError(
                "fallthrough statement outside switch".to_string(),
            ));
        }
        if let Some(ref label_ident) = branch.ident {
            return self.compile_labeled_branch(branch.key, &label_ident.name, out);
        }

        match branch.key {
            Keyword::Break => {
                let (extra, has_post, is_loop) = self
                    .loop_depth
                    .last()
                    .map(|(_, d, hp, il)| (*d, *hp, *il))
                    .unwrap_or((0, false, true));
                if is_loop {
                    out.push(Instruction::Br(1 + extra + has_post as u32));
                } else {
                    out.push(Instruction::Br(extra));
                }
            }
            Keyword::Continue => {
                let mut depth: u32 = 0;
                let mut found = false;
                for entry in self.loop_depth.iter().rev() {
                    if entry.3 {
                        depth += entry.1;
                        out.push(Instruction::Br(depth));
                        found = true;
                        break;
                    } else {
                        depth += entry.1 + 1;
                    }
                }
                if !found {
                    return Err(Error::InternalError(
                        "continue statement outside loop".to_string(),
                    ));
                }
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported branch keyword: {:?}",
                    branch.key
                )));
            }
        }
        Ok(())
    }

    fn compile_labeled_branch(
        &mut self,
        key: Keyword,
        label: &str,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        let target_idx = self
            .loop_depth
            .iter()
            .rposition(|(lbl, _, _, _)| lbl.as_deref() == Some(label))
            .ok_or_else(|| {
                Error::InternalError(format!("undefined label: {}", label))
            })?;

        let innermost = self.loop_depth.len() - 1;
        let inner_extra = self.loop_depth[innermost].1;

        let mut intermediate_depth: u32 = 0;

        if innermost > target_idx {
            // Add block levels for the innermost entry (its internal depth
            // is already accounted for by inner_extra, so exclude entry.1).
            let entry = &self.loop_depth[innermost];
            if entry.3 {
                intermediate_depth += 2 + entry.2 as u32;
            } else {
                intermediate_depth += 1;
            }

            // Add full contribution for entries between target and innermost
            for i in (target_idx + 1..innermost).rev() {
                let entry = &self.loop_depth[i];
                if entry.3 {
                    intermediate_depth += 2 + entry.2 as u32 + entry.1;
                } else {
                    intermediate_depth += 1 + entry.1;
                }
            }
        }

        match key {
            Keyword::Break => {
                let target = &self.loop_depth[target_idx];
                if target.3 {
                    let target_has_post = target.2 as u32;
                    let depth = inner_extra + intermediate_depth + target_has_post + 1;
                    out.push(Instruction::Br(depth));
                } else {
                    let depth = inner_extra + intermediate_depth;
                    out.push(Instruction::Br(depth));
                }
            }
            Keyword::Continue => {
                let depth = inner_extra + intermediate_depth;
                out.push(Instruction::Br(depth));
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported branch keyword: {:?}",
                    key
                )));
            }
        }
        Ok(())
    }

    fn emit_incdec_op(op: Operator, vt: ValType, out: &mut Vec<Instruction<'static>>) -> Result<(), Error> {
        match (op, vt) {
            (Operator::Inc, ValType::I64) => {
                out.push(Instruction::I64Const(1));
                out.push(Instruction::I64Add);
            }
            (Operator::Dec, ValType::I64) => {
                out.push(Instruction::I64Const(1));
                out.push(Instruction::I64Sub);
            }
            (Operator::Inc, ValType::I32) => {
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Add);
            }
            (Operator::Dec, ValType::I32) => {
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Sub);
            }
            (Operator::Inc, ValType::F64) => {
                out.push(Instruction::F64Const(1.0));
                out.push(Instruction::F64Add);
            }
            (Operator::Dec, ValType::F64) => {
                out.push(Instruction::F64Const(1.0));
                out.push(Instruction::F64Sub);
            }
            (Operator::Inc, ValType::F32) => {
                out.push(Instruction::F32Const(1.0));
                out.push(Instruction::F32Add);
            }
            (Operator::Dec, ValType::F32) => {
                out.push(Instruction::F32Const(1.0));
                out.push(Instruction::F32Sub);
            }
            (Operator::Inc, _) => {
                out.push(Instruction::I64Const(1));
                out.push(Instruction::I64Add);
            }
            (Operator::Dec, _) => {
                out.push(Instruction::I64Const(1));
                out.push(Instruction::I64Sub);
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported inc/dec operator: {:?}",
                    op
                )));
            }
        }
        Ok(())
    }

    fn compile_incdec(
        &mut self,
        incdec: &ast::IncDecStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        match &incdec.expr {
            ast::Expression::Ident(ident) => {
                if let Some(idx) = locals.find(&ident.name) {
                    out.push(Instruction::LocalGet(idx));
                    let vt = self.infer_val_type(&incdec.expr, locals);
                    Self::emit_incdec_op(incdec.op, vt, out)?;
                    out.push(Instruction::LocalSet(idx));
                } else if let Some(&(global_idx, vt)) = self.global_vars.get(&ident.name) {
                    out.push(Instruction::GlobalGet(global_idx));
                    Self::emit_incdec_op(incdec.op, vt, out)?;
                    out.push(Instruction::GlobalSet(global_idx));
                } else {
                    return Err(Error::InternalError(format!(
                        "undefined variable '{}' in increment/decrement",
                        ident.name
                    )));
                }
            }
            ast::Expression::Index(idx_expr) => {
                let (elem_vt, align) =
                    self.compile_index_store_addr(idx_expr, out, locals)?;
                let addr_tmp = locals.add_local(
                    &format!("__incdec_addr_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalSet(addr_tmp));

                out.push(Instruction::LocalGet(addr_tmp));
                Self::emit_typed_load(elem_vt, 0, align, out);

                Self::emit_incdec_op(incdec.op, elem_vt, out)?;

                let result_tmp = locals.add_local(
                    &format!("__incdec_res_{}", locals.locals.len()),
                    elem_vt,
                );
                out.push(Instruction::LocalSet(result_tmp));
                out.push(Instruction::LocalGet(addr_tmp));
                out.push(Instruction::LocalGet(result_tmp));
                Self::emit_typed_store(elem_vt, 0, align, out);
            }
            ast::Expression::Selector(sel) => {
                let (offset, field_vt) =
                    self.compile_selector_store_addr(sel, out, locals)?;
                let (_, align) = Self::elem_size_and_align(field_vt);
                let addr_tmp = locals.add_local(
                    &format!("__incdec_addr_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalSet(addr_tmp));

                out.push(Instruction::LocalGet(addr_tmp));
                Self::emit_typed_load(field_vt, offset, align, out);

                Self::emit_incdec_op(incdec.op, field_vt, out)?;

                let result_tmp = locals.add_local(
                    &format!("__incdec_res_{}", locals.locals.len()),
                    field_vt,
                );
                out.push(Instruction::LocalSet(result_tmp));
                out.push(Instruction::LocalGet(addr_tmp));
                out.push(Instruction::LocalGet(result_tmp));
                Self::emit_typed_store(field_vt, offset, align, out);
            }
            _ => {
                return Err(Error::InternalError(
                    "increment/decrement on unsupported expression".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn compile_decl_stmt(
        &mut self,
        decl: &ast::DeclStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        match decl {
            ast::DeclStmt::Variable(var_decl) => {
                for spec in &var_decl.specs {
                    if let Some(ref typ) = spec.typ {
                        Self::reject_unsupported_type(typ)?;
                    }
                    for (i, ident) in spec.name.iter().enumerate() {
                        let vt = if let Some(ref typ) = spec.typ {
                            self.expr_to_val_type(typ)
                        } else if i < spec.values.len() {
                            self.infer_val_type(&spec.values[i], locals)
                        } else {
                            ValType::I64
                        };

                        let local_idx = locals.add_local(&ident.name, vt);

                        // Track struct, string, interface, and unsigned types
                        let mut is_string = false;
                        let mut is_iface = false;
                        let mut iface_type_name: Option<String> = None;
                        if let Some(ref typ) = spec.typ {
                            if let ast::Expression::TypeInterface(_) = typ {
                                is_iface = true;
                                locals.set_var_struct_type(&ident.name, "__interface");
                                let tid_local = locals.add_local(
                                    &format!("{}__type_id", ident.name),
                                    ValType::I32,
                                );
                                self.iface_var_type_ids.insert(ident.name.clone(), tid_local);
                            } else if let ast::Expression::TypeSlice(slice_type) = typ {
                                locals.set_var_struct_type(&ident.name, "__slice");
                                let elem_vt = Self::infer_array_elem_vt(&slice_type.typ);
                                locals.slice_elem_types.insert(ident.name.clone(), elem_vt);
                                if let ast::Expression::TypeSlice(inner_st) = slice_type.typ.as_ref() {
                                    let inner_vt = Self::infer_array_elem_vt(&inner_st.typ);
                                    locals.nested_slice_inner_elem_types.insert(ident.name.clone(), inner_vt);
                                }
                                if let ast::Expression::Ident(el_id) = slice_type.typ.as_ref() {
                                    if self.struct_defs.contains_key(&el_id.name) {
                                        locals.slice_elem_struct_types.insert(ident.name.clone(), el_id.name.clone());
                                    }
                                }
                            } else if let ast::Expression::TypeMap(map_type) = typ {
                                locals.set_var_struct_type(&ident.name, "__map");
                                let (kv, ks, vv, vs, sk, sv, vst) = self.map_key_val_types(map_type);
                                let nested = self.build_nested_map_type_info(map_type);
                                locals.map_types.insert(
                                    ident.name.clone(),
                                    MapTypeInfo { key_vt: kv, val_vt: vv, key_size: ks, val_size: vs, is_string_key: sk, is_string_val: sv, val_struct_type: vst, nested_map_val_type: nested },
                                );
                            } else if let ast::Expression::TypeArray(arr_type) = typ {
                                let arr_len = if let ast::Expression::BasicLit(lit) = arr_type.len.as_ref() {
                                    Self::parse_go_int(&lit.value)
                                        .map_err(|e| Error::SyntaxError(e))? as u32
                                } else { 0 };
                                let elem_vt = Self::infer_array_elem_vt(&arr_type.typ);
                                locals.set_var_struct_type(&ident.name, "__array");
                                locals.array_info.insert(ident.name.clone(), (elem_vt, arr_len));
                                if let ast::Expression::TypeArray(inner_arr) = arr_type.typ.as_ref() {
                                    let inner_len = if let ast::Expression::BasicLit(lit) = inner_arr.len.as_ref() {
                                        Self::parse_go_int(&lit.value)
                                            .map_err(|e| Error::SyntaxError(e))? as u32
                                    } else { 0 };
                                    let inner_elem_vt = Self::infer_array_elem_vt(&inner_arr.typ);
                                    locals.nested_array_inner_info.insert(ident.name.clone(), (inner_elem_vt, inner_len));
                                }
                            } else if let ast::Expression::Ident(type_ident) = typ {
                                if type_ident.name == "string" {
                                    is_string = true;
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        "__string",
                                    );
                                } else if type_ident.name == "complex64" {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        "__complex64",
                                    );
                                } else if type_ident.name == "complex128" {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        "__complex128",
                                    );
                                } else if self.iface_defs.contains_key(&type_ident.name)
                                    || type_ident.name == "error"
                                    || type_ident.name == "any"
                                {
                                    is_iface = true;
                                    iface_type_name = Some(type_ident.name.clone());
                                    let iface_tag = format!("__iface_{}", type_ident.name);
                                    locals.set_var_struct_type(&ident.name, &iface_tag);
                                    let tid_local = locals.add_local(
                                        &format!("{}__type_id", ident.name),
                                        ValType::I32,
                                    );
                                    self.iface_var_type_ids.insert(ident.name.clone(), tid_local);
                                } else if self.struct_defs.contains_key(&type_ident.name) {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        &type_ident.name,
                                    );
                                } else if self.type_aliases.contains_key(&type_ident.name) {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        &type_ident.name,
                                    );
                                } else if let Some(underlying) = self.named_composite_types.get(&type_ident.name).cloned() {
                                    self.setup_named_composite_var(&ident.name, &underlying, locals);
                                }
                                if Self::is_unsigned_type_name(&type_ident.name) {
                                    locals.unsigned_vars.insert(ident.name.clone());
                                }
                            }
                        }
                        if i < spec.values.len() && !is_iface {
                            if let ast::Expression::CompositeLit(comp) = &spec.values[i]
                            {
                                if let ast::Expression::Ident(type_ident) =
                                    comp.typ.as_ref()
                                {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        &type_ident.name,
                                    );
                                }
                            }
                            if let ast::Expression::Operation(addr_op) = &spec.values[i] {
                                if addr_op.op == Operator::And && addr_op.y.is_none() {
                                    if let ast::Expression::CompositeLit(comp) = &*addr_op.x {
                                        if let ast::Expression::Ident(type_ident) = comp.typ.as_ref() {
                                            locals.set_var_struct_type(
                                                &ident.name,
                                                &type_ident.name,
                                            );
                                        }
                                    }
                                }
                            }
                            if !is_string && self.is_string_expr(&spec.values[i], locals) {
                                is_string = true;
                                locals.set_var_struct_type(
                                    &ident.name,
                                    "__string",
                                );
                            }
                        }

                        if is_string {
                            let len_local = locals.add_local(
                                &format!("{}__str_len", ident.name),
                                ValType::I32,
                            );
                            locals.string_locals.insert(
                                ident.name.clone(),
                                (local_idx, len_local),
                            );
                        }

                        if i < spec.values.len() {
                            self.compile_expression(&spec.values[i], out, locals)?;
                            if is_iface {
                                let rhs_vt = self.infer_val_type(&spec.values[i], locals);
                                let rhs_type_name = self.infer_concrete_type_name(&spec.values[i], locals);
                                if let Some(ref iname) = iface_type_name {
                                    if !self.is_interface_var_expr(&spec.values[i], locals) {
                                        self.check_iface_satisfaction(iname, &rhs_type_name)?;
                                    }
                                }
                                let type_id = self.get_or_create_type_id(&rhs_type_name);
                                let tid_local = *self.iface_var_type_ids.get(&ident.name).ok_or_else(|| {
                                    Error::InternalError(format!(
                                        "interface type-id local not found for '{}'", ident.name
                                    ))
                                })?;
                                let (elem_size, _) = Self::elem_size_and_align(rhs_vt);
                                self.emit_box_value(rhs_vt, elem_size, type_id, tid_local, local_idx, out, locals)?;
                            } else if is_string {
                                let (ptr_local, len_local) = locals.string_locals[&ident.name];
                                out.push(Instruction::LocalSet(len_local));
                                out.push(Instruction::LocalSet(ptr_local));
                            } else {
                                out.push(Instruction::LocalSet(local_idx));
                            }
                        } else if let Some(ref typ) = spec.typ {
                            if let ast::Expression::Ident(type_ident) = typ {
                                if let Some(sd) = self.struct_defs.get(&type_ident.name) {
                                    let size = sd.total_size as i32;
                                    out.push(Instruction::I32Const(size));
                                    out.push(Instruction::Call(self.alloc_func_idx()?));
                                    out.push(Instruction::LocalTee(local_idx));
                                    out.push(Instruction::I32Const(0));
                                    out.push(Instruction::I32Const(size));
                                    out.push(Instruction::MemoryFill(0));
                                }
                            } else if let ast::Expression::TypeStruct(_) = typ {
                                let field_count = if let ast::Expression::TypeStruct(st) = typ {
                                    st.fields.len()
                                } else {
                                    0
                                };
                                let size = ((field_count * 8) as i32).max(8);
                                out.push(Instruction::I32Const(size));
                                out.push(Instruction::Call(self.alloc_func_idx()?));
                                out.push(Instruction::LocalTee(local_idx));
                                out.push(Instruction::I32Const(0));
                                out.push(Instruction::I32Const(size));
                                out.push(Instruction::MemoryFill(0));
                            } else if let ast::Expression::TypeArray(arr_type) = typ {
                                let arr_len = if let ast::Expression::BasicLit(lit) = arr_type.len.as_ref() {
                                    Self::parse_go_int(&lit.value)
                                        .map_err(|e| Error::SyntaxError(e))? as u32
                                } else { 0 };
                                let elem_vt = Self::infer_array_elem_vt(&arr_type.typ);
                                let (elem_size, _) = Self::elem_size_and_align(elem_vt);
                                let total_bytes = if let Some(&(inner_elem_vt, inner_len)) = locals.nested_array_inner_info.get(&ident.name) {
                                    let (inner_elem_size, _) = Self::elem_size_and_align(inner_elem_vt);
                                    (arr_len * inner_len * inner_elem_size as u32) as i32
                                } else {
                                    (arr_len as i32) * elem_size
                                };
                                if total_bytes > 0 {
                                    out.push(Instruction::I32Const(total_bytes));
                                    out.push(Instruction::Call(self.alloc_func_idx()?));
                                    out.push(Instruction::LocalTee(local_idx));
                                    out.push(Instruction::I32Const(0));
                                    out.push(Instruction::I32Const(total_bytes));
                                    out.push(Instruction::MemoryFill(0));
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
            ast::DeclStmt::Const(const_decl) => {
                let old_iota = self.current_iota;
                let mut last_exprs: Vec<ast::Expression> = Vec::new();
                for (iota_val, spec) in const_decl.specs.iter().enumerate() {
                    self.current_iota = Some(iota_val as i64);
                    let exprs_to_use: &[ast::Expression] = if spec.values.is_empty() {
                        last_exprs.as_slice()
                    } else {
                        last_exprs = spec.values.clone();
                        spec.values.as_slice()
                    };
                    for (i, ident) in spec.name.iter().enumerate() {
                        let expr = exprs_to_use.get(i).or(exprs_to_use.first());
                        if let Some(expr) = expr {
                            let cv = self.eval_const_expr(expr, &ident.name)?;
                            self.constants.insert(ident.name.clone(), cv);
                        }
                    }
                }
                self.current_iota = old_iota;
                Ok(())
            }
            ast::DeclStmt::Type(type_decl) => {
                for spec in &type_decl.specs {
                    Self::reject_unsupported_type(&spec.typ)?;
                    if let ast::Expression::TypeStruct(struct_type) = &spec.typ {
                        let struct_def = self.compute_struct_def(&struct_type.fields);
                        self.struct_defs
                            .insert(spec.name.name.clone(), struct_def);
                        self.register_struct_field_map_types(&spec.name.name, &struct_type.fields);
                    }
                }
                Ok(())
            }
        }
    }

    fn compile_expression(
        &mut self,
        expr: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        match expr {
            ast::Expression::BasicLit(lit) => self.compile_basic_lit(lit, out, locals),
            ast::Expression::Ident(ident) => self.compile_ident(ident, out, locals),
            ast::Expression::Operation(op) => self.compile_operation(op, out, locals),
            ast::Expression::Call(call) => self.compile_call(call, out, locals),
            ast::Expression::Paren(paren) => {
                self.compile_expression(&paren.expr, out, locals)
            }
            ast::Expression::Selector(sel) => {
                self.compile_selector(sel, out, locals)
            }
            ast::Expression::FuncLit(func_lit) => {
                self.compile_func_lit(func_lit, out, locals)
            }
            ast::Expression::CompositeLit(comp) => {
                self.compile_composite_lit(comp, out, locals)
            }
            ast::Expression::Index(idx) => self.compile_index(idx, out, locals),
            ast::Expression::Star(star) => {
                self.compile_expression(&star.right, out, locals)?;
                let ptr_local = locals.add_local(
                    &format!("__deref_ptr_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalTee(ptr_local));
                out.push(Instruction::I32Eqz);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::Unreachable);
                out.push(Instruction::End);
                out.push(Instruction::LocalGet(ptr_local));

                let deref_vt = self.infer_deref_type(&star.right, locals);
                let (_size, mem_idx) = Self::elem_size_and_align(deref_vt);
                let mem_arg = MemArg {
                    offset: 0,
                    align: mem_idx,
                    memory_index: 0,
                };
                match deref_vt {
                    ValType::I64 => out.push(Instruction::I64Load(mem_arg)),
                    ValType::F32 => out.push(Instruction::F32Load(mem_arg)),
                    ValType::F64 => out.push(Instruction::F64Load(mem_arg)),
                    _ => out.push(Instruction::I32Load(mem_arg)),
                }
                Ok(())
            }
            ast::Expression::TypeAssert(ta) => {
                self.compile_type_assert(ta, out, locals)
            }
            ast::Expression::Slice(slice) => {
                self.compile_slice_expr(slice, out, locals)
            }
            ast::Expression::List(exprs) => {
                for e in exprs {
                    self.compile_expression(e, out, locals)?;
                }
                Ok(())
            }
            ast::Expression::Invar(inv) => {
                self.compile_expression(&inv.expr, out, locals)
            }
            ast::Expression::Range(_) => Err(Error::InternalError(
                "range expression is only valid inside a for statement".to_string(),
            )),
            ast::Expression::TypeMap(_) => Ok(()),
            ast::Expression::TypeInterface(_) => Ok(()),
            ast::Expression::TypeChannel(_) => Err(Error::InternalError(
                "channels are not supported in WASM UDFs".to_string(),
            )),
            ast::Expression::TypeArray(_)
            | ast::Expression::TypeSlice(_)
            | ast::Expression::TypeFunction(_)
            | ast::Expression::TypeStruct(_)
            | ast::Expression::TypePointer(_)
            | ast::Expression::IndexList(_)
            | ast::Expression::Ellipsis(_) => Ok(()),
        }
    }

    fn compile_basic_lit(
        &self,
        lit: &ast::BasicLit,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        match lit.kind {
            LitKind::Integer => {
                let val: i64 = Self::parse_go_int(&lit.value)
                    .map_err(|e| Error::SyntaxError(e))?;
                out.push(Instruction::I64Const(val));
            }
            LitKind::Float => {
                let val: f64 = Self::parse_go_float(&lit.value).map_err(|_| {
                    Error::SyntaxError(format!("invalid float literal: {}", lit.value))
                })?;
                out.push(Instruction::F64Const(val));
            }
            LitKind::String => {
                let bytes = Self::extract_string_bytes(&lit.value);
                let len = bytes.len() as i32;

                out.push(Instruction::I32Const(len));
                out.push(Instruction::Call(self.alloc_func_idx()?));

                let ptr_local = locals.add_local(
                    &format!("__str_ptr_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalSet(ptr_local));

                for (i, &byte) in bytes.iter().enumerate() {
                    out.push(Instruction::LocalGet(ptr_local));
                    out.push(Instruction::I32Const(byte as i32));
                    out.push(Instruction::I32Store8(MemArg {
                        offset: i as u64,
                        align: 0,
                        memory_index: 0,
                    }));
                }

                out.push(Instruction::LocalGet(ptr_local));
                out.push(Instruction::I32Const(len));
            }
            LitKind::Char => {
                let s = lit.value.trim_matches('\'');
                let ch = Self::unescape_go_char(s)? as i32;
                out.push(Instruction::I32Const(ch));
            }
            LitKind::Imag => {
                let num_str = lit.value.trim_end_matches('i');
                let imag_val: f64 = num_str.parse().map_err(|_| {
                    Error::SyntaxError(format!("invalid imaginary literal: {}", lit.value))
                })?;
                let total_size: i32 = 16; // complex128: two f64s
                let float_align: u32 = 3;

                let real_local = locals.add_local(
                    &format!("__imag_r_{}", locals.locals.len()),
                    ValType::F64,
                );
                let imag_local = locals.add_local(
                    &format!("__imag_i_{}", locals.locals.len()),
                    ValType::F64,
                );
                out.push(Instruction::F64Const(0.0));
                out.push(Instruction::LocalSet(real_local));
                out.push(Instruction::F64Const(imag_val));
                out.push(Instruction::LocalSet(imag_local));

                out.push(Instruction::I32Const(total_size));
                out.push(Instruction::Call(self.alloc_func_idx()?));
                let ptr = locals.add_local(
                    &format!("__imag_ptr_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalSet(ptr));

                out.push(Instruction::LocalGet(ptr));
                out.push(Instruction::LocalGet(real_local));
                out.push(Instruction::F64Store(MemArg { offset: 0, align: float_align, memory_index: 0 }));

                out.push(Instruction::LocalGet(ptr));
                out.push(Instruction::LocalGet(imag_local));
                out.push(Instruction::F64Store(MemArg { offset: 8, align: float_align, memory_index: 0 }));

                out.push(Instruction::LocalGet(ptr));
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported literal kind: {:?}",
                    lit.kind
                )));
            }
        }
        Ok(())
    }

    fn extract_string_bytes(lit_value: &str) -> Vec<u8> {
        if lit_value.starts_with('`') {
            let s = lit_value.trim_matches('`');
            // Go spec: carriage return characters inside raw string literals are discarded
            s.bytes().filter(|&b| b != b'\r').collect()
        } else {
            let s = lit_value.trim_matches('"');
            Self::unescape_go_string(s)
        }
    }

    fn extract_string_content(lit_value: &str) -> Option<String> {
        if lit_value.starts_with('`') {
            let s: String = lit_value.trim_matches('`').chars().filter(|&c| c != '\r').collect();
            Some(s)
        } else {
            let s = lit_value.trim_matches('"');
            let bytes = Self::unescape_go_string(s);
            String::from_utf8(bytes).ok()
        }
    }

    fn unescape_go_string(s: &str) -> Vec<u8> {
        let mut result = Vec::new();
        let mut chars = s.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('n') => result.push(b'\n'),
                    Some('t') => result.push(b'\t'),
                    Some('r') => result.push(b'\r'),
                    Some('\\') => result.push(b'\\'),
                    Some('"') => result.push(b'"'),
                    Some('\'') => result.push(b'\''),
                    Some('a') => result.push(0x07),
                    Some('b') => result.push(0x08),
                    Some('f') => result.push(0x0C),
                    Some('v') => result.push(0x0B),
                    Some('x') => {
                        let hex: String = chars.by_ref().take(2).collect();
                        if let Ok(val) = u8::from_str_radix(&hex, 16) {
                            result.push(val);
                        }
                    }
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if let Ok(val) = u32::from_str_radix(&hex, 16) {
                            if let Some(c) = char::from_u32(val) {
                                let mut buf = [0u8; 4];
                                result.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                            }
                        }
                    }
                    Some('U') => {
                        let hex: String = chars.by_ref().take(8).collect();
                        if let Ok(val) = u32::from_str_radix(&hex, 16) {
                            if let Some(c) = char::from_u32(val) {
                                let mut buf = [0u8; 4];
                                result.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                            }
                        }
                    }
                    Some(d) if d.is_ascii_digit() && d < '8' => {
                        let mut octal = String::new();
                        octal.push(d);
                        for _ in 0..2 {
                            if let Some(&c) = chars.peek() {
                                if c.is_ascii_digit() && c < '8' {
                                    octal.push(c);
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                        }
                        if let Ok(val) = u8::from_str_radix(&octal, 8) {
                            result.push(val);
                        }
                    }
                    Some(other) => {
                        result.push(b'\\');
                        let mut buf = [0u8; 4];
                        result.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
                    }
                    None => result.push(b'\\'),
                }
            } else {
                let mut buf = [0u8; 4];
                result.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
        }
        result
    }

    fn unescape_go_char(s: &str) -> Result<char, Error> {
        let mut chars = s.chars();
        match chars.next() {
            Some('\\') => match chars.next() {
                Some('n') => Ok('\n'),
                Some('t') => Ok('\t'),
                Some('r') => Ok('\r'),
                Some('\\') => Ok('\\'),
                Some('\'') => Ok('\''),
                Some('"') => Ok('"'),
                Some('0') => Ok('\0'),
                Some('a') => Ok('\x07'),
                Some('b') => Ok('\x08'),
                Some('f') => Ok('\x0C'),
                Some('v') => Ok('\x0B'),
                Some(other) => Err(Error::SyntaxError(format!(
                    "invalid escape sequence: \\{}",
                    other
                ))),
                None => Err(Error::SyntaxError(
                    "incomplete escape sequence".to_string(),
                )),
            },
            Some(c) => Ok(c),
            None => Err(Error::SyntaxError(
                "empty character literal".to_string(),
            )),
        }
    }

    fn compile_ident(
        &mut self,
        ident: &ast::Ident,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        match ident.name.as_str() {
            "true" => {
                out.push(Instruction::I32Const(1));
                return Ok(());
            }
            "false" => {
                out.push(Instruction::I32Const(0));
                return Ok(());
            }
            "nil" => {
                out.push(Instruction::I32Const(0));
                return Ok(());
            }
            _ => {}
        }

        if let Some(&(ptr_local, len_local)) = locals.string_locals.get(&ident.name) {
            out.push(Instruction::LocalGet(ptr_local));
            out.push(Instruction::LocalGet(len_local));
            return Ok(());
        }

        if let Some(idx) = locals.find(&ident.name) {
            out.push(Instruction::LocalGet(idx));
            return Ok(());
        }

        // Check if this is a captured variable from an outer closure scope
        if let Some(cc) = &mut self.closure_captures {
            // Already captured?
            if let Some(cap) = cc.captures.iter().find(|c| c.name == ident.name) {
                let env_offset = cap.env_offset as u64;
                let vt = cap.val_type;
                out.push(Instruction::LocalGet(0)); // env_ptr is param 0
                match vt {
                    ValType::I64 => out.push(Instruction::I64Load(MemArg {
                        offset: env_offset,
                        align: 3,
                        memory_index: 0,
                    })),
                    ValType::F64 => out.push(Instruction::F64Load(MemArg {
                        offset: env_offset,
                        align: 3,
                        memory_index: 0,
                    })),
                    ValType::F32 => out.push(Instruction::F32Load(MemArg {
                        offset: env_offset,
                        align: 2,
                        memory_index: 0,
                    })),
                    _ => out.push(Instruction::I32Load(MemArg {
                        offset: env_offset,
                        align: 2,
                        memory_index: 0,
                    })),
                }
                return Ok(());
            }

            // Check if it exists in outer locals
            let found = cc
                .outer_locals
                .iter()
                .enumerate()
                .find(|(_, (n, _))| n == &ident.name)
                .map(|(i, (_, vt))| (i as u32, *vt));

            if let Some((outer_idx, vt)) = found {
                let env_offset = aligned_capture_env_offset(&cc.captures, vt);
                cc.captures.push(CapturedVar {
                    name: ident.name.clone(),
                    val_type: vt,
                    outer_local_idx: outer_idx,
                    env_offset,
                });
                out.push(Instruction::LocalGet(0)); // env_ptr is param 0
                match vt {
                    ValType::I64 => out.push(Instruction::I64Load(MemArg {
                        offset: env_offset as u64,
                        align: 3,
                        memory_index: 0,
                    })),
                    ValType::F64 => out.push(Instruction::F64Load(MemArg {
                        offset: env_offset as u64,
                        align: 3,
                        memory_index: 0,
                    })),
                    ValType::F32 => out.push(Instruction::F32Load(MemArg {
                        offset: env_offset as u64,
                        align: 2,
                        memory_index: 0,
                    })),
                    _ => out.push(Instruction::I32Load(MemArg {
                        offset: env_offset as u64,
                        align: 2,
                        memory_index: 0,
                    })),
                }
                return Ok(());
            }
        }

        if let Some(cv) = self.constants.get(&ident.name) {
            match cv {
                ConstValue::I64(v) => out.push(Instruction::I64Const(*v)),
                ConstValue::F64(v) => out.push(Instruction::F64Const(*v)),
                ConstValue::Bool(v) => out.push(Instruction::I32Const(*v as i32)),
                ConstValue::Str(s) => {
                    let bytes = s.as_bytes();
                    let len = bytes.len() as i32;
                    let ptr_local = locals.add_local(
                        &format!("__const_str_ptr_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::I32Const(len));
                    out.push(Instruction::Call(self.alloc_func_idx()?));
                    out.push(Instruction::LocalTee(ptr_local));

                    for (i, &byte) in bytes.iter().enumerate() {
                        out.push(Instruction::LocalGet(ptr_local));
                        out.push(Instruction::I32Const(byte as i32));
                        out.push(Instruction::I32Store8(MemArg {
                            offset: i as u64,
                            align: 0,
                            memory_index: 0,
                        }));
                    }

                    out.push(Instruction::LocalGet(ptr_local));
                    out.push(Instruction::I32Const(len));
                }
                ConstValue::Complex128(real, imag) => {
                    let total_size: i32 = 16;
                    let float_align: u32 = 3;
                    let ptr_local = locals.add_local(
                        &format!("__const_cmplx_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::I32Const(total_size));
                    out.push(Instruction::Call(self.alloc_func_idx()?));
                    out.push(Instruction::LocalSet(ptr_local));

                    out.push(Instruction::LocalGet(ptr_local));
                    out.push(Instruction::F64Const(*real));
                    out.push(Instruction::F64Store(MemArg { offset: 0, align: float_align, memory_index: 0 }));

                    out.push(Instruction::LocalGet(ptr_local));
                    out.push(Instruction::F64Const(*imag));
                    out.push(Instruction::F64Store(MemArg { offset: 8, align: float_align, memory_index: 0 }));

                    out.push(Instruction::LocalGet(ptr_local));
                }
            }
            return Ok(());
        }

        if let Some(&(global_idx, _vt)) = self.global_vars.get(&ident.name) {
            // String globals use two globals: ptr (global_idx) and len (global_idx+1)
            let len_key = format!("{}_1", ident.name);
            if let Some(&(len_global_idx, _)) = self.global_vars.get(&len_key) {
                out.push(Instruction::GlobalGet(global_idx));
                out.push(Instruction::GlobalGet(len_global_idx));
            } else {
                out.push(Instruction::GlobalGet(global_idx));
            }
            return Ok(());
        }

        let is_type_or_package = self.struct_defs.contains_key(&ident.name)
            || self.is_known_package(&ident.name);
        if is_type_or_package {
            out.push(Instruction::I64Const(0));
            return Ok(());
        }

        if let Some(fi) = self.functions.iter().find(|f| f.name == ident.name) {
            let idx = fi.wasm_func_idx;
            out.push(Instruction::I64Const(idx as i64));
            self.last_func_value_idx = Some(idx);
            return Ok(());
        }

        Err(Error::InternalError(format!(
            "undefined identifier: {}",
            ident.name
        )))
    }

    fn is_known_package(&self, name: &str) -> bool {
        matches!(
            name,
            "fmt" | "math" | "strings" | "strconv" | "sort" | "unicode"
                | "bytes" | "errors" | "encoding"
        )
    }

    fn is_complex64_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> bool {
        if let ast::Expression::Ident(ident) = expr {
            locals.get_var_struct_type(&ident.name) == Some("__complex64")
        } else if let ast::Expression::Call(call) = expr {
            if let ast::Expression::Ident(ident) = call.func.as_ref() {
                if ident.name == "complex" {
                    if let Some(arg) = call.args.first() {
                        return self.infer_val_type(arg, locals) == ValType::F32;
                    }
                }
            }
            false
        } else {
            false
        }
    }

    fn is_complex_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> bool {
        if let ast::Expression::Ident(ident) = expr {
            let st = locals.get_var_struct_type(&ident.name);
            st == Some("__complex64") || st == Some("__complex128")
        } else if let ast::Expression::Call(call) = expr {
            if let ast::Expression::Ident(ident) = call.func.as_ref() {
                return ident.name == "complex";
            }
            false
        } else if let ast::Expression::BasicLit(lit) = expr {
            lit.kind == LitKind::Imag
        } else {
            false
        }
    }

    fn emit_complex_binop(
        &mut self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        op: Operator,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let is_c64 = self.is_complex64_expr(lhs, locals);
        let float_vt = if is_c64 { ValType::F32 } else { ValType::F64 };
        let imag_offset = if is_c64 { 4u64 } else { 8u64 };
        let float_align = if is_c64 { 2u32 } else { 3u32 };
        let total_size = if is_c64 { 8i32 } else { 16i32 };

        let ar = locals.add_local(&format!("__cx_ar_{}", locals.locals.len()), float_vt);
        let ai = locals.add_local(&format!("__cx_ai_{}", locals.locals.len()), float_vt);
        let br = locals.add_local(&format!("__cx_br_{}", locals.locals.len()), float_vt);
        let bi = locals.add_local(&format!("__cx_bi_{}", locals.locals.len()), float_vt);

        // Load lhs real and imag parts
        self.compile_expression(lhs, out, locals)?;
        let lptr = locals.add_local(&format!("__cx_lp_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalSet(lptr));
        out.push(Instruction::LocalGet(lptr));
        if is_c64 {
            out.push(Instruction::F32Load(MemArg { offset: 0, align: float_align, memory_index: 0 }));
        } else {
            out.push(Instruction::F64Load(MemArg { offset: 0, align: float_align, memory_index: 0 }));
        }
        out.push(Instruction::LocalSet(ar));
        out.push(Instruction::LocalGet(lptr));
        if is_c64 {
            out.push(Instruction::F32Load(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
        } else {
            out.push(Instruction::F64Load(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
        }
        out.push(Instruction::LocalSet(ai));

        // Load rhs real and imag parts
        self.compile_expression(rhs, out, locals)?;
        let rptr = locals.add_local(&format!("__cx_rp_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalSet(rptr));
        out.push(Instruction::LocalGet(rptr));
        if is_c64 {
            out.push(Instruction::F32Load(MemArg { offset: 0, align: float_align, memory_index: 0 }));
        } else {
            out.push(Instruction::F64Load(MemArg { offset: 0, align: float_align, memory_index: 0 }));
        }
        out.push(Instruction::LocalSet(br));
        out.push(Instruction::LocalGet(rptr));
        if is_c64 {
            out.push(Instruction::F32Load(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
        } else {
            out.push(Instruction::F64Load(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
        }
        out.push(Instruction::LocalSet(bi));

        match op {
            Operator::Equal => {
                // a.r == b.r && a.i == b.i
                out.push(Instruction::LocalGet(ar));
                out.push(Instruction::LocalGet(br));
                if is_c64 { out.push(Instruction::F32Eq); } else { out.push(Instruction::F64Eq); }
                out.push(Instruction::LocalGet(ai));
                out.push(Instruction::LocalGet(bi));
                if is_c64 { out.push(Instruction::F32Eq); } else { out.push(Instruction::F64Eq); }
                out.push(Instruction::I32And);
                return Ok(());
            }
            Operator::NotEqual => {
                // a.r != b.r || a.i != b.i
                out.push(Instruction::LocalGet(ar));
                out.push(Instruction::LocalGet(br));
                if is_c64 { out.push(Instruction::F32Ne); } else { out.push(Instruction::F64Ne); }
                out.push(Instruction::LocalGet(ai));
                out.push(Instruction::LocalGet(bi));
                if is_c64 { out.push(Instruction::F32Ne); } else { out.push(Instruction::F64Ne); }
                out.push(Instruction::I32Or);
                return Ok(());
            }
            _ => {}
        }

        // Arithmetic: allocate result
        let res_r = locals.add_local(&format!("__cx_rr_{}", locals.locals.len()), float_vt);
        let res_i = locals.add_local(&format!("__cx_ri_{}", locals.locals.len()), float_vt);

        match op {
            Operator::Add => {
                // (ar+br, ai+bi)
                out.push(Instruction::LocalGet(ar));
                out.push(Instruction::LocalGet(br));
                if is_c64 { out.push(Instruction::F32Add); } else { out.push(Instruction::F64Add); }
                out.push(Instruction::LocalSet(res_r));
                out.push(Instruction::LocalGet(ai));
                out.push(Instruction::LocalGet(bi));
                if is_c64 { out.push(Instruction::F32Add); } else { out.push(Instruction::F64Add); }
                out.push(Instruction::LocalSet(res_i));
            }
            Operator::Sub => {
                // (ar-br, ai-bi)
                out.push(Instruction::LocalGet(ar));
                out.push(Instruction::LocalGet(br));
                if is_c64 { out.push(Instruction::F32Sub); } else { out.push(Instruction::F64Sub); }
                out.push(Instruction::LocalSet(res_r));
                out.push(Instruction::LocalGet(ai));
                out.push(Instruction::LocalGet(bi));
                if is_c64 { out.push(Instruction::F32Sub); } else { out.push(Instruction::F64Sub); }
                out.push(Instruction::LocalSet(res_i));
            }
            Operator::Star => {
                // (ar*br - ai*bi, ar*bi + ai*br)
                out.push(Instruction::LocalGet(ar));
                out.push(Instruction::LocalGet(br));
                if is_c64 { out.push(Instruction::F32Mul); } else { out.push(Instruction::F64Mul); }
                out.push(Instruction::LocalGet(ai));
                out.push(Instruction::LocalGet(bi));
                if is_c64 { out.push(Instruction::F32Mul); } else { out.push(Instruction::F64Mul); }
                if is_c64 { out.push(Instruction::F32Sub); } else { out.push(Instruction::F64Sub); }
                out.push(Instruction::LocalSet(res_r));

                out.push(Instruction::LocalGet(ar));
                out.push(Instruction::LocalGet(bi));
                if is_c64 { out.push(Instruction::F32Mul); } else { out.push(Instruction::F64Mul); }
                out.push(Instruction::LocalGet(ai));
                out.push(Instruction::LocalGet(br));
                if is_c64 { out.push(Instruction::F32Mul); } else { out.push(Instruction::F64Mul); }
                if is_c64 { out.push(Instruction::F32Add); } else { out.push(Instruction::F64Add); }
                out.push(Instruction::LocalSet(res_i));
            }
            Operator::Quo => {
                // denom = br*br + bi*bi
                // real = (ar*br + ai*bi) / denom
                // imag = (ai*br - ar*bi) / denom
                let denom = locals.add_local(&format!("__cx_d_{}", locals.locals.len()), float_vt);
                out.push(Instruction::LocalGet(br));
                out.push(Instruction::LocalGet(br));
                if is_c64 { out.push(Instruction::F32Mul); } else { out.push(Instruction::F64Mul); }
                out.push(Instruction::LocalGet(bi));
                out.push(Instruction::LocalGet(bi));
                if is_c64 { out.push(Instruction::F32Mul); } else { out.push(Instruction::F64Mul); }
                if is_c64 { out.push(Instruction::F32Add); } else { out.push(Instruction::F64Add); }
                out.push(Instruction::LocalSet(denom));

                // real
                out.push(Instruction::LocalGet(ar));
                out.push(Instruction::LocalGet(br));
                if is_c64 { out.push(Instruction::F32Mul); } else { out.push(Instruction::F64Mul); }
                out.push(Instruction::LocalGet(ai));
                out.push(Instruction::LocalGet(bi));
                if is_c64 { out.push(Instruction::F32Mul); } else { out.push(Instruction::F64Mul); }
                if is_c64 { out.push(Instruction::F32Add); } else { out.push(Instruction::F64Add); }
                out.push(Instruction::LocalGet(denom));
                if is_c64 { out.push(Instruction::F32Div); } else { out.push(Instruction::F64Div); }
                out.push(Instruction::LocalSet(res_r));

                // imag
                out.push(Instruction::LocalGet(ai));
                out.push(Instruction::LocalGet(br));
                if is_c64 { out.push(Instruction::F32Mul); } else { out.push(Instruction::F64Mul); }
                out.push(Instruction::LocalGet(ar));
                out.push(Instruction::LocalGet(bi));
                if is_c64 { out.push(Instruction::F32Mul); } else { out.push(Instruction::F64Mul); }
                if is_c64 { out.push(Instruction::F32Sub); } else { out.push(Instruction::F64Sub); }
                out.push(Instruction::LocalGet(denom));
                if is_c64 { out.push(Instruction::F32Div); } else { out.push(Instruction::F64Div); }
                out.push(Instruction::LocalSet(res_i));
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported operator {:?} for complex numbers",
                    op
                )));
            }
        }

        // Allocate and store result complex
        out.push(Instruction::I32Const(total_size));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let res_ptr = locals.add_local(&format!("__cx_rptr_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalSet(res_ptr));

        out.push(Instruction::LocalGet(res_ptr));
        out.push(Instruction::LocalGet(res_r));
        if is_c64 {
            out.push(Instruction::F32Store(MemArg { offset: 0, align: float_align, memory_index: 0 }));
        } else {
            out.push(Instruction::F64Store(MemArg { offset: 0, align: float_align, memory_index: 0 }));
        }
        out.push(Instruction::LocalGet(res_ptr));
        out.push(Instruction::LocalGet(res_i));
        if is_c64 {
            out.push(Instruction::F32Store(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
        } else {
            out.push(Instruction::F64Store(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
        }

        out.push(Instruction::LocalGet(res_ptr));
        Ok(())
    }

    fn is_selector_slice_field(&self, sel: &ast::Selector, locals: &LocalAlloc) -> bool {
        if let Some(type_name) = self.infer_struct_type_from_expr(sel.x.as_ref(), locals) {
            if let Some(struct_def) = self.struct_defs.get(&type_name) {
                if let Some(field) = struct_def.find_field(&sel.sel.name) {
                    return field.go_type_tag.as_deref() == Some("__slice");
                }
            }
        }
        false
    }

    fn is_selector_map_field(&self, sel: &ast::Selector, locals: &LocalAlloc) -> bool {
        if let Some(type_name) = self.infer_struct_type_from_expr(sel.x.as_ref(), locals) {
            if let Some(struct_def) = self.struct_defs.get(&type_name) {
                if let Some(field) = struct_def.find_field(&sel.sel.name) {
                    return field.go_type_tag.as_deref() == Some("__map");
                }
            }
        }
        false
    }


    fn is_string_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> bool {
        match expr {
            ast::Expression::BasicLit(lit) => lit.kind == LitKind::String,
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    if ident.name == "string" || ident.name == "recover" {
                        return true;
                    }
                    if (ident.name == "min" || ident.name == "max") && !call.args.is_empty() {
                        return self.is_string_expr(&call.args[0], locals);
                    }
                    false
                } else if let ast::Expression::Selector(sel) = call.func.as_ref() {
                    if let ast::Expression::Ident(recv) = sel.x.as_ref() {
                        if self.is_interface_var(&recv.name, locals) {
                            // Check if the method returns a string by looking at implementations
                            let method_name = &sel.sel.name;
                            for fi in &self.functions {
                                if fi.recv_type.is_some() && fi.name.ends_with(&format!(".{}", method_name)) {
                                    return fi.results.len() == 2
                                        && fi.results[0] == WasmType::I32
                                        && fi.results[1] == WasmType::I32;
                                }
                            }
                        }
                    }
                    false
                } else {
                    false
                }
            }
            ast::Expression::Ident(ident) => {
                locals.get_var_struct_type(&ident.name) == Some("__string")
                    || self.global_vars.contains_key(&format!("{}_1", ident.name))
            }
            ast::Expression::Paren(p) => self.is_string_expr(&p.expr, locals),
            ast::Expression::Operation(op) if op.op == Operator::Add && op.y.is_some() => {
                self.is_string_expr(&op.x, locals)
                    && self.is_string_expr(op.y.as_ref().unwrap(), locals)
            }
            ast::Expression::Slice(slice) => {
                self.is_string_expr(&slice.left, locals)
            }
            _ => false,
        }
    }

    fn is_unsigned_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> bool {
        match expr {
            ast::Expression::Ident(ident) => locals.unsigned_vars.contains(&ident.name),
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    matches!(ident.name.as_str(), "uint" | "uint64" | "uint32" | "uint8" | "uint16" | "byte")
                } else {
                    false
                }
            }
            ast::Expression::Paren(p) => self.is_unsigned_expr(&p.expr, locals),
            ast::Expression::Operation(op) if op.y.is_some() => {
                self.is_unsigned_expr(&op.x, locals)
                    || self.is_unsigned_expr(op.y.as_ref().unwrap(), locals)
            }
            _ => false,
        }
    }

    fn is_unsigned_type_name(name: &str) -> bool {
        matches!(name, "uint" | "uint64" | "uint32" | "uint8" | "uint16" | "byte" | "uintptr")
    }

    fn find_captured_closure(&self, name: &str) -> Option<(u32, u32, Vec<(String, u32, ValType)>)> {
        let cc = self.closure_captures.as_ref()?;
        let (func_idx, env_local) = cc.outer_closure_info.get(name)?;
        let env_captures = cc.outer_closure_env_captures.get(name).cloned().unwrap_or_default();
        Some((*func_idx, *env_local, env_captures))
    }

    fn find_or_add_capture(&mut self, name: &str) -> Option<(u32, ValType)> {
        let cc = self.closure_captures.as_mut()?;

        if let Some(cap) = cc.captures.iter().find(|c| c.name == name) {
            return Some((cap.env_offset, cap.val_type));
        }

        let found = cc.outer_locals.iter().enumerate()
            .find(|(_, (n, _))| n == name)
            .map(|(i, (_, vt))| (i as u32, *vt));

        if let Some((outer_idx, vt)) = found {
            let env_offset = aligned_capture_env_offset(&cc.captures, vt);
            cc.captures.push(CapturedVar {
                name: name.to_string(),
                val_type: vt,
                outer_local_idx: outer_idx,
                env_offset,
            });
            Some((env_offset, vt))
        } else {
            None
        }
    }

    fn get_struct_type_of_expr<'b>(&self, expr: &ast::Expression, locals: &'b LocalAlloc) -> Option<&'b str> {
        match expr {
            ast::Expression::Ident(ident) => {
                let st = locals.get_var_struct_type(&ident.name)?;
                if self.struct_defs.contains_key(st) {
                    Some(st)
                } else {
                    None
                }
            }
            ast::Expression::Paren(p) => self.get_struct_type_of_expr(&p.expr, locals),
            _ => None,
        }
    }

    fn get_comparable_struct_type(
        &self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        locals: &LocalAlloc,
    ) -> Option<String> {
        let lhs_type = self.get_struct_type_of_expr(lhs, locals)?;
        let rhs_type = self.get_struct_type_of_expr(rhs, locals)?;
        if lhs_type != rhs_type {
            return None;
        }
        let sdef = self.struct_defs.get(lhs_type)?;
        for field in &sdef.fields {
            if field.go_type_tag.as_deref() == Some("__slice")
                || field.go_type_tag.as_deref() == Some("__map")
            {
                return None;
            }
        }
        Some(lhs_type.to_string())
    }

    fn emit_struct_compare(
        &mut self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        struct_type: &str,
        op: Operator,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let sdef = self.struct_defs.get(struct_type).cloned().ok_or_else(|| {
            Error::InternalError(format!("struct type '{}' not found", struct_type))
        })?;

        self.compile_expression(lhs, out, locals)?;
        let lhs_ptr = locals.add_local(
            &format!("__scmp_l_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(lhs_ptr));

        self.compile_expression(rhs, out, locals)?;
        let rhs_ptr = locals.add_local(
            &format!("__scmp_r_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(rhs_ptr));

        let result = locals.add_local(
            &format!("__scmp_res_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::I32Const(1)); // assume equal
        out.push(Instruction::LocalSet(result));

        out.push(Instruction::Block(BlockType::Empty));
        for field in &sdef.fields {
            let offset = field.offset as u64;
            if field.go_type_tag.as_deref() == Some("__string") {
                // Compare string fields: load (ptr, len) from each side
                out.push(Instruction::LocalGet(lhs_ptr));
                out.push(Instruction::I32Load(MemArg { offset, align: 2, memory_index: 0 }));
                let l_sptr = locals.add_local(&format!("__scf_lp_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalSet(l_sptr));
                out.push(Instruction::LocalGet(lhs_ptr));
                out.push(Instruction::I32Load(MemArg { offset: offset + 4, align: 2, memory_index: 0 }));
                let l_slen = locals.add_local(&format!("__scf_ll_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalSet(l_slen));

                out.push(Instruction::LocalGet(rhs_ptr));
                out.push(Instruction::I32Load(MemArg { offset, align: 2, memory_index: 0 }));
                let r_sptr = locals.add_local(&format!("__scf_rp_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalSet(r_sptr));
                out.push(Instruction::LocalGet(rhs_ptr));
                out.push(Instruction::I32Load(MemArg { offset: offset + 4, align: 2, memory_index: 0 }));
                let r_slen = locals.add_local(&format!("__scf_rl_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalSet(r_slen));

                // Check lengths first
                out.push(Instruction::LocalGet(l_slen));
                out.push(Instruction::LocalGet(r_slen));
                out.push(Instruction::I32Ne);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(result));
                out.push(Instruction::Br(1)); // break out of block
                out.push(Instruction::End);

                // Byte-by-byte comparison
                let si = locals.add_local(&format!("__scf_si_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(si));
                out.push(Instruction::Block(BlockType::Empty));
                out.push(Instruction::Loop(BlockType::Empty));
                out.push(Instruction::LocalGet(si));
                out.push(Instruction::LocalGet(l_slen));
                out.push(Instruction::I32GeU);
                out.push(Instruction::BrIf(1));
                out.push(Instruction::LocalGet(l_sptr));
                out.push(Instruction::LocalGet(si));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                out.push(Instruction::LocalGet(r_sptr));
                out.push(Instruction::LocalGet(si));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                out.push(Instruction::I32Ne);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(result));
                out.push(Instruction::Br(3)); // break out of outer block
                out.push(Instruction::End);
                out.push(Instruction::LocalGet(si));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(si));
                out.push(Instruction::Br(0));
                out.push(Instruction::End); // loop
                out.push(Instruction::End); // block
            } else {
                // Numeric field comparison
                let (load_instr_l, load_instr_r, ne_instr) = match field.wasm_type {
                    WasmType::I64 => (
                        Instruction::I64Load(MemArg { offset, align: 3, memory_index: 0 }),
                        Instruction::I64Load(MemArg { offset, align: 3, memory_index: 0 }),
                        Instruction::I64Ne,
                    ),
                    WasmType::F64 => (
                        Instruction::F64Load(MemArg { offset, align: 3, memory_index: 0 }),
                        Instruction::F64Load(MemArg { offset, align: 3, memory_index: 0 }),
                        Instruction::F64Ne,
                    ),
                    WasmType::F32 => (
                        Instruction::F32Load(MemArg { offset, align: 2, memory_index: 0 }),
                        Instruction::F32Load(MemArg { offset, align: 2, memory_index: 0 }),
                        Instruction::F32Ne,
                    ),
                    WasmType::I32 => (
                        Instruction::I32Load(MemArg { offset, align: 2, memory_index: 0 }),
                        Instruction::I32Load(MemArg { offset, align: 2, memory_index: 0 }),
                        Instruction::I32Ne,
                    ),
                };
                out.push(Instruction::LocalGet(lhs_ptr));
                out.push(load_instr_l);
                out.push(Instruction::LocalGet(rhs_ptr));
                out.push(load_instr_r);
                out.push(ne_instr);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(result));
                out.push(Instruction::Br(1));
                out.push(Instruction::End);
            }
        }
        out.push(Instruction::End); // block

        out.push(Instruction::LocalGet(result));
        if op == Operator::NotEqual {
            out.push(Instruction::I32Eqz);
        }
        Ok(())
    }

    fn get_array_type_of_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> Option<(ValType, u32)> {
        match expr {
            ast::Expression::Ident(ident) => locals.array_info.get(&ident.name).copied(),
            ast::Expression::Paren(p) => self.get_array_type_of_expr(&p.expr, locals),
            _ => None,
        }
    }

    fn get_comparable_array_type(
        &self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        locals: &LocalAlloc,
    ) -> Option<(ValType, u32)> {
        let (lhs_vt, lhs_len) = self.get_array_type_of_expr(lhs, locals)?;
        let (rhs_vt, rhs_len) = self.get_array_type_of_expr(rhs, locals)?;
        if lhs_vt != rhs_vt || lhs_len != rhs_len {
            return None;
        }
        Some((lhs_vt, lhs_len))
    }

    fn emit_array_compare(
        &mut self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        elem_vt: ValType,
        arr_len: u32,
        op: Operator,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        self.compile_expression(lhs, out, locals)?;
        let lhs_ptr = locals.add_local(
            &format!("__acmp_l_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(lhs_ptr));

        self.compile_expression(rhs, out, locals)?;
        let rhs_ptr = locals.add_local(
            &format!("__acmp_r_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(rhs_ptr));

        let result = locals.add_local(
            &format!("__acmp_res_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::I32Const(1)); // assume equal
        out.push(Instruction::LocalSet(result));

        let elem_size = match elem_vt {
            ValType::I64 | ValType::F64 => 8u32,
            _ => 4u32,
        };

        let idx = locals.add_local(
            &format!("__acmp_i_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(idx));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));

        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Const(arr_len as i32));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        // Load lhs[idx]
        out.push(Instruction::LocalGet(lhs_ptr));
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Const(elem_size as i32));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        let (load_instr, ne_instr) = match elem_vt {
            ValType::I64 => (
                Instruction::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }),
                Instruction::I64Ne,
            ),
            ValType::F64 => (
                Instruction::F64Load(MemArg { offset: 0, align: 3, memory_index: 0 }),
                Instruction::F64Ne,
            ),
            ValType::F32 => (
                Instruction::F32Load(MemArg { offset: 0, align: 2, memory_index: 0 }),
                Instruction::F32Ne,
            ),
            _ => (
                Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }),
                Instruction::I32Ne,
            ),
        };
        out.push(load_instr.clone());

        // Load rhs[idx]
        out.push(Instruction::LocalGet(rhs_ptr));
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Const(elem_size as i32));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(load_instr);

        out.push(ne_instr);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(result));
        out.push(Instruction::Br(2)); // break outer block
        out.push(Instruction::End);

        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(idx));
        out.push(Instruction::Br(0)); // continue loop
        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block

        out.push(Instruction::LocalGet(result));
        if op == Operator::NotEqual {
            out.push(Instruction::I32Eqz);
        }
        Ok(())
    }

    fn compile_operation(
        &mut self,
        op: &ast::Operation,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        if let Some(ref y) = op.y {
            if op.op == Operator::Add
                && self.is_string_expr(&op.x, locals)
                && self.is_string_expr(y, locals)
            {
                return self.emit_string_concat(&op.x, y, out, locals);
            }

            if matches!(op.op, Operator::Equal | Operator::NotEqual | Operator::Less | Operator::Greater | Operator::LessEqual | Operator::GreaterEqual)
                && self.is_string_expr(&op.x, locals)
                && self.is_string_expr(y, locals)
            {
                return self.emit_string_compare(&op.x, y, op.op, out, locals);
            }

            // Complex number arithmetic
            if self.is_complex_expr(&op.x, locals) && self.is_complex_expr(y, locals) {
                return self.emit_complex_binop(&op.x, y, op.op, out, locals);
            }

            // Interface nil comparison: err == nil or err != nil
            if op.op == Operator::Equal || op.op == Operator::NotEqual {
                let (iface_name, is_nil_cmp) = self.check_interface_nil_cmp(&op.x, y, locals);
                if is_nil_cmp {
                    if let Some(tid_local) = self.get_iface_type_id_local(&iface_name) {
                        out.push(Instruction::LocalGet(tid_local));
                        if op.op == Operator::Equal {
                            out.push(Instruction::I32Eqz);
                        } else {
                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::I32Ne);
                        }
                        return Ok(());
                    }
                }
            }

            // Struct comparison: s1 == s2 or s1 != s2
            if matches!(op.op, Operator::Equal | Operator::NotEqual) {
                if let Some(struct_type) = self.get_comparable_struct_type(&op.x, y, locals) {
                    return self.emit_struct_compare(&op.x, y, &struct_type, op.op, out, locals);
                }
                if let Some((elem_vt, arr_len)) = self.get_comparable_array_type(&op.x, y, locals) {
                    return self.emit_array_compare(&op.x, y, elem_vt, arr_len, op.op, out, locals);
                }
            }

            if op.op == Operator::AndAnd {
                self.compile_expression(&op.x, out, locals)?;
                out.push(Instruction::If(BlockType::Result(ValType::I32)));
                self.compile_expression(y, out, locals)?;
                out.push(Instruction::Else);
                out.push(Instruction::I32Const(0));
                out.push(Instruction::End);
                return Ok(());
            }

            if op.op == Operator::OrOr {
                self.compile_expression(&op.x, out, locals)?;
                out.push(Instruction::If(BlockType::Result(ValType::I32)));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::Else);
                self.compile_expression(y, out, locals)?;
                out.push(Instruction::End);
                return Ok(());
            }

            let lhs_type = self.infer_val_type(&op.x, locals);
            let rhs_type = self.infer_val_type(y, locals);

            if lhs_type == ValType::F32 && rhs_type == ValType::F32 {
                self.compile_expression(&op.x, out, locals)?;
                self.compile_expression(y, out, locals)?;
                return self.emit_f32_op(op.op, out);
            }

            if lhs_type == ValType::F32 || rhs_type == ValType::F32 {
                let mut lhs_buf = Vec::new();
                self.compile_expression(&op.x, &mut lhs_buf, locals)?;
                let mut rhs_buf = Vec::new();
                self.compile_expression(y, &mut rhs_buf, locals)?;

                out.extend(lhs_buf);
                if lhs_type != ValType::F32 {
                    if lhs_type == ValType::I32 {
                        out.push(Instruction::F32ConvertI32S);
                    } else if lhs_type == ValType::I64 {
                        out.push(Instruction::F32ConvertI64S);
                    }
                }
                out.extend(rhs_buf);
                if rhs_type != ValType::F32 {
                    if rhs_type == ValType::I32 {
                        out.push(Instruction::F32ConvertI32S);
                    } else if rhs_type == ValType::I64 {
                        out.push(Instruction::F32ConvertI64S);
                    }
                }
                return self.emit_f32_op(op.op, out);
            }

            if lhs_type == ValType::F64 || rhs_type == ValType::F64 {
                let mut lhs_buf = Vec::new();
                self.compile_expression(&op.x, &mut lhs_buf, locals)?;
                let mut rhs_buf = Vec::new();
                self.compile_expression(y, &mut rhs_buf, locals)?;

                out.extend(lhs_buf);
                if lhs_type != ValType::F64 {
                    if lhs_type == ValType::I32 {
                        out.push(Instruction::F64ConvertI32S);
                    } else {
                        out.push(Instruction::F64ConvertI64S);
                    }
                }
                out.extend(rhs_buf);
                if rhs_type != ValType::F64 {
                    if rhs_type == ValType::I32 {
                        out.push(Instruction::F64ConvertI32S);
                    } else {
                        out.push(Instruction::F64ConvertI64S);
                    }
                }
                return self.emit_f64_op(op.op, out);
            }

            let is_unsigned = self.is_unsigned_expr(&op.x, locals)
                || self.is_unsigned_expr(y, locals);

            if lhs_type == ValType::I32 && rhs_type == ValType::I32 {
                self.compile_expression(&op.x, out, locals)?;
                self.compile_expression(y, out, locals)?;
                return self.emit_i32_op_signed(op.op, !is_unsigned, out);
            }

            if lhs_type == ValType::I32 && rhs_type == ValType::I64 {
                let mut lhs_buf = Vec::new();
                self.compile_expression(&op.x, &mut lhs_buf, locals)?;
                let mut rhs_buf = Vec::new();
                self.compile_expression(y, &mut rhs_buf, locals)?;

                out.extend(lhs_buf);
                if is_unsigned {
                    out.push(Instruction::I64ExtendI32U);
                } else {
                    out.push(Instruction::I64ExtendI32S);
                }
                out.extend(rhs_buf);
            } else if lhs_type == ValType::I64 && rhs_type == ValType::I32 {
                self.compile_expression(&op.x, out, locals)?;
                let mut rhs_buf = Vec::new();
                self.compile_expression(y, &mut rhs_buf, locals)?;
                out.extend(rhs_buf);
                if is_unsigned {
                    out.push(Instruction::I64ExtendI32U);
                } else {
                    out.push(Instruction::I64ExtendI32S);
                }
            } else {
                self.compile_expression(&op.x, out, locals)?;
                self.compile_expression(y, out, locals)?;
            }

            return self.emit_i64_op_signed(op.op, !is_unsigned, out);
        }

        // Unary operations
        match op.op {
            Operator::Add => {
                self.compile_expression(&op.x, out, locals)?;
            }
            Operator::Sub => {
                if self.is_complex_expr(&op.x, locals) {
                    let is_c64 = self.is_complex64_expr(&op.x, locals);
                    let (total_size, float_align, imag_offset): (i32, u32, u64) =
                        if is_c64 { (8, 2, 4) } else { (16, 3, 8) };

                    self.compile_expression(&op.x, out, locals)?;
                    let src_ptr = locals.add_local(
                        &format!("__cneg_src_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(src_ptr));

                    out.push(Instruction::I32Const(total_size));
                    out.push(Instruction::Call(self.alloc_func_idx()?));
                    let res_ptr = locals.add_local(
                        &format!("__cneg_res_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(res_ptr));

                    // Negate real part
                    out.push(Instruction::LocalGet(res_ptr));
                    out.push(Instruction::LocalGet(src_ptr));
                    if is_c64 {
                        out.push(Instruction::F32Load(MemArg { offset: 0, align: float_align, memory_index: 0 }));
                        out.push(Instruction::F32Neg);
                        out.push(Instruction::F32Store(MemArg { offset: 0, align: float_align, memory_index: 0 }));
                    } else {
                        out.push(Instruction::F64Load(MemArg { offset: 0, align: float_align, memory_index: 0 }));
                        out.push(Instruction::F64Neg);
                        out.push(Instruction::F64Store(MemArg { offset: 0, align: float_align, memory_index: 0 }));
                    }

                    // Negate imaginary part
                    out.push(Instruction::LocalGet(res_ptr));
                    out.push(Instruction::LocalGet(src_ptr));
                    if is_c64 {
                        out.push(Instruction::F32Load(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
                        out.push(Instruction::F32Neg);
                        out.push(Instruction::F32Store(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
                    } else {
                        out.push(Instruction::F64Load(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
                        out.push(Instruction::F64Neg);
                        out.push(Instruction::F64Store(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
                    }

                    out.push(Instruction::LocalGet(res_ptr));
                } else {
                    let vt = self.infer_val_type(&op.x, locals);
                    match vt {
                        ValType::I64 => {
                            out.push(Instruction::I64Const(0));
                            self.compile_expression(&op.x, out, locals)?;
                            out.push(Instruction::I64Sub);
                        }
                        ValType::F64 => {
                            self.compile_expression(&op.x, out, locals)?;
                            out.push(Instruction::F64Neg);
                        }
                        ValType::I32 => {
                            out.push(Instruction::I32Const(0));
                            self.compile_expression(&op.x, out, locals)?;
                            out.push(Instruction::I32Sub);
                        }
                        ValType::F32 => {
                            self.compile_expression(&op.x, out, locals)?;
                            out.push(Instruction::F32Neg);
                        }
                        _ => {
                            return Err(Error::InternalError(format!(
                                "unsupported type for unary negation: {:?}",
                                vt
                            )));
                        }
                    }
                }
            }
            Operator::Not => {
                self.compile_expression(&op.x, out, locals)?;
                out.push(Instruction::I32Eqz);
            }
            Operator::Star => {
                self.compile_expression(&op.x, out, locals)?;
                let ptr_local = locals.add_local(
                    &format!("__deref_optr_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalTee(ptr_local));
                out.push(Instruction::I32Eqz);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::Unreachable);
                out.push(Instruction::End);
                out.push(Instruction::LocalGet(ptr_local));

                let deref_vt = self.infer_deref_type(&op.x, locals);
                let (_size, align) = Self::elem_size_and_align(deref_vt);
                let mem_arg = MemArg {
                    offset: 0,
                    align,
                    memory_index: 0,
                };
                match deref_vt {
                    ValType::I64 => out.push(Instruction::I64Load(mem_arg)),
                    ValType::F32 => out.push(Instruction::F32Load(mem_arg)),
                    ValType::F64 => out.push(Instruction::F64Load(mem_arg)),
                    _ => out.push(Instruction::I32Load(mem_arg)),
                }
            }
            Operator::Xor => {
                let vt = self.infer_val_type(&op.x, locals);
                self.compile_expression(&op.x, out, locals)?;
                match vt {
                    ValType::I64 => {
                        out.push(Instruction::I64Const(-1));
                        out.push(Instruction::I64Xor);
                    }
                    ValType::I32 => {
                        out.push(Instruction::I32Const(-1));
                        out.push(Instruction::I32Xor);
                    }
                    _ => {
                        return Err(Error::InternalError(format!(
                            "unsupported type for unary bitwise complement: {:?}",
                            vt
                        )));
                    }
                }
            }
            Operator::And => {
                // Address-of: &x
                let is_struct_var = if let ast::Expression::Ident(id) = &*op.x {
                    locals.get_var_struct_type(&id.name).map_or(false, |t| {
                        self.struct_defs.contains_key(t) || t == "__slice" || t == "__map"
                    })
                } else {
                    false
                };
                let is_composite = matches!(&*op.x, ast::Expression::CompositeLit(_));

                // &slice[i] or &array[i]: compute element address in-place
                let is_index_addr = if let ast::Expression::Index(idx) = &*op.x {
                    if let Some(ast::Expression::Ident(id)) = idx.left.as_deref() {
                        locals.get_var_struct_type(&id.name).map_or(false, |t| {
                            t == "__slice" || t == "__array"
                        }) || locals.array_info.contains_key(&id.name)
                            || locals.slice_elem_types.contains_key(&id.name)
                    } else { false }
                } else { false };

                if is_index_addr {
                    if let ast::Expression::Index(idx) = &*op.x {
                        let (elem_vt, _align) = self.compile_index_store_addr(idx, out, locals)?;
                        let _ = elem_vt;
                    }
                } else if is_struct_var || is_composite {
                    self.compile_expression(&op.x, out, locals)?;
                } else {
                    let val_vt = self.infer_val_type(&op.x, locals);
                    let (elem_size, align) = Self::elem_size_and_align(val_vt);
                    self.compile_expression(&op.x, out, locals)?;
                    let val_tmp = locals.add_local(
                        &format!("__addr_val_{}", locals.locals.len()),
                        val_vt,
                    );
                    out.push(Instruction::LocalSet(val_tmp));
                    out.push(Instruction::I32Const(elem_size));
                    out.push(Instruction::Call(self.alloc_func_idx()?));
                    let ptr_tmp = locals.add_local(
                        &format!("__addr_ptr_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(ptr_tmp));
                    out.push(Instruction::LocalGet(ptr_tmp));
                    out.push(Instruction::LocalGet(val_tmp));
                    Self::emit_typed_store(val_vt, 0, align, out);
                    out.push(Instruction::LocalGet(ptr_tmp));
                }
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported unary operator: {:?}",
                    op.op
                )));
            }
        }

        Ok(())
    }

    fn emit_string_concat(
        &mut self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        // Compile both strings: each pushes (ptr, len)
        self.compile_expression(lhs, out, locals)?;
        self.compile_expression(rhs, out, locals)?;

        let len2 = locals.add_local("__scat_len2", ValType::I32);
        let ptr2 = locals.add_local("__scat_ptr2", ValType::I32);
        let len1 = locals.add_local("__scat_len1", ValType::I32);
        let ptr1 = locals.add_local("__scat_ptr1", ValType::I32);

        out.push(Instruction::LocalSet(len2));
        out.push(Instruction::LocalSet(ptr2));
        out.push(Instruction::LocalSet(len1));
        out.push(Instruction::LocalSet(ptr1));

        // Total length (with overflow check)
        let total_len = locals.add_local("__scat_total", ValType::I32);
        out.push(Instruction::LocalGet(len1));
        out.push(Instruction::LocalGet(len2));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalTee(total_len));

        // Overflow: if total_len < len1 then the add wrapped around
        out.push(Instruction::LocalGet(len1));
        out.push(Instruction::I32LtU);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::Call(self.oom_func_idx));
        out.push(Instruction::Unreachable);
        out.push(Instruction::End);

        // Allocate buffer
        out.push(Instruction::LocalGet(total_len));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let new_ptr = locals.add_local("__scat_new", ValType::I32);
        out.push(Instruction::LocalSet(new_ptr));

        // Copy first string: memory.copy(new_ptr, ptr1, len1)
        out.push(Instruction::LocalGet(new_ptr));
        out.push(Instruction::LocalGet(ptr1));
        out.push(Instruction::LocalGet(len1));
        out.push(Instruction::MemoryCopy {
            dst_mem: 0,
            src_mem: 0,
        });

        // Copy second string: memory.copy(new_ptr + len1, ptr2, len2)
        out.push(Instruction::LocalGet(new_ptr));
        out.push(Instruction::LocalGet(len1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalGet(ptr2));
        out.push(Instruction::LocalGet(len2));
        out.push(Instruction::MemoryCopy {
            dst_mem: 0,
            src_mem: 0,
        });

        // Push result (ptr, len)
        out.push(Instruction::LocalGet(new_ptr));
        out.push(Instruction::LocalGet(total_len));

        Ok(())
    }

    /// Compile a single expression and convert its result to a (ptr, len) string pair on the stack.
    fn emit_expr_to_string_on_stack(
        &mut self,
        expr: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        if self.is_string_expr(expr, locals) {
            self.compile_expression(expr, out, locals)?;
            return Ok(());
        }

        // Detect bool literals
        if let ast::Expression::Ident(id) = expr {
            if id.name == "true" || id.name == "false" {
                self.emit_bool_to_string(id.name == "true", out, locals)?;
                return Ok(());
            }
        }

        let vt = self.infer_val_type(expr, locals);
        self.compile_expression(expr, out, locals)?;
        match vt {
            ValType::I64 | ValType::I32 => {
                if vt == ValType::I32 {
                    out.push(Instruction::I64ExtendI32S);
                }
                self.emit_i64_to_string(out, locals)?;
            }
            ValType::F64 => {
                self.emit_f64_to_string(out, locals)?;
            }
            ValType::F32 => {
                out.push(Instruction::F64PromoteF32);
                self.emit_f64_to_string(out, locals)?;
            }
            _ => {
                out.push(Instruction::Drop);
                out.push(Instruction::I32Const(0));
                out.push(Instruction::I32Const(0));
            }
        }
        Ok(())
    }

    /// Emit code to produce the string "true" or "false" as (ptr, len) on the stack.
    fn emit_bool_to_string(
        &mut self,
        val: bool,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let s = if val { b"true" as &[u8] } else { b"false" as &[u8] };
        let ptr_local = locals.add_local("__bts_ptr", ValType::I32);
        out.push(Instruction::I32Const(s.len() as i32));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        out.push(Instruction::LocalSet(ptr_local));
        for (i, &byte) in s.iter().enumerate() {
            out.push(Instruction::LocalGet(ptr_local));
            out.push(Instruction::I32Const(byte as i32));
            out.push(Instruction::I32Store8(MemArg { offset: i as u64, align: 0, memory_index: 0 }));
        }
        out.push(Instruction::LocalGet(ptr_local));
        out.push(Instruction::I32Const(s.len() as i32));
        Ok(())
    }

    /// Takes a (ptr, len) string on the stack and appends a newline, producing a new (ptr, len).
    fn emit_append_newline(
        &mut self,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let src_len = locals.add_local("__nl_src_len", ValType::I32);
        let src_ptr = locals.add_local("__nl_src_ptr", ValType::I32);
        out.push(Instruction::LocalSet(src_len));
        out.push(Instruction::LocalSet(src_ptr));

        let new_len = locals.add_local("__nl_new_len", ValType::I32);
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(new_len));

        let new_ptr = locals.add_local("__nl_new_ptr", ValType::I32);
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        out.push(Instruction::LocalSet(new_ptr));

        // Copy original string
        out.push(Instruction::LocalGet(new_ptr));
        out.push(Instruction::LocalGet(src_ptr));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

        // Write newline at end
        out.push(Instruction::LocalGet(new_ptr));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32Add);
        out.push(Instruction::I32Const(b'\n' as i32));
        out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

        out.push(Instruction::LocalGet(new_ptr));
        out.push(Instruction::LocalGet(new_len));
        Ok(())
    }

    fn emit_string_compare(
        &mut self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        op: Operator,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        self.compile_expression(lhs, out, locals)?;
        self.compile_expression(rhs, out, locals)?;

        let len2 = locals.add_local(&format!("__scmp_len2_{}", locals.locals.len()), ValType::I32);
        let ptr2 = locals.add_local(&format!("__scmp_ptr2_{}", locals.locals.len()), ValType::I32);
        let len1 = locals.add_local(&format!("__scmp_len1_{}", locals.locals.len()), ValType::I32);
        let ptr1 = locals.add_local(&format!("__scmp_ptr1_{}", locals.locals.len()), ValType::I32);

        out.push(Instruction::LocalSet(len2));
        out.push(Instruction::LocalSet(ptr2));
        out.push(Instruction::LocalSet(len1));
        out.push(Instruction::LocalSet(ptr1));

        if op == Operator::Equal || op == Operator::NotEqual {
            // Equality: compare lengths, then bytes
            let result = locals.add_local(&format!("__scmp_res_{}", locals.locals.len()), ValType::I32);
            let idx = locals.add_local(&format!("__scmp_idx_{}", locals.locals.len()), ValType::I32);

            out.push(Instruction::I32Const(1));
            out.push(Instruction::LocalSet(result));

            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::LocalGet(len2));
            out.push(Instruction::I32Ne);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(result));
            out.push(Instruction::Else);
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(idx));
            out.push(Instruction::Block(BlockType::Empty));
            out.push(Instruction::Loop(BlockType::Empty));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::I32GeU);
            out.push(Instruction::BrIf(1));
            out.push(Instruction::LocalGet(ptr1));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::LocalGet(ptr2));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::I32Ne);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(result));
            out.push(Instruction::Br(2));
            out.push(Instruction::End);
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(idx));
            out.push(Instruction::Br(0));
            out.push(Instruction::End); // loop
            out.push(Instruction::End); // block
            out.push(Instruction::End); // else

            out.push(Instruction::LocalGet(result));
            if op == Operator::NotEqual {
                out.push(Instruction::I32Eqz);
            }
        } else {
            // Lexicographic comparison: <, >, <=, >=
            // result: -1 if lhs < rhs, 0 if equal, 1 if lhs > rhs
            let cmp = locals.add_local(&format!("__scmp_cmp_{}", locals.locals.len()), ValType::I32);
            let idx = locals.add_local(&format!("__scmp_idx_{}", locals.locals.len()), ValType::I32);
            let min_len = locals.add_local(&format!("__scmp_min_{}", locals.locals.len()), ValType::I32);
            let b1 = locals.add_local(&format!("__scmp_b1_{}", locals.locals.len()), ValType::I32);
            let b2 = locals.add_local(&format!("__scmp_b2_{}", locals.locals.len()), ValType::I32);

            // min_len = min(len1, len2)
            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::LocalGet(len2));
            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::LocalGet(len2));
            out.push(Instruction::I32LeU);
            out.push(Instruction::Select);
            out.push(Instruction::LocalSet(min_len));

            // cmp = 0 (equal so far)
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(cmp));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(idx));

            out.push(Instruction::Block(BlockType::Empty));
            out.push(Instruction::Loop(BlockType::Empty));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::LocalGet(min_len));
            out.push(Instruction::I32GeU);
            out.push(Instruction::BrIf(1));

            out.push(Instruction::LocalGet(ptr1));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::LocalSet(b1));
            out.push(Instruction::LocalGet(ptr2));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::LocalSet(b2));

            out.push(Instruction::LocalGet(b1));
            out.push(Instruction::LocalGet(b2));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::I32Const(-1i32));
            out.push(Instruction::LocalSet(cmp));
            out.push(Instruction::Br(2));
            out.push(Instruction::End);

            out.push(Instruction::LocalGet(b1));
            out.push(Instruction::LocalGet(b2));
            out.push(Instruction::I32GtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::LocalSet(cmp));
            out.push(Instruction::Br(2));
            out.push(Instruction::End);

            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(idx));
            out.push(Instruction::Br(0));
            out.push(Instruction::End); // loop
            out.push(Instruction::End); // block

            // If bytes were equal, compare lengths
            out.push(Instruction::LocalGet(cmp));
            out.push(Instruction::I32Eqz);
            out.push(Instruction::If(BlockType::Empty));
            {
                out.push(Instruction::LocalGet(len1));
                out.push(Instruction::LocalGet(len2));
                out.push(Instruction::I32LtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(-1i32));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::Else);
                out.push(Instruction::LocalGet(len1));
                out.push(Instruction::LocalGet(len2));
                out.push(Instruction::I32GtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::End);
                out.push(Instruction::End);
            }
            out.push(Instruction::End);

            // Convert cmp to boolean based on operator
            match op {
                Operator::Less => {
                    out.push(Instruction::LocalGet(cmp));
                    out.push(Instruction::I32Const(-1i32));
                    out.push(Instruction::I32Eq);
                }
                Operator::Greater => {
                    out.push(Instruction::LocalGet(cmp));
                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::I32Eq);
                }
                Operator::LessEqual => {
                    out.push(Instruction::LocalGet(cmp));
                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::I32Ne);
                }
                Operator::GreaterEqual => {
                    out.push(Instruction::LocalGet(cmp));
                    out.push(Instruction::I32Const(-1i32));
                    out.push(Instruction::I32Ne);
                }
                _ => {
                    return Err(Error::InternalError(format!(
                        "unsupported operator {:?} for string comparison",
                        op
                    )));
                }
            }
        }

        Ok(())
    }

    fn emit_string_min_max(
        &mut self,
        args: &[ast::Expression],
        is_min: bool,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let best_ptr = locals.add_local(&format!("__smm_bp_{}", locals.locals.len()), ValType::I32);
        let best_len = locals.add_local(&format!("__smm_bl_{}", locals.locals.len()), ValType::I32);
        self.compile_expression(&args[0], out, locals)?;
        out.push(Instruction::LocalSet(best_len));
        out.push(Instruction::LocalSet(best_ptr));

        for arg in &args[1..] {
            let cur_ptr = locals.add_local(&format!("__smm_cp_{}", locals.locals.len()), ValType::I32);
            let cur_len = locals.add_local(&format!("__smm_cl_{}", locals.locals.len()), ValType::I32);
            self.compile_expression(arg, out, locals)?;
            out.push(Instruction::LocalSet(cur_len));
            out.push(Instruction::LocalSet(cur_ptr));

            let cmp = locals.add_local(&format!("__smm_cmp_{}", locals.locals.len()), ValType::I32);
            let idx = locals.add_local(&format!("__smm_i_{}", locals.locals.len()), ValType::I32);
            let min_len_l = locals.add_local(&format!("__smm_ml_{}", locals.locals.len()), ValType::I32);
            let b1 = locals.add_local(&format!("__smm_b1_{}", locals.locals.len()), ValType::I32);
            let b2 = locals.add_local(&format!("__smm_b2_{}", locals.locals.len()), ValType::I32);

            // min_len = min(best_len, cur_len)
            out.push(Instruction::LocalGet(best_len));
            out.push(Instruction::LocalGet(cur_len));
            out.push(Instruction::LocalGet(best_len));
            out.push(Instruction::LocalGet(cur_len));
            out.push(Instruction::I32LeU);
            out.push(Instruction::Select);
            out.push(Instruction::LocalSet(min_len_l));

            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(cmp));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(idx));

            out.push(Instruction::Block(BlockType::Empty));
            out.push(Instruction::Loop(BlockType::Empty));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::LocalGet(min_len_l));
            out.push(Instruction::I32GeU);
            out.push(Instruction::BrIf(1));

            out.push(Instruction::LocalGet(best_ptr));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::LocalSet(b1));
            out.push(Instruction::LocalGet(cur_ptr));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::LocalSet(b2));

            out.push(Instruction::LocalGet(b1));
            out.push(Instruction::LocalGet(b2));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::I32Const(-1i32));
            out.push(Instruction::LocalSet(cmp));
            out.push(Instruction::Br(2));
            out.push(Instruction::End);

            out.push(Instruction::LocalGet(b1));
            out.push(Instruction::LocalGet(b2));
            out.push(Instruction::I32GtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::LocalSet(cmp));
            out.push(Instruction::Br(2));
            out.push(Instruction::End);

            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(idx));
            out.push(Instruction::Br(0));
            out.push(Instruction::End); // loop
            out.push(Instruction::End); // block

            // If bytes equal, compare lengths
            out.push(Instruction::LocalGet(cmp));
            out.push(Instruction::I32Eqz);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::LocalGet(best_len));
            out.push(Instruction::LocalGet(cur_len));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::I32Const(-1i32));
            out.push(Instruction::LocalSet(cmp));
            out.push(Instruction::Else);
            out.push(Instruction::LocalGet(best_len));
            out.push(Instruction::LocalGet(cur_len));
            out.push(Instruction::I32GtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::LocalSet(cmp));
            out.push(Instruction::End);
            out.push(Instruction::End);
            out.push(Instruction::End);

            // For min: if cmp > 0 (best > cur), replace best with cur
            // For max: if cmp < 0 (best < cur), replace best with cur
            let should_replace = if is_min {
                // cmp > 0 means best > cur, so cur is smaller
                out.push(Instruction::LocalGet(cmp));
                out.push(Instruction::I32Const(0));
                Instruction::I32GtS
            } else {
                // cmp < 0 means best < cur, so cur is larger
                out.push(Instruction::LocalGet(cmp));
                out.push(Instruction::I32Const(0));
                Instruction::I32LtS
            };
            out.push(should_replace);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::LocalGet(cur_ptr));
            out.push(Instruction::LocalSet(best_ptr));
            out.push(Instruction::LocalGet(cur_len));
            out.push(Instruction::LocalSet(best_len));
            out.push(Instruction::End);
        }

        out.push(Instruction::LocalGet(best_ptr));
        out.push(Instruction::LocalGet(best_len));
        Ok(())
    }

    fn emit_i64_op_signed(
        &self,
        op: Operator,
        signed: bool,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        match op {
            Operator::Add => out.push(Instruction::I64Add),
            Operator::Sub => out.push(Instruction::I64Sub),
            Operator::Star => out.push(Instruction::I64Mul),
            Operator::Quo => out.push(if signed { Instruction::I64DivS } else { Instruction::I64DivU }),
            Operator::Rem => out.push(if signed { Instruction::I64RemS } else { Instruction::I64RemU }),
            Operator::And => out.push(Instruction::I64And),
            Operator::Or => out.push(Instruction::I64Or),
            Operator::Xor => out.push(Instruction::I64Xor),
            Operator::Shl => out.push(Instruction::I64Shl),
            Operator::Shr => out.push(if signed { Instruction::I64ShrS } else { Instruction::I64ShrU }),
            Operator::AndNot => {
                out.push(Instruction::I64Const(-1));
                out.push(Instruction::I64Xor);
                out.push(Instruction::I64And);
            }
            Operator::Equal => out.push(Instruction::I64Eq),
            Operator::NotEqual => out.push(Instruction::I64Ne),
            Operator::Less => out.push(if signed { Instruction::I64LtS } else { Instruction::I64LtU }),
            Operator::LessEqual => out.push(if signed { Instruction::I64LeS } else { Instruction::I64LeU }),
            Operator::Greater => out.push(if signed { Instruction::I64GtS } else { Instruction::I64GtU }),
            Operator::GreaterEqual => out.push(if signed { Instruction::I64GeS } else { Instruction::I64GeU }),
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported operator {:?} for i64 type",
                    op
                )))
            }
        }
        Ok(())
    }

    fn emit_i32_op_signed(
        &self,
        op: Operator,
        signed: bool,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        match op {
            Operator::Add => out.push(Instruction::I32Add),
            Operator::Sub => out.push(Instruction::I32Sub),
            Operator::Star => out.push(Instruction::I32Mul),
            Operator::Quo => out.push(if signed { Instruction::I32DivS } else { Instruction::I32DivU }),
            Operator::Rem => out.push(if signed { Instruction::I32RemS } else { Instruction::I32RemU }),
            Operator::And => out.push(Instruction::I32And),
            Operator::Or => out.push(Instruction::I32Or),
            Operator::Xor => out.push(Instruction::I32Xor),
            Operator::Shl => out.push(Instruction::I32Shl),
            Operator::Shr => out.push(if signed { Instruction::I32ShrS } else { Instruction::I32ShrU }),
            Operator::AndNot => {
                out.push(Instruction::I32Const(-1));
                out.push(Instruction::I32Xor);
                out.push(Instruction::I32And);
            }
            Operator::Equal => out.push(Instruction::I32Eq),
            Operator::NotEqual => out.push(Instruction::I32Ne),
            Operator::Less => out.push(if signed { Instruction::I32LtS } else { Instruction::I32LtU }),
            Operator::LessEqual => out.push(if signed { Instruction::I32LeS } else { Instruction::I32LeU }),
            Operator::Greater => out.push(if signed { Instruction::I32GtS } else { Instruction::I32GtU }),
            Operator::GreaterEqual => out.push(if signed { Instruction::I32GeS } else { Instruction::I32GeU }),
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported operator {:?} for i32 type",
                    op
                )))
            }
        }
        Ok(())
    }

    fn emit_f64_op(
        &self,
        op: Operator,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        match op {
            Operator::Add => out.push(Instruction::F64Add),
            Operator::Sub => out.push(Instruction::F64Sub),
            Operator::Star => out.push(Instruction::F64Mul),
            Operator::Quo => out.push(Instruction::F64Div),
            Operator::Equal => out.push(Instruction::F64Eq),
            Operator::NotEqual => out.push(Instruction::F64Ne),
            Operator::Less => out.push(Instruction::F64Lt),
            Operator::LessEqual => out.push(Instruction::F64Le),
            Operator::Greater => out.push(Instruction::F64Gt),
            Operator::GreaterEqual => out.push(Instruction::F64Ge),
            Operator::Rem => {
                return Err(Error::InternalError(
                    "the modulo operator (%) is not valid on floating-point types"
                        .to_string(),
                ))
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported operator {:?} for f64 type",
                    op
                )))
            }
        }
        Ok(())
    }

    fn emit_f32_op(
        &self,
        op: Operator,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        match op {
            Operator::Add => out.push(Instruction::F32Add),
            Operator::Sub => out.push(Instruction::F32Sub),
            Operator::Star => out.push(Instruction::F32Mul),
            Operator::Quo => out.push(Instruction::F32Div),
            Operator::Equal => out.push(Instruction::F32Eq),
            Operator::NotEqual => out.push(Instruction::F32Ne),
            Operator::Less => out.push(Instruction::F32Lt),
            Operator::LessEqual => out.push(Instruction::F32Le),
            Operator::Greater => out.push(Instruction::F32Gt),
            Operator::GreaterEqual => out.push(Instruction::F32Ge),
            Operator::Rem => {
                return Err(Error::InternalError(
                    "the modulo operator (%) is not valid on floating-point types"
                        .to_string(),
                ))
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported operator {:?} for f32 type",
                    op
                )))
            }
        }
        Ok(())
    }

    fn infer_slice_elem_type(type_arg: Option<&ast::Expression>) -> ValType {
        match type_arg {
            Some(ast::Expression::TypeSlice(slice_type)) => {
                Self::infer_array_elem_vt(&slice_type.typ)
            }
            _ => ValType::I64,
        }
    }

    fn compile_builtin_len(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let arg = match call.args.first() {
            Some(a) => a,
            None => {
                return Err(Error::InternalError(
                    "len() requires 1 argument".to_string(),
                ));
            }
        };

        let mut done = false;
        if let ast::Expression::Ident(ident) = arg {
            if let Some(&(_, len_local)) = locals.string_locals.get(&ident.name) {
                out.push(Instruction::LocalGet(len_local));
                done = true;
            }
            if !done && locals.get_var_struct_type(&ident.name) == Some("__slice") {
                self.compile_expression(arg, out, locals)?;
                out.push(Instruction::I32Load(MemArg {
                    offset: 4,
                    align: 2,
                    memory_index: 0,
                }));
                done = true;
            }
            if !done && locals.get_var_struct_type(&ident.name) == Some("__map") {
                self.compile_expression(arg, out, locals)?;
                out.push(Instruction::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                done = true;
            }
            if !done {
                if let Some(&(_, arr_len)) = locals.array_info.get(&ident.name) {
                    out.push(Instruction::I32Const(arr_len as i32));
                    done = true;
                }
            }
        }

        if !done {
            if let ast::Expression::Selector(sel) = arg {
                if self.is_selector_slice_field(sel, locals) {
                    self.compile_expression(arg, out, locals)?;
                    out.push(Instruction::I32Load(MemArg {
                        offset: 4,
                        align: 2,
                        memory_index: 0,
                    }));
                    done = true;
                } else if self.is_selector_map_field(sel, locals) {
                    self.compile_expression(arg, out, locals)?;
                    out.push(Instruction::I32Load(MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                    done = true;
                }
            }
        }

        if !done {
            // Non-string Slice expressions produce a header pointer; load len from header[4]
            let is_non_string_slice = matches!(arg, ast::Expression::Slice(_))
                && !self.is_string_expr(arg, locals);

            self.compile_expression(arg, out, locals)?;

            if is_non_string_slice {
                out.push(Instruction::I32Load(MemArg {
                    offset: 4,
                    align: 2,
                    memory_index: 0,
                }));
            } else {
                let result_count = self.expression_result_count(arg, Some(locals));
                if result_count >= 3 {
                    out.push(Instruction::Drop); // cap
                    let len_tmp = locals.add_local(
                        &format!("__len_tmp_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(len_tmp));
                    out.push(Instruction::Drop); // ptr
                    out.push(Instruction::LocalGet(len_tmp));
                } else if result_count == 2 {
                    let len_tmp = locals.add_local(
                        &format!("__len_tmp_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(len_tmp));
                    out.push(Instruction::Drop); // ptr
                    out.push(Instruction::LocalGet(len_tmp));
                }
            }
        }

        // Per Go spec, len() returns int (I64)
        out.push(Instruction::I64ExtendI32S);
        Ok(())
    }

    fn compile_builtin_cap(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let arg = match call.args.first() {
            Some(a) => a,
            None => {
                return Err(Error::InternalError(
                    "cap() requires 1 argument".to_string(),
                ));
            }
        };

        let mut done = false;
        if let ast::Expression::Ident(ident) = arg {
            if let Some(&(_, arr_len)) = locals.array_info.get(&ident.name) {
                out.push(Instruction::I32Const(arr_len as i32));
                done = true;
            }
            if !done && locals.get_var_struct_type(&ident.name) == Some("__slice") {
                self.compile_expression(arg, out, locals)?;
                out.push(Instruction::I32Load(MemArg {
                    offset: 8,
                    align: 2,
                    memory_index: 0,
                }));
                done = true;
            }
        }

        if !done {
            let is_non_string_slice = matches!(arg, ast::Expression::Slice(_))
                && !self.is_string_expr(arg, locals);

            self.compile_expression(arg, out, locals)?;

            if is_non_string_slice {
                out.push(Instruction::I32Load(MemArg {
                    offset: 8,
                    align: 2,
                    memory_index: 0,
                }));
            } else {
                let result_count = self.expression_result_count(arg, Some(locals));
                if result_count >= 3 {
                    let cap_tmp = locals.add_local(
                        &format!("__cap_tmp_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(cap_tmp));
                    out.push(Instruction::Drop); // len
                    out.push(Instruction::Drop); // ptr
                    out.push(Instruction::LocalGet(cap_tmp));
                } else if result_count == 2 {
                    out.push(Instruction::Drop); // len
                }
            }
        }

        // Per Go spec, cap() returns int (I64)
        out.push(Instruction::I64ExtendI32S);
        Ok(())
    }

    fn compile_builtin_copy(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        if call.args.len() < 2 {
            return Err(Error::InternalError(
                "copy() requires 2 arguments".to_string(),
            ));
        }

        let src_is_string = self.is_string_expr(&call.args[1], locals);

        if src_is_string {
            return self.compile_builtin_copy_from_string(call, out, locals);
        }

        let elem_vt = if let ast::Expression::Ident(ident) = &call.args[0] {
            locals
                .slice_elem_types
                .get(&ident.name)
                .copied()
                .unwrap_or(ValType::I64)
        } else {
            ValType::I64
        };
        let (elem_size, _) = Self::elem_size_and_align(elem_vt);

        // Compile dst slice header
        self.compile_expression(&call.args[0], out, locals)?;
        let dst_hdr = locals.add_local(
            &format!("__copy_dst_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(dst_hdr));

        // Compile src slice header
        self.compile_expression(&call.args[1], out, locals)?;
        let src_hdr = locals.add_local(
            &format!("__copy_src_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(src_hdr));

        // Load dst len
        let dst_len = locals.add_local(
            &format!("__copy_dlen_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalSet(dst_len));

        // Load src len
        let src_len = locals.add_local(
            &format!("__copy_slen_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(src_hdr));
        out.push(Instruction::I32Load(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalSet(src_len));

        // n = min(dst_len, src_len)
        let n_local = locals.add_local(
            &format!("__copy_n_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32LeU);
        out.push(Instruction::Select);
        out.push(Instruction::LocalSet(n_local));

        // memory.copy(dst_data_ptr, src_data_ptr, n * elem_size)
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalGet(src_hdr));
        out.push(Instruction::I32Load(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalGet(n_local));
        out.push(Instruction::I32Const(elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::MemoryCopy {
            dst_mem: 0,
            src_mem: 0,
        });

        // Push n as i64 (Go copy returns int)
        out.push(Instruction::LocalGet(n_local));
        out.push(Instruction::I64ExtendI32S);
        Ok(())
    }

    fn compile_builtin_copy_from_string(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        // copy(dst []byte, src string): copy bytes from string into byte slice
        // Strings are (ptr, len) on the stack; byte slices store each byte as an I32 in 4-byte slots.

        // Compile dst slice header
        self.compile_expression(&call.args[0], out, locals)?;
        let dst_hdr = locals.add_local("__cpys_dhdr", ValType::I32);
        out.push(Instruction::LocalSet(dst_hdr));

        // Compile src string (pushes ptr, len)
        self.compile_expression(&call.args[1], out, locals)?;
        let src_len = locals.add_local("__cpys_slen", ValType::I32);
        let src_ptr = locals.add_local("__cpys_sptr", ValType::I32);
        out.push(Instruction::LocalSet(src_len));
        out.push(Instruction::LocalSet(src_ptr));

        // Load dst len
        let dst_len = locals.add_local("__cpys_dlen", ValType::I32);
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_len));

        // n = min(dst_len, src_len)
        let n_local = locals.add_local("__cpys_n", ValType::I32);
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32LeU);
        out.push(Instruction::Select);
        out.push(Instruction::LocalSet(n_local));

        // Load dst data pointer
        let dst_data = locals.add_local("__cpys_ddata", ValType::I32);
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_data));

        // Loop: copy each byte from string into 4-byte I32 slots
        let idx = locals.add_local("__cpys_idx", ValType::I32);
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(idx));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));

        // if idx >= n, break
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::LocalGet(n_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        // dst_data[idx * 4] = (i32) src_ptr[idx]
        out.push(Instruction::LocalGet(dst_data));
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Const(4));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalGet(src_ptr));
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Add);
        out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

        // idx++
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(idx));
        out.push(Instruction::Br(0));

        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block

        // Push n as i64 (Go copy returns int)
        out.push(Instruction::LocalGet(n_local));
        out.push(Instruction::I64ExtendI32S);
        Ok(())
    }

    fn map_key_val_types(&self, map_type: &ast::MapType) -> (ValType, u32, ValType, u32, bool, bool, Option<String>) {
        let key_vt = self.expr_to_val_type(&map_type.key);
        let val_vt = self.expr_to_val_type(&map_type.val);
        let is_string_key = matches!(&*map_type.key, ast::Expression::Ident(id) if id.name == "string");
        let is_string_val = matches!(&*map_type.val, ast::Expression::Ident(id) if id.name == "string");
        let key_size = if is_string_key { 8u32 } else { match key_vt { ValType::I64 | ValType::F64 => 8u32, _ => 4u32 } };
        let val_size = if is_string_val { 8u32 } else { match val_vt { ValType::I64 | ValType::F64 => 8u32, _ => 4u32 } };
        let val_struct = if let ast::Expression::Ident(id) = map_type.val.as_ref() {
            if self.struct_defs.contains_key(&id.name) {
                Some(id.name.clone())
            } else {
                None
            }
        } else {
            None
        };
        (key_vt, key_size, val_vt, val_size, is_string_key, is_string_val, val_struct)
    }

    fn build_nested_map_type_info(&self, map_type: &ast::MapType) -> Option<Box<MapTypeInfo>> {
        if let ast::Expression::TypeMap(inner_map) = map_type.val.as_ref() {
            let (kv, ks, vv, vs, sk, sv, vst) = self.map_key_val_types(inner_map);
            let nested = self.build_nested_map_type_info(inner_map);
            Some(Box::new(MapTypeInfo {
                key_vt: kv, val_vt: vv, key_size: ks, val_size: vs,
                is_string_key: sk, is_string_val: sv, val_struct_type: vst,
                nested_map_val_type: nested,
            }))
        } else {
            None
        }
    }

    fn map_entry_size(key_size: u32, val_size: u32) -> u32 {
        4 + key_size + val_size // tag(4) + key + val
    }

    fn compile_builtin_make(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        // Check if this is a map make
        if let Some(ast::Expression::TypeMap(map_type)) = call.args.first() {
            return self.compile_make_map(map_type, call, out, locals);
        }

        // Resolve named composite types: make(MySlice, n) or make(MyMap, n)
        if let Some(ast::Expression::Ident(type_ident)) = call.args.first() {
            if let Some(underlying) = self.named_composite_types.get(&type_ident.name).cloned() {
                if let ast::Expression::TypeMap(map_type) = &underlying {
                    return self.compile_make_map(map_type, call, out, locals);
                }
            }
        }

        let elem_vt = Self::infer_slice_elem_type(call.args.first());
        let (elem_size, _align) = Self::elem_size_and_align(elem_vt);
        const HEADER_SIZE: i32 = 12;

        let len_local = locals.add_local(
            &format!("__make_len_{}", locals.locals.len()),
            ValType::I32,
        );

        if let Some(len_arg) = call.args.get(1) {
            self.compile_expression(len_arg, out, locals)?;
            let vt = self.infer_val_type(len_arg, locals);
            if vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }
        } else {
            out.push(Instruction::I32Const(0));
        }
        out.push(Instruction::LocalSet(len_local));

        let cap_local = locals.add_local(
            &format!("__make_cap_{}", locals.locals.len()),
            ValType::I32,
        );
        if let Some(cap_arg) = call.args.get(2) {
            self.compile_expression(cap_arg, out, locals)?;
            let vt = self.infer_val_type(cap_arg, locals);
            if vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }
        } else {
            out.push(Instruction::LocalGet(len_local));
        }
        out.push(Instruction::LocalSet(cap_local));

        // Allocate header (12 bytes)
        out.push(Instruction::I32Const(HEADER_SIZE));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let hdr_local = locals.add_local(
            &format!("__make_hdr_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(hdr_local));

        // Allocate data region (cap * elem_size bytes)
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32Const(elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let data_local = locals.add_local(
            &format!("__make_data_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(data_local));

        // Store data_ptr at header[0]
        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::I32Store(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        }));

        // Store len at header[4]
        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::LocalGet(len_local));
        out.push(Instruction::I32Store(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        }));

        // Store cap at header[8]
        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32Store(MemArg {
            offset: 8,
            align: 2,
            memory_index: 0,
        }));

        // Push header pointer as the slice value
        out.push(Instruction::LocalGet(hdr_local));
        Ok(())
    }

    fn compile_make_map(
        &mut self,
        map_type: &ast::MapType,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let (key_vt, key_size, val_vt, val_size, _is_string_key, _is_string_val, _val_struct) = self.map_key_val_types(map_type);
        let entry_size = Self::map_entry_size(key_size, val_size);

        let init_cap = if let Some(cap_arg) = call.args.get(1) {
            // Compile the capacity hint; use it as initial capacity
            self.compile_expression(cap_arg, out, locals)?;
            let vt = self.infer_val_type(cap_arg, locals);
            if vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }
            let cap_l = locals.add_local(&format!("__mmap_cap_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalSet(cap_l));
            Some(cap_l)
        } else {
            None
        };

        // Map header: 20 bytes — count(4), cap(4), data_ptr(4), key_size(4), val_size(4)
        out.push(Instruction::I32Const(20));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let hdr = locals.add_local(&format!("__mmap_hdr_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalSet(hdr));

        let actual_cap = if let Some(cl) = init_cap {
            cl
        } else {
            let default_cap = locals.add_local(&format!("__mmap_dcap_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::I32Const(8)); // default capacity 8
            out.push(Instruction::LocalSet(default_cap));
            default_cap
        };

        // Allocate data: cap * entry_size
        out.push(Instruction::LocalGet(actual_cap));
        out.push(Instruction::I32Const(entry_size as i32));
        out.push(Instruction::I32Mul);
        let data_sz = locals.add_local(&format!("__mmap_dsz_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalTee(data_sz));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let data_ptr = locals.add_local(&format!("__mmap_dp_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalTee(data_ptr));

        // Zero-fill data
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalGet(data_sz));
        out.push(Instruction::MemoryFill(0));

        // Write header fields
        // count = 0
        out.push(Instruction::LocalGet(hdr));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
        // capacity
        out.push(Instruction::LocalGet(hdr));
        out.push(Instruction::LocalGet(actual_cap));
        out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
        // data_ptr
        out.push(Instruction::LocalGet(hdr));
        out.push(Instruction::LocalGet(data_ptr));
        out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));
        // key_size
        out.push(Instruction::LocalGet(hdr));
        out.push(Instruction::I32Const(key_size as i32));
        out.push(Instruction::I32Store(MemArg { offset: 12, align: 2, memory_index: 0 }));
        // val_size
        out.push(Instruction::LocalGet(hdr));
        out.push(Instruction::I32Const(val_size as i32));
        out.push(Instruction::I32Store(MemArg { offset: 16, align: 2, memory_index: 0 }));

        out.push(Instruction::LocalGet(hdr));

        // Stash info so that index/assign can find key/val types
        let _ = (key_vt, val_vt);
        Ok(())
    }

    fn compile_builtin_append(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        if call.args.len() < 2 {
            return Err(Error::InternalError(
                "append() requires at least 2 arguments".to_string(),
            ));
        }

        let elem_vt_from_slice = if let ast::Expression::Ident(ident) = &call.args[0] {
            locals
                .slice_elem_types
                .get(&ident.name)
                .copied()
                .unwrap_or(ValType::I64)
        } else {
            ValType::I64
        };
        let (elem_size, _align) = Self::elem_size_and_align(elem_vt_from_slice);
        let elem_vt = elem_vt_from_slice;

        // Handle append(s1, s2...) — spread a source slice into the destination
        if call.dots.is_some() && call.args.len() == 2 {
            if self.is_string_expr(&call.args[1], locals) {
                return self.compile_builtin_append_spread_string(call, out, locals);
            }
            return self.compile_builtin_append_spread(call, out, locals, elem_vt, elem_size);
        }

        let num_new_elems = (call.args.len() - 1) as i32;

        // Compile and store each element to append
        let mut elem_locals = Vec::with_capacity(num_new_elems as usize);
        for i in 1..call.args.len() {
            self.compile_expression(&call.args[i], out, locals)?;
            let expr_vt = self.infer_val_type(&call.args[i], locals);
            if expr_vt != elem_vt_from_slice {
                match (expr_vt, elem_vt_from_slice) {
                    (ValType::I64, ValType::I32) => out.push(Instruction::I32WrapI64),
                    (ValType::I32, ValType::I64) => out.push(Instruction::I64ExtendI32S),
                    (ValType::F64, ValType::F32) => out.push(Instruction::F32DemoteF64),
                    (ValType::F32, ValType::F64) => out.push(Instruction::F64PromoteF32),
                    _ => {
                        return Err(Error::InternalError(format!(
                            "type mismatch in append: element type {:?} cannot be coerced to slice element type {:?}",
                            expr_vt, elem_vt_from_slice
                        )));
                    }
                }
            }
            let el = locals.add_local(
                &format!("__app_elem_{}_{}", i, locals.locals.len()),
                elem_vt,
            );
            out.push(Instruction::LocalSet(el));
            elem_locals.push(el);
        }

        // Compile slice argument (header pointer)
        self.compile_expression(&call.args[0], out, locals)?;
        let hdr_local = locals.add_local(
            &format!("__app_hdr_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(hdr_local));

        // Load current len
        let old_len = locals.add_local(
            &format!("__app_len_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::I32Load(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalSet(old_len));

        // new_len = old_len + num_new_elems
        let new_len = locals.add_local(
            &format!("__app_nlen_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(old_len));
        out.push(Instruction::I32Const(num_new_elems));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(new_len));

        // Load current cap
        let cap_local = locals.add_local(
            &format!("__app_cap_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::I32Load(MemArg {
            offset: 8,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalSet(cap_local));

        // If new_len > cap, grow
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GtU);
        out.push(Instruction::If(BlockType::Empty));
        {
            // new_cap = max((cap + 1) * 2, new_len)
            let new_cap = locals.add_local(
                &format!("__app_ncap_{}", locals.locals.len()),
                ValType::I32,
            );
            // doubled = (cap + 1) * 2 with overflow guard
            out.push(Instruction::LocalGet(cap_local));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            let cap_plus1 = locals.add_local(
                &format!("__app_cp1_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalTee(cap_plus1));
            // overflow if cap_plus1 < cap (wrapped)
            out.push(Instruction::LocalGet(cap_local));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            out.push(Instruction::LocalGet(cap_plus1));
            out.push(Instruction::I32Const(2));
            out.push(Instruction::I32Mul);
            let doubled = locals.add_local(
                &format!("__app_dbl_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalTee(doubled));
            // overflow if doubled < cap_plus1 (wrapped around on multiply)
            out.push(Instruction::LocalGet(cap_plus1));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            // new_cap = select(doubled, new_len, doubled >= new_len)
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::I32GeU);
            out.push(Instruction::Select);
            out.push(Instruction::LocalSet(new_cap));

            // Allocate new data: new_cap * elem_size (with overflow check)
            let new_data = locals.add_local(
                &format!("__app_ndata_{}", locals.locals.len()),
                ValType::I32,
            );
            let alloc_bytes = locals.add_local(
                &format!("__app_abytes_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::LocalTee(alloc_bytes));
            // overflow if alloc_bytes / elem_size != new_cap (and elem_size > 1)
            if elem_size > 1 {
                out.push(Instruction::I32Const(elem_size));
                out.push(Instruction::I32DivU);
                out.push(Instruction::LocalGet(new_cap));
                out.push(Instruction::I32Ne);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::Call(self.oom_func_idx));
                out.push(Instruction::Unreachable);
                out.push(Instruction::End);
            }
            out.push(Instruction::LocalGet(alloc_bytes));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(new_data));

            // Copy old data: memory.copy(new_data, old_data_ptr, old_len * elem_size)
            out.push(Instruction::LocalGet(new_data));
            out.push(Instruction::LocalGet(hdr_local));
            out.push(Instruction::I32Load(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
            out.push(Instruction::LocalGet(old_len));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::MemoryCopy {
                dst_mem: 0,
                src_mem: 0,
            });

            // Update header: data_ptr = new_data
            out.push(Instruction::LocalGet(hdr_local));
            out.push(Instruction::LocalGet(new_data));
            out.push(Instruction::I32Store(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));

            // Update header: cap = new_cap
            out.push(Instruction::LocalGet(hdr_local));
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Store(MemArg {
                offset: 8,
                align: 2,
                memory_index: 0,
            }));
        }
        out.push(Instruction::End);

        // Load data_ptr from header
        let data_ptr = locals.add_local(
            &format!("__app_dptr_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::I32Load(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalSet(data_ptr));

        // Store each element at data_ptr + (old_len + i) * elem_size
        for (i, &el) in elem_locals.iter().enumerate() {
            out.push(Instruction::LocalGet(data_ptr));
            out.push(Instruction::LocalGet(old_len));
            out.push(Instruction::I32Const(i as i32));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(el));
            match elem_vt {
                ValType::I64 => out.push(Instruction::I64Store(MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                })),
                ValType::F64 => out.push(Instruction::F64Store(MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                })),
                ValType::I32 => out.push(Instruction::I32Store(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                })),
                ValType::F32 => out.push(Instruction::F32Store(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                })),
                _ => out.push(Instruction::I64Store(MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                })),
            }
        }

        // Update header: len = new_len
        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::I32Store(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        }));

        // Push header pointer as result
        out.push(Instruction::LocalGet(hdr_local));
        Ok(())
    }

    /// Handle `append(dst, src...)` where src is a slice spread into dst.
    fn compile_builtin_append_spread(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        _elem_vt: ValType,
        elem_size: i32,
    ) -> Result<(), Error> {
        // Compile source slice (second arg) — get its header pointer
        self.compile_expression(&call.args[1], out, locals)?;
        let src_hdr = locals.add_local("__appsprd_shdr", ValType::I32);
        out.push(Instruction::LocalSet(src_hdr));

        // Load source len and data pointer
        let src_len = locals.add_local("__appsprd_slen", ValType::I32);
        let src_data = locals.add_local("__appsprd_sdata", ValType::I32);
        out.push(Instruction::LocalGet(src_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(src_len));
        out.push(Instruction::LocalGet(src_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(src_data));

        // Compile destination slice (first arg)
        self.compile_expression(&call.args[0], out, locals)?;
        let dst_hdr = locals.add_local("__appsprd_dhdr", ValType::I32);
        out.push(Instruction::LocalSet(dst_hdr));

        // Load dst len and cap
        let dst_len = locals.add_local("__appsprd_dlen", ValType::I32);
        let dst_cap = locals.add_local("__appsprd_dcap", ValType::I32);
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_len));
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_cap));

        // new_len = dst_len + src_len
        let new_len = locals.add_local("__appsprd_nlen", ValType::I32);
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(new_len));

        // If new_len > cap, grow
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::LocalGet(dst_cap));
        out.push(Instruction::I32GtU);
        out.push(Instruction::If(BlockType::Empty));
        {
            let new_cap = locals.add_local("__appsprd_ncap", ValType::I32);
            // doubled = (cap + 1) * 2
            out.push(Instruction::LocalGet(dst_cap));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            let cap_plus1 = locals.add_local("__appsprd_cp1", ValType::I32);
            out.push(Instruction::LocalTee(cap_plus1));
            out.push(Instruction::LocalGet(dst_cap));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            out.push(Instruction::LocalGet(cap_plus1));
            out.push(Instruction::I32Const(2));
            out.push(Instruction::I32Mul);
            let doubled = locals.add_local("__appsprd_dbl", ValType::I32);
            out.push(Instruction::LocalTee(doubled));
            out.push(Instruction::LocalGet(cap_plus1));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            // new_cap = max(doubled, new_len)
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::I32GeU);
            out.push(Instruction::Select);
            out.push(Instruction::LocalSet(new_cap));

            // Allocate new_cap * elem_size
            let alloc_bytes = locals.add_local("__appsprd_abytes", ValType::I32);
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::LocalTee(alloc_bytes));
            if elem_size > 1 {
                out.push(Instruction::I32Const(elem_size));
                out.push(Instruction::I32DivU);
                out.push(Instruction::LocalGet(new_cap));
                out.push(Instruction::I32Ne);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::Call(self.oom_func_idx));
                out.push(Instruction::Unreachable);
                out.push(Instruction::End);
            }
            let new_data = locals.add_local("__appsprd_ndata", ValType::I32);
            out.push(Instruction::LocalGet(alloc_bytes));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(new_data));

            // Copy old data
            out.push(Instruction::LocalGet(new_data));
            out.push(Instruction::LocalGet(dst_hdr));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalGet(dst_len));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

            // Update header
            out.push(Instruction::LocalGet(dst_hdr));
            out.push(Instruction::LocalGet(new_data));
            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalGet(dst_hdr));
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));
        }
        out.push(Instruction::End);

        // Bulk copy: memory.copy(dst_data + dst_len*elem_size, src_data, src_len*elem_size)
        let dst_data = locals.add_local("__appsprd_ddptr", ValType::I32);
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_data));

        out.push(Instruction::LocalGet(dst_data));
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::I32Const(elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalGet(src_data));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32Const(elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

        // Update header: len = new_len
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

        out.push(Instruction::LocalGet(dst_hdr));
        Ok(())
    }

    /// Handle `append(dst []byte, src string...)`: append bytes from string into byte slice.
    fn compile_builtin_append_spread_string(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let elem_size: i32 = 4; // []byte stores each byte in a 4-byte I32 slot

        // Compile source string (pushes ptr, len)
        self.compile_expression(&call.args[1], out, locals)?;
        let src_len = locals.add_local("__appstr_slen", ValType::I32);
        let src_ptr = locals.add_local("__appstr_sptr", ValType::I32);
        out.push(Instruction::LocalSet(src_len));
        out.push(Instruction::LocalSet(src_ptr));

        // Compile destination slice (first arg)
        self.compile_expression(&call.args[0], out, locals)?;
        let dst_hdr = locals.add_local("__appstr_dhdr", ValType::I32);
        out.push(Instruction::LocalSet(dst_hdr));

        // Load dst len and cap
        let dst_len = locals.add_local("__appstr_dlen", ValType::I32);
        let dst_cap = locals.add_local("__appstr_dcap", ValType::I32);
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_len));
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_cap));

        // new_len = dst_len + src_len
        let new_len = locals.add_local("__appstr_nlen", ValType::I32);
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(new_len));

        // If new_len > cap, grow (same growth logic as append_spread)
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::LocalGet(dst_cap));
        out.push(Instruction::I32GtU);
        out.push(Instruction::If(BlockType::Empty));
        {
            let new_cap = locals.add_local("__appstr_ncap", ValType::I32);
            out.push(Instruction::LocalGet(dst_cap));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            let cap_plus1 = locals.add_local("__appstr_cp1", ValType::I32);
            out.push(Instruction::LocalTee(cap_plus1));
            out.push(Instruction::LocalGet(dst_cap));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            out.push(Instruction::LocalGet(cap_plus1));
            out.push(Instruction::I32Const(2));
            out.push(Instruction::I32Mul);
            let doubled = locals.add_local("__appstr_dbl", ValType::I32);
            out.push(Instruction::LocalTee(doubled));
            out.push(Instruction::LocalGet(cap_plus1));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::I32GeU);
            out.push(Instruction::Select);
            out.push(Instruction::LocalSet(new_cap));

            let alloc_bytes = locals.add_local("__appstr_abytes", ValType::I32);
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::LocalTee(alloc_bytes));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32DivU);
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Ne);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            let new_data = locals.add_local("__appstr_ndata", ValType::I32);
            out.push(Instruction::LocalGet(alloc_bytes));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(new_data));

            // Copy old data
            out.push(Instruction::LocalGet(new_data));
            out.push(Instruction::LocalGet(dst_hdr));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalGet(dst_len));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

            // Update header
            out.push(Instruction::LocalGet(dst_hdr));
            out.push(Instruction::LocalGet(new_data));
            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalGet(dst_hdr));
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));
        }
        out.push(Instruction::End);

        // Copy bytes from string into I32 slots: loop over each byte
        let dst_data = locals.add_local("__appstr_ddptr", ValType::I32);
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_data));

        let idx = locals.add_local("__appstr_idx", ValType::I32);
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(idx));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));

        // if idx >= src_len, break
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        // dst_data[(dst_len + idx) * 4] = (i32) src_ptr[idx]
        out.push(Instruction::LocalGet(dst_data));
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Add);
        out.push(Instruction::I32Const(elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalGet(src_ptr));
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Add);
        out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

        // idx++
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(idx));
        out.push(Instruction::Br(0));

        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block

        // Update header: len = new_len
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

        out.push(Instruction::LocalGet(dst_hdr));
        Ok(())
    }

    fn compile_call(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        match call.func.as_ref() {
            ast::Expression::Ident(ident) => {
                match ident.name.as_str() {
                    "len" => {
                        return self.compile_builtin_len(call, out, locals);
                    }
                    "make" => {
                        return self.compile_builtin_make(call, out, locals);
                    }
                    "append" => {
                        return self.compile_builtin_append(call, out, locals);
                    }
                    "cap" => {
                        return self.compile_builtin_cap(call, out, locals);
                    }
                    "copy" => {
                        return self.compile_builtin_copy(call, out, locals);
                    }
                    "panic" => {
                        if let Some(arg) = call.args.first() {
                            let str_ptr_local = locals.add_local(
                                &format!("__panic_ptr_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            let str_len_local = locals.add_local(
                                &format!("__panic_len_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            if self.is_string_expr(arg, locals) {
                                self.compile_expression(arg, out, locals)?;
                            } else {
                                let vt = self.infer_val_type(arg, locals);
                                self.compile_expression(arg, out, locals)?;
                                match vt {
                                    ValType::I64 | ValType::I32 => {
                                        if vt == ValType::I32 {
                                            out.push(Instruction::I64ExtendI32S);
                                        }
                                        self.emit_i64_to_string(out, locals)?;
                                    }
                                    ValType::F64 => {
                                        self.emit_f64_to_string(out, locals)?;
                                    }
                                    ValType::F32 => {
                                        out.push(Instruction::F64PromoteF32);
                                        self.emit_f64_to_string(out, locals)?;
                                    }
                                    _ => {
                                        out.push(Instruction::Drop);
                                        out.push(Instruction::I32Const(0));
                                        out.push(Instruction::I32Const(0));
                                    }
                                }
                            }
                            out.push(Instruction::LocalSet(str_len_local));
                            out.push(Instruction::LocalSet(str_ptr_local));

                            out.push(Instruction::LocalGet(str_ptr_local));
                            out.push(Instruction::GlobalSet(self.panic_value_ptr_global));
                            out.push(Instruction::LocalGet(str_len_local));
                            out.push(Instruction::GlobalSet(self.panic_value_len_global));

                            out.push(Instruction::LocalGet(str_ptr_local));
                            out.push(Instruction::LocalGet(str_len_local));
                            out.push(Instruction::Call(0)); // ctx_log
                        }

                        // Set panicking flag
                        out.push(Instruction::I32Const(1));
                        out.push(Instruction::GlobalSet(self.panicking_global));

                        // Run deferred calls so recover() can clear the flag
                        self.emit_deferred_calls(out);

                        // If still panicking (no recover), trap
                        out.push(Instruction::GlobalGet(self.panicking_global));
                        out.push(Instruction::If(BlockType::Empty));
                        out.push(Instruction::Unreachable);
                        out.push(Instruction::End);

                        // Recovered: push zero return values and return
                        for rt in &self.current_result_types.clone() {
                            match rt {
                                ValType::I32 => out.push(Instruction::I32Const(0)),
                                ValType::I64 => out.push(Instruction::I64Const(0)),
                                ValType::F32 => out.push(Instruction::F32Const(0.0)),
                                ValType::F64 => out.push(Instruction::F64Const(0.0)),
                                _ => out.push(Instruction::I32Const(0)),
                            }
                        }
                        out.push(Instruction::Return);
                        return Ok(());
                    }
                    "recover" => {
                        let rec_ptr = locals.add_local(
                            &format!("__recover_ptr_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        let rec_len = locals.add_local(
                            &format!("__recover_len_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::LocalSet(rec_ptr));
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::LocalSet(rec_len));
                        out.push(Instruction::GlobalGet(self.panicking_global));
                        out.push(Instruction::If(BlockType::Empty));
                        out.push(Instruction::GlobalGet(self.panic_value_ptr_global));
                        out.push(Instruction::LocalSet(rec_ptr));
                        out.push(Instruction::GlobalGet(self.panic_value_len_global));
                        out.push(Instruction::LocalSet(rec_len));
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::GlobalSet(self.panicking_global));
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::GlobalSet(self.panic_value_ptr_global));
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::GlobalSet(self.panic_value_len_global));
                        out.push(Instruction::End);
                        out.push(Instruction::LocalGet(rec_ptr));
                        out.push(Instruction::LocalGet(rec_len));
                        return Ok(());
                    }
                    "int" | "int64" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            let is_unsigned = self.is_unsigned_expr(arg, locals);
                            match vt {
                                ValType::F64 => out.push(Instruction::I64TruncF64S),
                                ValType::F32 => out.push(Instruction::I64TruncF32S),
                                ValType::I32 => out.push(if is_unsigned { Instruction::I64ExtendI32U } else { Instruction::I64ExtendI32S }),
                                ValType::I64 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for int conversion",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(());
                    }
                    "float64" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            let is_unsigned = self.is_unsigned_expr(arg, locals);
                            match vt {
                                ValType::I64 => out.push(if is_unsigned { Instruction::F64ConvertI64U } else { Instruction::F64ConvertI64S }),
                                ValType::I32 => out.push(if is_unsigned { Instruction::F64ConvertI32U } else { Instruction::F64ConvertI32S }),
                                ValType::F32 => out.push(Instruction::F64PromoteF32),
                                ValType::F64 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for float64 conversion",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(());
                    }
                    "float32" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            let is_unsigned = self.is_unsigned_expr(arg, locals);
                            match vt {
                                ValType::I64 => out.push(if is_unsigned { Instruction::F32ConvertI64U } else { Instruction::F32ConvertI64S }),
                                ValType::I32 => out.push(if is_unsigned { Instruction::F32ConvertI32U } else { Instruction::F32ConvertI32S }),
                                ValType::F64 => out.push(Instruction::F32DemoteF64),
                                ValType::F32 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for float32 conversion",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(());
                    }
                    "int32" | "rune" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::F64 => out.push(Instruction::I32TruncF64S),
                                ValType::F32 => out.push(Instruction::I32TruncF32S),
                                ValType::I32 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for int32 conversion",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(());
                    }
                    "int8" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::F64 => out.push(Instruction::I32TruncF64S),
                                ValType::F32 => out.push(Instruction::I32TruncF32S),
                                ValType::I32 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for int8 conversion",
                                        vt
                                    )));
                                }
                            }
                            // Sign-extend from 8 bits
                            out.push(Instruction::I32Const(24));
                            out.push(Instruction::I32Shl);
                            out.push(Instruction::I32Const(24));
                            out.push(Instruction::I32ShrS);
                        }
                        return Ok(());
                    }
                    "int16" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::F64 => out.push(Instruction::I32TruncF64S),
                                ValType::F32 => out.push(Instruction::I32TruncF32S),
                                ValType::I32 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for int16 conversion",
                                        vt
                                    )));
                                }
                            }
                            // Sign-extend from 16 bits
                            out.push(Instruction::I32Const(16));
                            out.push(Instruction::I32Shl);
                            out.push(Instruction::I32Const(16));
                            out.push(Instruction::I32ShrS);
                        }
                        return Ok(());
                    }
                    "byte" | "uint8" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::I32 => {}
                                ValType::F64 => out.push(Instruction::I32TruncF64U),
                                ValType::F32 => out.push(Instruction::I32TruncF32U),
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for byte/uint8 conversion",
                                        vt
                                    )));
                                }
                            }
                            out.push(Instruction::I32Const(0xFF));
                            out.push(Instruction::I32And);
                        }
                        return Ok(());
                    }
                    "uint16" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::I32 => {}
                                ValType::F64 => out.push(Instruction::I32TruncF64U),
                                ValType::F32 => out.push(Instruction::I32TruncF32U),
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for uint16 conversion",
                                        vt
                                    )));
                                }
                            }
                            out.push(Instruction::I32Const(0xFFFF));
                            out.push(Instruction::I32And);
                        }
                        return Ok(());
                    }
                    "uint32" | "uintptr" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::I32 => {}
                                ValType::F64 => out.push(Instruction::I32TruncF64U),
                                ValType::F32 => out.push(Instruction::I32TruncF32U),
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for uint32 conversion",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(());
                    }
                    "uint" | "uint64" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::F64 => out.push(Instruction::I64TruncF64U),
                                ValType::F32 => out.push(Instruction::I64TruncF32U),
                                ValType::I32 => out.push(Instruction::I64ExtendI32U),
                                ValType::I64 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for uint/uint64 conversion",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(());
                    }
                    "bool" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                        }
                        return Ok(());
                    }
                    "string" => {
                        if let Some(arg) = call.args.first() {
                            let vt = self.infer_val_type(arg, locals);
                            if vt == ValType::I32
                                && self.is_string_expr(arg, locals)
                            {
                                self.compile_expression(arg, out, locals)?;
                                return Ok(());
                            }
                            // Check if arg is a []rune slice => string([]rune)
                            if let ast::Expression::Ident(arg_ident) = arg {
                                if locals.rune_slices.contains(&arg_ident.name)
                                    && locals.get_var_struct_type(&arg_ident.name) == Some("__slice")
                                {
                                    let hdr_idx = locals.find(&arg_ident.name).ok_or_else(|| {
                                        Error::InternalError(format!(
                                            "local variable '{}' not found for []rune to string conversion", arg_ident.name
                                        ))
                                    })?;
                                    let src_ptr = locals.add_local(&format!("__sr2s_sp_{}", locals.locals.len()), ValType::I32);
                                    let s_len = locals.add_local(&format!("__sr2s_ln_{}", locals.locals.len()), ValType::I32);
                                    let dst_ptr = locals.add_local(&format!("__sr2s_dp_{}", locals.locals.len()), ValType::I32);
                                    let loop_i = locals.add_local(&format!("__sr2s_i_{}", locals.locals.len()), ValType::I32);
                                    let cur_rune = locals.add_local(&format!("__sr2s_r_{}", locals.locals.len()), ValType::I32);
                                    let write_pos = locals.add_local(&format!("__sr2s_wp_{}", locals.locals.len()), ValType::I32);

                                    // Read slice header
                                    out.push(Instruction::LocalGet(hdr_idx));
                                    out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalSet(src_ptr));
                                    out.push(Instruction::LocalGet(hdr_idx));
                                    out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalSet(s_len));

                                    // Allocate max possible bytes: len * 4 (worst case UTF-8)
                                    out.push(Instruction::LocalGet(s_len));
                                    out.push(Instruction::I32Const(4));
                                    out.push(Instruction::I32Mul);
                                    out.push(Instruction::Call(self.alloc_func_idx()?));
                                    out.push(Instruction::LocalSet(dst_ptr));

                                    out.push(Instruction::I32Const(0));
                                    out.push(Instruction::LocalSet(loop_i));
                                    out.push(Instruction::I32Const(0));
                                    out.push(Instruction::LocalSet(write_pos));

                                    out.push(Instruction::Block(BlockType::Empty));
                                    out.push(Instruction::Loop(BlockType::Empty));
                                    // if loop_i >= s_len, break
                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::LocalGet(s_len));
                                    out.push(Instruction::I32GeU);
                                    out.push(Instruction::BrIf(1));

                                    // cur_rune = src[loop_i * 4]
                                    out.push(Instruction::LocalGet(src_ptr));
                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::I32Const(4));
                                    out.push(Instruction::I32Mul);
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalSet(cur_rune));

                                    // UTF-8 encode cur_rune into dst_ptr + write_pos
                                    // 1-byte: rune < 0x80
                                    out.push(Instruction::LocalGet(cur_rune));
                                    out.push(Instruction::I32Const(0x80));
                                    out.push(Instruction::I32LtU);
                                    out.push(Instruction::If(BlockType::Empty));
                                    {
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Const(1));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalSet(write_pos));
                                    }
                                    out.push(Instruction::Else);
                                    // 2-byte: rune < 0x800
                                    out.push(Instruction::LocalGet(cur_rune));
                                    out.push(Instruction::I32Const(0x800));
                                    out.push(Instruction::I32LtU);
                                    out.push(Instruction::If(BlockType::Empty));
                                    {
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0xC0));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(6));
                                        out.push(Instruction::I32ShrU);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0x80));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Const(2));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalSet(write_pos));
                                    }
                                    out.push(Instruction::Else);
                                    // 3-byte: rune < 0x10000
                                    out.push(Instruction::LocalGet(cur_rune));
                                    out.push(Instruction::I32Const(0x10000));
                                    out.push(Instruction::I32LtU);
                                    out.push(Instruction::If(BlockType::Empty));
                                    {
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0xE0));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(12));
                                        out.push(Instruction::I32ShrU);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0x80));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(6));
                                        out.push(Instruction::I32ShrU);
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0x80));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 2, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Const(3));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalSet(write_pos));
                                    }
                                    out.push(Instruction::Else);
                                    {
                                        // 4-byte
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0xF0));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(18));
                                        out.push(Instruction::I32ShrU);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0x80));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(12));
                                        out.push(Instruction::I32ShrU);
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0x80));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(6));
                                        out.push(Instruction::I32ShrU);
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 2, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0x80));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 3, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Const(4));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalSet(write_pos));
                                    }
                                    out.push(Instruction::End); // 3-byte vs 4-byte
                                    out.push(Instruction::End); // 2-byte vs rest
                                    out.push(Instruction::End); // 1-byte vs rest

                                    // loop_i++
                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::I32Const(1));
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::LocalSet(loop_i));
                                    out.push(Instruction::Br(0));
                                    out.push(Instruction::End); // loop
                                    out.push(Instruction::End); // block

                                    out.push(Instruction::LocalGet(dst_ptr));
                                    out.push(Instruction::LocalGet(write_pos));
                                    return Ok(());
                                }
                            }
                            // Check if arg is a []byte slice => string([]byte)
                            if let ast::Expression::Ident(arg_ident) = arg {
                                if locals.slice_elem_types.get(&arg_ident.name) == Some(&ValType::I32)
                                    && locals.get_var_struct_type(&arg_ident.name) == Some("__slice")
                                {
                                    // Pack I32 slice elements back into compact bytes
                                    let hdr_idx = locals.find(&arg_ident.name).ok_or_else(|| {
                                        Error::InternalError(format!(
                                            "local variable '{}' not found for []byte to string conversion", arg_ident.name
                                        ))
                                    })?;
                                    let src_ptr = locals.add_local(&format!("__s2b_sp_{}", locals.locals.len()), ValType::I32);
                                    let s_len = locals.add_local(&format!("__s2b_ln_{}", locals.locals.len()), ValType::I32);
                                    let dst_ptr = locals.add_local(&format!("__s2b_dp_{}", locals.locals.len()), ValType::I32);
                                    let loop_i = locals.add_local(&format!("__s2b_i_{}", locals.locals.len()), ValType::I32);

                                    out.push(Instruction::LocalGet(hdr_idx));
                                    out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalSet(src_ptr));
                                    out.push(Instruction::LocalGet(hdr_idx));
                                    out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalSet(s_len));

                                    out.push(Instruction::LocalGet(s_len));
                                    out.push(Instruction::Call(self.alloc_func_idx()?));
                                    out.push(Instruction::LocalSet(dst_ptr));

                                    out.push(Instruction::I32Const(0));
                                    out.push(Instruction::LocalSet(loop_i));

                                    out.push(Instruction::Block(BlockType::Empty));
                                    out.push(Instruction::Loop(BlockType::Empty));
                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::LocalGet(s_len));
                                    out.push(Instruction::I32GeU);
                                    out.push(Instruction::BrIf(1));

                                    // dst[i] = (byte) src[i*4]
                                    out.push(Instruction::LocalGet(dst_ptr));
                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::LocalGet(src_ptr));
                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::I32Const(4));
                                    out.push(Instruction::I32Mul);
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                    out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::I32Const(1));
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::LocalSet(loop_i));
                                    out.push(Instruction::Br(0));
                                    out.push(Instruction::End);
                                    out.push(Instruction::End);

                                    out.push(Instruction::LocalGet(dst_ptr));
                                    out.push(Instruction::LocalGet(s_len));
                                    return Ok(());
                                }
                            }
                            self.compile_expression(arg, out, locals)?;
                            let rune_local = locals.add_local(
                                &format!("__rune_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::I32 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "string() conversion from {:?} is not supported",
                                        vt
                                    )));
                                }
                            }
                            out.push(Instruction::LocalSet(rune_local));

                            let buf_local = locals.add_local(
                                &format!("__rune_buf_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            let len_local = locals.add_local(
                                &format!("__rune_len_{}", locals.locals.len()),
                                ValType::I32,
                            );

                            out.push(Instruction::I32Const(4));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            out.push(Instruction::LocalSet(buf_local));

                            // Replace invalid runes with U+FFFD:
                            // surrogates (0xD800..0xDFFF) or values >= 0x110000
                            {
                                let is_invalid = locals.add_local(
                                    &format!("__rune_inv_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                // Check >= 0x110000
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(0x110000));
                                out.push(Instruction::I32GeU);
                                out.push(Instruction::LocalSet(is_invalid));
                                // Check surrogate range: rune >= 0xD800 && rune < 0xE000
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(0xD800u32 as i32));
                                out.push(Instruction::I32GeU);
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(0xE000u32 as i32));
                                out.push(Instruction::I32LtU);
                                out.push(Instruction::I32And);
                                out.push(Instruction::LocalGet(is_invalid));
                                out.push(Instruction::I32Or);
                                out.push(Instruction::If(BlockType::Empty));
                                out.push(Instruction::I32Const(0xFFFD));
                                out.push(Instruction::LocalSet(rune_local));
                                out.push(Instruction::End);
                            }

                            // UTF-8 encode: 1-byte (0..0x80), 2-byte (0x80..0x800),
                            // 3-byte (0x800..0x10000), 4-byte (0x10000..0x110000)
                            out.push(Instruction::LocalGet(rune_local));
                            out.push(Instruction::I32Const(0x80));
                            out.push(Instruction::I32LtU);
                            out.push(Instruction::If(BlockType::Empty));
                            {
                                // 1-byte: buf[0] = rune
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                out.push(Instruction::I32Const(1));
                                out.push(Instruction::LocalSet(len_local));
                            }
                            out.push(Instruction::Else);
                            out.push(Instruction::LocalGet(rune_local));
                            out.push(Instruction::I32Const(0x800));
                            out.push(Instruction::I32LtU);
                            out.push(Instruction::If(BlockType::Empty));
                            {
                                // 2-byte: buf[0] = 0xC0 | (rune >> 6), buf[1] = 0x80 | (rune & 0x3F)
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0xC0));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(6));
                                out.push(Instruction::I32ShrU);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0x80));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                out.push(Instruction::I32Const(2));
                                out.push(Instruction::LocalSet(len_local));
                            }
                            out.push(Instruction::Else);
                            out.push(Instruction::LocalGet(rune_local));
                            out.push(Instruction::I32Const(0x10000));
                            out.push(Instruction::I32LtU);
                            out.push(Instruction::If(BlockType::Empty));
                            {
                                // 3-byte
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0xE0));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(12));
                                out.push(Instruction::I32ShrU);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0x80));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(6));
                                out.push(Instruction::I32ShrU);
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0x80));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 2, align: 0, memory_index: 0 }));
                                out.push(Instruction::I32Const(3));
                                out.push(Instruction::LocalSet(len_local));
                            }
                            out.push(Instruction::Else);
                            {
                                // 4-byte
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0xF0));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(18));
                                out.push(Instruction::I32ShrU);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0x80));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(12));
                                out.push(Instruction::I32ShrU);
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0x80));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(6));
                                out.push(Instruction::I32ShrU);
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 2, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0x80));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 3, align: 0, memory_index: 0 }));
                                out.push(Instruction::I32Const(4));
                                out.push(Instruction::LocalSet(len_local));
                            }
                            out.push(Instruction::End); // closes if/else (3-byte vs 4-byte)
                            out.push(Instruction::End); // closes if/else (2-byte vs rest)
                            out.push(Instruction::End); // closes if/else (1-byte vs rest)

                            out.push(Instruction::LocalGet(buf_local));
                            out.push(Instruction::LocalGet(len_local));
                            return Ok(());
                        }
                        return Ok(());
                    }
                    "new" => {
                        if let Some(type_arg) = call.args.first() {
                            let alloc_size = if let ast::Expression::Ident(ti) = type_arg {
                                if let Some(sdef) = self.struct_defs.get(&ti.name) {
                                    sdef.total_size
                                } else {
                                    match ti.name.as_str() {
                                        "int" | "int64" | "uint" | "uint64" | "float64" => 8,
                                        "int32" | "uint32" | "float32" | "rune" => 4,
                                        "int16" | "uint16" => 2,
                                        "int8" | "uint8" | "byte" | "bool" => 1,
                                        _ => 8,
                                    }
                                }
                            } else {
                                8u32
                            };
                            let alloc_aligned = alloc_size.max(8);
                            out.push(Instruction::I32Const(alloc_aligned as i32));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            let ptr_local = locals.add_local(
                                &format!("__new_ptr_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            out.push(Instruction::LocalTee(ptr_local));
                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::I32Const(alloc_aligned as i32));
                            out.push(Instruction::MemoryFill(0));
                            out.push(Instruction::LocalGet(ptr_local));
                        }
                        return Ok(());
                    }
                    "complex" => {
                        if call.args.len() < 2 {
                            return Err(Error::InternalError(
                                "complex() requires two arguments".to_string(),
                            ));
                        }
                        let r_vt = self.infer_val_type(&call.args[0], locals);
                        let is_complex64 = r_vt == ValType::F32;
                        let total_size: i32 = if is_complex64 { 8 } else { 16 };
                        let float_align: u32 = if is_complex64 { 2 } else { 3 };

                        self.compile_expression(&call.args[0], out, locals)?;
                        let real_local = locals.add_local(
                            &format!("__cplx_r_{}", locals.locals.len()),
                            if is_complex64 { ValType::F32 } else { ValType::F64 },
                        );
                        out.push(Instruction::LocalSet(real_local));

                        self.compile_expression(&call.args[1], out, locals)?;
                        if !is_complex64 && self.infer_val_type(&call.args[1], locals) == ValType::F32 {
                            out.push(Instruction::F64PromoteF32);
                        }
                        let imag_local = locals.add_local(
                            &format!("__cplx_i_{}", locals.locals.len()),
                            if is_complex64 { ValType::F32 } else { ValType::F64 },
                        );
                        out.push(Instruction::LocalSet(imag_local));

                        out.push(Instruction::I32Const(total_size));
                        out.push(Instruction::Call(self.alloc_func_idx()?));
                        let ptr = locals.add_local(
                            &format!("__cplx_ptr_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::LocalSet(ptr));

                        // Store real part
                        out.push(Instruction::LocalGet(ptr));
                        out.push(Instruction::LocalGet(real_local));
                        if is_complex64 {
                            out.push(Instruction::F32Store(MemArg { offset: 0, align: float_align, memory_index: 0 }));
                        } else {
                            out.push(Instruction::F64Store(MemArg { offset: 0, align: float_align, memory_index: 0 }));
                        }

                        // Store imag part
                        out.push(Instruction::LocalGet(ptr));
                        out.push(Instruction::LocalGet(imag_local));
                        let imag_offset = if is_complex64 { 4u64 } else { 8u64 };
                        if is_complex64 {
                            out.push(Instruction::F32Store(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
                        } else {
                            out.push(Instruction::F64Store(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
                        }

                        out.push(Instruction::LocalGet(ptr));
                        return Ok(());
                    }
                    "real" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let is_64 = self.is_complex64_expr(arg, locals);
                            let align = if is_64 { 2u32 } else { 3u32 };
                            if is_64 {
                                out.push(Instruction::F32Load(MemArg { offset: 0, align, memory_index: 0 }));
                            } else {
                                out.push(Instruction::F64Load(MemArg { offset: 0, align, memory_index: 0 }));
                            }
                        }
                        return Ok(());
                    }
                    "imag" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let is_64 = self.is_complex64_expr(arg, locals);
                            let (imag_offset, align) = if is_64 { (4u64, 2u32) } else { (8u64, 3u32) };
                            if is_64 {
                                out.push(Instruction::F32Load(MemArg { offset: imag_offset, align, memory_index: 0 }));
                            } else {
                                out.push(Instruction::F64Load(MemArg { offset: imag_offset, align, memory_index: 0 }));
                            }
                        }
                        return Ok(());
                    }
                    "min" | "max" => {
                        let is_min = ident.name == "min";
                        if call.args.len() >= 2 && self.is_string_expr(&call.args[0], locals) {
                            self.emit_string_min_max(&call.args, is_min, out, locals)?;
                        } else if call.args.len() >= 2 {
                            self.compile_expression(&call.args[0], out, locals)?;
                            let vt = self.infer_val_type(&call.args[0], locals);
                            let is_unsigned = self.is_unsigned_expr(&call.args[0], locals);
                            for arg in &call.args[1..] {
                                self.compile_expression(arg, out, locals)?;
                                let arg_vt = self.infer_val_type(arg, locals);
                                Self::emit_typed_coerce(arg_vt, vt, out)?;
                                match vt {
                                    ValType::F64 => out.push(if is_min { Instruction::F64Min } else { Instruction::F64Max }),
                                    ValType::F32 => out.push(if is_min { Instruction::F32Min } else { Instruction::F32Max }),
                                    _ => {
                                        let tmp_a = locals.add_local(
                                            &format!("__mm_a_{}", locals.locals.len()),
                                            vt,
                                        );
                                        let tmp_b = locals.add_local(
                                            &format!("__mm_b_{}", locals.locals.len()),
                                            vt,
                                        );
                                        out.push(Instruction::LocalSet(tmp_b));
                                        out.push(Instruction::LocalSet(tmp_a));
                                        out.push(Instruction::LocalGet(tmp_a));
                                        out.push(Instruction::LocalGet(tmp_b));
                                        out.push(Instruction::LocalGet(tmp_a));
                                        out.push(Instruction::LocalGet(tmp_b));
                                        match (vt, is_min) {
                                            (ValType::I64, true) => out.push(if is_unsigned { Instruction::I64LeU } else { Instruction::I64LeS }),
                                            (ValType::I64, false) => out.push(if is_unsigned { Instruction::I64GeU } else { Instruction::I64GeS }),
                                            (_, true) => out.push(if is_unsigned { Instruction::I32LeU } else { Instruction::I32LeS }),
                                            (_, false) => out.push(if is_unsigned { Instruction::I32GeU } else { Instruction::I32GeS }),
                                        }
                                        out.push(Instruction::Select);
                                    }
                                }
                            }
                        } else if call.args.len() == 1 {
                            self.compile_expression(&call.args[0], out, locals)?;
                        }
                        return Ok(());
                    }
                    "println" | "print" => {
                        let is_println = ident.name == "println";
                        if call.args.is_empty() {
                            if is_println {
                                // println() with no args: emit a newline
                                let nl_ptr = locals.add_local("__pln_nl_ptr", ValType::I32);
                                out.push(Instruction::I32Const(1));
                                out.push(Instruction::Call(self.alloc_func_idx()?));
                                out.push(Instruction::LocalTee(nl_ptr));
                                out.push(Instruction::I32Const(b'\n' as i32));
                                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(nl_ptr));
                                out.push(Instruction::I32Const(1));
                            } else {
                                out.push(Instruction::I32Const(0));
                                out.push(Instruction::I32Const(0));
                            }
                        } else if call.args.len() == 1 {
                            self.emit_expr_to_string_on_stack(&call.args[0], out, locals)?;
                            if is_println {
                                self.emit_append_newline(out, locals)?;
                            }
                        } else {
                            // Multiple args: convert each to string, store (ptr,len) pairs
                            let n = call.args.len();
                            let mut arg_ptrs = Vec::with_capacity(n);
                            let mut arg_lens = Vec::with_capacity(n);
                            for (i, arg) in call.args.iter().enumerate() {
                                self.emit_expr_to_string_on_stack(arg, out, locals)?;
                                let l = locals.add_local(&format!("__pln_len_{i}"), ValType::I32);
                                let p = locals.add_local(&format!("__pln_ptr_{i}"), ValType::I32);
                                out.push(Instruction::LocalSet(l));
                                out.push(Instruction::LocalSet(p));
                                arg_ptrs.push(p);
                                arg_lens.push(l);
                            }

                            let separator = if is_println { b' ' } else { 0u8 };
                            let has_sep = is_println;
                            let num_seps = if has_sep { n - 1 } else { 0 };
                            let nl_extra: u32 = if is_println { 1 } else { 0 };

                            // Calculate total length
                            let total = locals.add_local("__pln_total", ValType::I32);
                            out.push(Instruction::I32Const(0));
                            for &l in &arg_lens {
                                out.push(Instruction::LocalGet(l));
                                out.push(Instruction::I32Add);
                            }
                            out.push(Instruction::I32Const((num_seps as i32) + (nl_extra as i32)));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalSet(total));

                            // Allocate buffer
                            let buf = locals.add_local("__pln_buf", ValType::I32);
                            out.push(Instruction::LocalGet(total));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            out.push(Instruction::LocalSet(buf));

                            // Copy each string with separators
                            let offset_local = locals.add_local("__pln_off", ValType::I32);
                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::LocalSet(offset_local));

                            for i in 0..n {
                                // Insert space separator before args after the first (println only)
                                if has_sep && i > 0 {
                                    out.push(Instruction::LocalGet(buf));
                                    out.push(Instruction::LocalGet(offset_local));
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::I32Const(separator as i32));
                                    out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                    out.push(Instruction::LocalGet(offset_local));
                                    out.push(Instruction::I32Const(1));
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::LocalSet(offset_local));
                                }
                                // memory.copy(buf + offset, ptr_i, len_i)
                                out.push(Instruction::LocalGet(buf));
                                out.push(Instruction::LocalGet(offset_local));
                                out.push(Instruction::I32Add);
                                out.push(Instruction::LocalGet(arg_ptrs[i]));
                                out.push(Instruction::LocalGet(arg_lens[i]));
                                out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });
                                // offset += len_i
                                out.push(Instruction::LocalGet(offset_local));
                                out.push(Instruction::LocalGet(arg_lens[i]));
                                out.push(Instruction::I32Add);
                                out.push(Instruction::LocalSet(offset_local));
                            }

                            if is_println {
                                // Append newline
                                out.push(Instruction::LocalGet(buf));
                                out.push(Instruction::LocalGet(offset_local));
                                out.push(Instruction::I32Add);
                                out.push(Instruction::I32Const(b'\n' as i32));
                                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                            }

                            out.push(Instruction::LocalGet(buf));
                            out.push(Instruction::LocalGet(total));
                        }
                        out.push(Instruction::Call(0)); // ctx_log is import index 0
                        return Ok(());
                    }
                    "close" => {
                        return Err(Error::InternalError(
                            "close() is not supported in WASM UDFs (channels are not available)".to_string(),
                        ));
                    }
                    "delete" => {
                        if call.args.len() < 2 {
                            return Err(Error::InternalError(
                                "delete() requires 2 arguments: map and key".to_string(),
                            ));
                        }
                        let map_name = match &call.args[0] {
                            ast::Expression::Ident(id) => id.name.clone(),
                            ast::Expression::Selector(sel) => {
                                if let ast::Expression::Ident(recv) = sel.x.as_ref() {
                                    let synth = format!("{}.{}", recv.name, sel.sel.name);
                                    if locals.map_types.contains_key(&synth) {
                                        synth
                                    } else {
                                        return Err(Error::InternalError(
                                            "delete() first argument: map type info not found for selector expression".to_string(),
                                        ));
                                    }
                                } else {
                                    return Err(Error::InternalError(
                                        "delete() first argument must be a map variable".to_string(),
                                    ));
                                }
                            }
                            _ => {
                                return Err(Error::InternalError(
                                    "delete() first argument must be a map variable".to_string(),
                                ));
                            }
                        };
                        let key_expr = call.args[1].clone();
                        self.compile_map_delete(&map_name, &key_expr, out, locals)?;
                        return Ok(());
                    }
                    "clear" => {
                        if let Some(arg) = call.args.first() {
                            if let ast::Expression::Selector(_sel) = arg {
                                self.compile_expression(arg, out, locals)?;
                                let hdr_local = locals.add_local(
                                    &format!("__clr_sel_hdr_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(hdr_local));

                                let sel_expr = arg;
                                let sel_elem_vt = if let ast::Expression::Selector(sel) = sel_expr {
                                    if let ast::Expression::Ident(recv_id) = sel.x.as_ref() {
                                        locals.slice_elem_types.get(&format!("{}.{}", recv_id.name, sel.sel.name))
                                            .or_else(|| locals.slice_elem_types.get(&recv_id.name))
                                            .copied()
                                            .unwrap_or(ValType::I64)
                                    } else {
                                        ValType::I64
                                    }
                                } else {
                                    ValType::I64
                                };
                                let (sel_elem_size, _) = Self::elem_size_and_align(sel_elem_vt);

                                // nil check: skip if header pointer is 0
                                out.push(Instruction::LocalGet(hdr_local));
                                out.push(Instruction::I32Const(0));
                                out.push(Instruction::I32Ne);
                                out.push(Instruction::If(BlockType::Empty));
                                // memory.fill(data_ptr, 0, len * elem_size)
                                out.push(Instruction::LocalGet(hdr_local));
                                out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                out.push(Instruction::I32Const(0));
                                out.push(Instruction::LocalGet(hdr_local));
                                out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                                out.push(Instruction::I32Const(sel_elem_size));
                                out.push(Instruction::I32Mul);
                                out.push(Instruction::MemoryFill(0));
                                out.push(Instruction::End);
                                return Ok(());
                            }
                            if let ast::Expression::Ident(ident_arg) = arg {
                                if let Some(local_idx) = locals.find(&ident_arg.name) {
                                    let struct_type = locals.get_var_struct_type(&ident_arg.name);
                                    if struct_type == Some("__slice") || locals.slice_elem_types.contains_key(&ident_arg.name) {
                                        let elem_vt = locals.slice_elem_types.get(&ident_arg.name).copied().unwrap_or(ValType::I64);
                                        let (elem_size, _) = Self::elem_size_and_align(elem_vt);
                                        // nil check: skip if slice header pointer is 0
                                        out.push(Instruction::LocalGet(local_idx));
                                        out.push(Instruction::I32Const(0));
                                        out.push(Instruction::I32Ne);
                                        out.push(Instruction::If(BlockType::Empty));
                                        // memory.fill(data_ptr, 0, len * elem_size)
                                        out.push(Instruction::LocalGet(local_idx));
                                        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                        out.push(Instruction::I32Const(0));
                                        out.push(Instruction::LocalGet(local_idx));
                                        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                                        out.push(Instruction::I32Const(elem_size));
                                        out.push(Instruction::I32Mul);
                                        out.push(Instruction::MemoryFill(0));
                                        out.push(Instruction::End);
                                        return Ok(());
                                    }
                                    if struct_type == Some("__map") {
                                        // nil check: skip if map pointer is 0
                                        out.push(Instruction::LocalGet(local_idx));
                                        out.push(Instruction::I32Const(0));
                                        out.push(Instruction::I32Ne);
                                        out.push(Instruction::If(BlockType::Empty));
                                        // Zero out count field (offset 0)
                                        out.push(Instruction::LocalGet(local_idx));
                                        out.push(Instruction::I32Const(0));
                                        out.push(Instruction::I32Store(MemArg {
                                            offset: 0,
                                            align: 2,
                                            memory_index: 0,
                                        }));
                                        // Zero out all entries in the data array
                                        let map_info = locals.map_types.get(&ident_arg.name).cloned();
                                        if let Some(info) = map_info {
                                            let entry_size = Self::map_entry_size(info.key_size, info.val_size);
                                            let cap_tmp = locals.add_local(
                                                &format!("__clr_cap_{}", locals.locals.len()),
                                                ValType::I32,
                                            );
                                            out.push(Instruction::LocalGet(local_idx));
                                            out.push(Instruction::I32Load(MemArg {
                                                offset: 4,
                                                align: 2,
                                                memory_index: 0,
                                            }));
                                            out.push(Instruction::LocalSet(cap_tmp));

                                            let data_ptr_tmp = locals.add_local(
                                                &format!("__clr_dptr_{}", locals.locals.len()),
                                                ValType::I32,
                                            );
                                            out.push(Instruction::LocalGet(local_idx));
                                            out.push(Instruction::I32Load(MemArg {
                                                offset: 8,
                                                align: 2,
                                                memory_index: 0,
                                            }));
                                            out.push(Instruction::LocalSet(data_ptr_tmp));

                                            let total_tmp = locals.add_local(
                                                &format!("__clr_tot_{}", locals.locals.len()),
                                                ValType::I32,
                                            );
                                            out.push(Instruction::LocalGet(cap_tmp));
                                            out.push(Instruction::I32Const(entry_size as i32));
                                            out.push(Instruction::I32Mul);
                                            out.push(Instruction::LocalSet(total_tmp));

                                            let loop_i = locals.add_local(
                                                &format!("__clr_i_{}", locals.locals.len()),
                                                ValType::I32,
                                            );
                                            out.push(Instruction::I32Const(0));
                                            out.push(Instruction::LocalSet(loop_i));

                                            out.push(Instruction::Block(BlockType::Empty));
                                            out.push(Instruction::Loop(BlockType::Empty));

                                            out.push(Instruction::LocalGet(loop_i));
                                            out.push(Instruction::LocalGet(total_tmp));
                                            out.push(Instruction::I32GeU);
                                            out.push(Instruction::BrIf(1));

                                            out.push(Instruction::LocalGet(data_ptr_tmp));
                                            out.push(Instruction::LocalGet(loop_i));
                                            out.push(Instruction::I32Add);
                                            out.push(Instruction::I32Const(0));
                                            out.push(Instruction::I32Store8(MemArg {
                                                offset: 0,
                                                align: 0,
                                                memory_index: 0,
                                            }));

                                            out.push(Instruction::LocalGet(loop_i));
                                            out.push(Instruction::I32Const(1));
                                            out.push(Instruction::I32Add);
                                            out.push(Instruction::LocalSet(loop_i));
                                            out.push(Instruction::Br(0));

                                            out.push(Instruction::End); // loop
                                            out.push(Instruction::End); // block
                                        }
                                        out.push(Instruction::End); // if (nil check)
                                        return Ok(());
                                    }
                                    if struct_type == Some("__array") {
                                        if let Some(&(elem_vt, arr_len)) = locals.array_info.get(&ident_arg.name) {
                                            let (elem_size, _) = Self::elem_size_and_align(elem_vt);
                                            let total_bytes = elem_size as i32 * arr_len as i32;
                                            out.push(Instruction::LocalGet(local_idx));
                                            out.push(Instruction::I32Const(0));
                                            out.push(Instruction::I32Const(total_bytes));
                                            out.push(Instruction::MemoryFill(0));
                                            return Ok(());
                                        }
                                    }
                                }
                            }
                        }
                        return Err(Error::InternalError(
                            "clear() is currently only supported on slices, maps, and arrays".to_string(),
                        ));
                    }
                    _ => {}
                }

                // Check if it's a closure variable or method expression
                if let Some(&(func_idx, env_local)) =
                    locals.closure_info.get(&ident.name)
                {
                    let is_method_expr = locals.method_expr_vars.contains(&ident.name);

                    if is_method_expr {
                        for arg in &call.args {
                            self.compile_expression(arg, out, locals)?;
                        }
                        out.push(Instruction::Call(func_idx));
                        return Ok(());
                    }

                    let cap_info = locals.closure_env_captures.get(&ident.name).cloned().unwrap_or_default();

                    if env_local != u32::MAX && !cap_info.is_empty() {
                        for (cap_name, env_offset, vt) in &cap_info {
                            if let Some(outer_local) = locals.find(cap_name) {
                                out.push(Instruction::LocalGet(env_local));
                                out.push(Instruction::LocalGet(outer_local));
                                let (_, align) = Self::elem_size_and_align(*vt);
                                Self::emit_typed_store(*vt, *env_offset as u64, align, out);
                            }
                        }
                    }

                    if env_local != u32::MAX {
                        out.push(Instruction::LocalGet(env_local));
                    } else {
                        out.push(Instruction::I32Const(0));
                    }
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }
                    out.push(Instruction::Call(func_idx));

                    if env_local != u32::MAX && !cap_info.is_empty() {
                        for (cap_name, env_offset, vt) in &cap_info {
                            if let Some(outer_local) = locals.find(cap_name) {
                                out.push(Instruction::LocalGet(env_local));
                                let (_, align) = Self::elem_size_and_align(*vt);
                                Self::emit_typed_load(*vt, *env_offset as u64, align, out);
                                out.push(Instruction::LocalSet(outer_local));
                            }
                        }
                    }

                    return Ok(());
                }

                // Look up as a user function
                let fi_lookup = self.functions.iter().find(|f| f.name == ident.name).cloned();
                if let Some(func_info) = fi_lookup {
                    if func_info.is_variadic {
                        // Fixed params (excluding variadic)
                        let fixed_count = func_info.params.len() - 1;
                        for arg in call.args.iter().take(fixed_count) {
                            self.compile_expression(arg, out, locals)?;
                        }
                        // Variadic args: pack into a slice
                        let variadic_args = &call.args[fixed_count..];
                        let elem_vt = func_info.variadic_elem_vt.unwrap_or(ValType::I64);
                        let (elem_size, elem_align) = Self::elem_size_and_align(elem_vt);
                        let n_variadic = variadic_args.len() as i32;

                        if call.dots.is_some() && n_variadic == 1 {
                            // s... spreading: arg is already a slice, pass directly
                            self.compile_expression(&variadic_args[0], out, locals)?;
                        } else {
                            // Allocate slice header (12 bytes: ptr, len, cap)
                            let hdr = locals.add_local(&format!("__va_hdr_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::I32Const(12));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            out.push(Instruction::LocalSet(hdr));

                            // Allocate data buffer
                            let data_ptr = locals.add_local(&format!("__va_data_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::I32Const(n_variadic * elem_size));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            out.push(Instruction::LocalSet(data_ptr));

                            // Store each variadic arg
                            for (j, arg) in variadic_args.iter().enumerate() {
                                out.push(Instruction::LocalGet(data_ptr));
                                self.compile_expression(arg, out, locals)?;
                                let arg_vt = self.infer_val_type(arg, locals);
                                Self::emit_typed_coerce(arg_vt, elem_vt, out)?;
                                let offset = j as u64 * elem_size as u64;
                                Self::emit_typed_store(elem_vt, offset, elem_align, out);
                            }

                            // Write header: ptr, len, cap
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(data_ptr));
                            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::I32Const(n_variadic));
                            out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::I32Const(n_variadic));
                            out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

                            out.push(Instruction::LocalGet(hdr));
                        }
                        out.push(Instruction::Call(func_info.wasm_func_idx));
                    } else {
                        // Multi-value call as argument: f(g()) where g() returns
                        // multiple values matching f's parameter count
                        if call.args.len() == 1 {
                            if let ast::Expression::Call(_) = &call.args[0] {
                                let inner_count = self.expression_result_count(&call.args[0], Some(locals));
                                if inner_count > 1 && inner_count == func_info.params.len() {
                                    self.compile_expression(&call.args[0], out, locals)?;
                                    out.push(Instruction::Call(func_info.wasm_func_idx));
                                    return Ok(());
                                }
                            }
                        }
                        for (arg_i, arg) in call.args.iter().enumerate() {
                            self.compile_expression(arg, out, locals)?;
                            if func_info.iface_param_indices.contains(&arg_i) {
                                if let ast::Expression::Ident(arg_ident) = arg {
                                    if arg_ident.name == "nil" {
                                        out.push(Instruction::I32Const(0));
                                    } else if let Some(&tid) = self.iface_var_type_ids.get(&arg_ident.name) {
                                        out.push(Instruction::LocalGet(tid));
                                    } else {
                                        let concrete_type = locals.get_var_struct_type(&arg_ident.name)
                                            .map(|s| s.to_string())
                                            .unwrap_or_else(|| arg_ident.name.clone());
                                        let type_id = self.get_or_create_type_id(&concrete_type);
                                        let rhs_vt = self.infer_val_type(arg, locals);
                                        let (elem_size, _) = Self::elem_size_and_align(rhs_vt);
                                        let box_tmp = locals.add_local(
                                            &format!("__ibox_tmp_{}", locals.locals.len()),
                                            rhs_vt,
                                        );
                                        out.push(Instruction::LocalSet(box_tmp));
                                        let alloc_size = (elem_size as i32).max(8);
                                        out.push(Instruction::I32Const(alloc_size));
                                        out.push(Instruction::Call(self.alloc_func_idx()?));
                                        let box_ptr = locals.add_local(
                                            &format!("__ibox_ptr_{}", locals.locals.len()),
                                            ValType::I32,
                                        );
                                        out.push(Instruction::LocalTee(box_ptr));
                                        out.push(Instruction::LocalGet(box_tmp));
                                        let (_, align) = Self::elem_size_and_align(rhs_vt);
                                        Self::emit_typed_store(rhs_vt, 0, align, out);
                                        out.push(Instruction::LocalGet(box_ptr));
                                        out.push(Instruction::I32Const(type_id as i32));
                                    }
                                } else if let ast::Expression::CompositeLit(comp) = arg {
                                    let concrete_type = if let ast::Expression::Ident(ti) = comp.typ.as_ref() {
                                        ti.name.clone()
                                    } else {
                                        "unknown".to_string()
                                    };
                                    let type_id = self.get_or_create_type_id(&concrete_type);
                                    let rhs_vt = ValType::I32;
                                    let box_tmp = locals.add_local(
                                        &format!("__ibox_tmp_{}", locals.locals.len()),
                                        rhs_vt,
                                    );
                                    out.push(Instruction::LocalSet(box_tmp));
                                    out.push(Instruction::I32Const(8));
                                    out.push(Instruction::Call(self.alloc_func_idx()?));
                                    let box_ptr = locals.add_local(
                                        &format!("__ibox_ptr_{}", locals.locals.len()),
                                        ValType::I32,
                                    );
                                    out.push(Instruction::LocalTee(box_ptr));
                                    out.push(Instruction::LocalGet(box_tmp));
                                    out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalGet(box_ptr));
                                    out.push(Instruction::I32Const(type_id as i32));
                                } else if let ast::Expression::Operation(addr_op) = arg {
                                    if addr_op.op == Operator::And {
                                        let concrete_type = if let ast::Expression::Ident(inner_id) = &*addr_op.x {
                                            locals.get_var_struct_type(&inner_id.name)
                                                .map(|s| s.to_string())
                                                .unwrap_or_else(|| inner_id.name.clone())
                                        } else if let ast::Expression::CompositeLit(comp) = &*addr_op.x {
                                            if let ast::Expression::Ident(ti) = comp.typ.as_ref() {
                                                ti.name.clone()
                                            } else {
                                                "unknown".to_string()
                                            }
                                        } else {
                                            "unknown".to_string()
                                        };
                                        let type_id = self.get_or_create_type_id(&concrete_type);
                                        let rhs_vt = ValType::I32;
                                        let box_tmp = locals.add_local(
                                            &format!("__ibox_tmp_{}", locals.locals.len()),
                                            rhs_vt,
                                        );
                                        out.push(Instruction::LocalSet(box_tmp));
                                        out.push(Instruction::I32Const(8));
                                        out.push(Instruction::Call(self.alloc_func_idx()?));
                                        let box_ptr = locals.add_local(
                                            &format!("__ibox_ptr_{}", locals.locals.len()),
                                            ValType::I32,
                                        );
                                        out.push(Instruction::LocalTee(box_ptr));
                                        out.push(Instruction::LocalGet(box_tmp));
                                        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(box_ptr));
                                        out.push(Instruction::I32Const(type_id as i32));
                                    } else {
                                        out.push(Instruction::I32Const(0));
                                    }
                                } else {
                                    out.push(Instruction::I32Const(0));
                                }
                            }
                        }
                        out.push(Instruction::Call(func_info.wasm_func_idx));
                    }
                } else if self.type_aliases.contains_key(&ident.name) {
                    if let Some(arg) = call.args.first() {
                        self.compile_expression(arg, out, locals)?;
                        let src_vt = self.infer_val_type(arg, locals);
                        let resolved = self.resolve_type_name(&ident.name);
                        let target_vt = Self::val_type_for_type_name(resolved);
                        if src_vt != target_vt {
                            Self::emit_typed_coerce(src_vt, target_vt, out)?;
                        }
                    }
                } else if self.generic_funcs.contains_key(&ident.name) {
                    // Type inference for generic function calls: F(args) instead of F[T](args)
                    let template = self.generic_funcs.get(&ident.name).cloned().ok_or_else(|| {
                        Error::InternalError(format!(
                            "generic function template not found for '{}'", ident.name
                        ))
                    })?;
                    let type_param_names: std::collections::HashSet<String> = template.typ.typ_params.list.iter()
                        .flat_map(|f| f.name.iter().map(|n| n.name.clone()))
                        .collect();

                    let mut subst: HashMap<String, String> = HashMap::new();
                    let mut arg_idx = 0usize;
                    for field in &template.typ.params.list {
                        for _name in &field.name {
                            if arg_idx < call.args.len() {
                                if let Some(arg_type) = self.infer_go_type_from_expr(&call.args[arg_idx], locals) {
                                    Self::try_unify_type_param(&field.typ, &arg_type, &type_param_names, &mut subst);
                                }
                            }
                            arg_idx += 1;
                        }
                    }

                    if subst.len() == type_param_names.len() {
                        let type_args: Vec<String> = template.typ.typ_params.list.iter()
                            .flat_map(|f| f.name.iter())
                            .filter_map(|n| subst.get(&n.name).cloned())
                            .collect();
                        let mono_name = format!("{}__mono_{}", ident.name, type_args.join("_"));

                        if let Some(&func_idx) = self.monomorphized.get(&mono_name) {
                            for arg in &call.args {
                                self.compile_expression(arg, out, locals)?;
                            }
                            out.push(Instruction::Call(func_idx));
                        } else {
                            Self::validate_type_constraints(&template, &subst)?;
                            let specialized = self.monomorphize_func_decl(&template, &mono_name, &subst);
                            let saved = self.generic_funcs.clone();
                            self.compile_func_decl(&specialized, true)?;
                            self.generic_funcs = saved;

                            let func_idx = self.functions.iter()
                                .find(|f| f.name == mono_name)
                                .map(|f| f.wasm_func_idx);
                            if let Some(idx) = func_idx {
                                self.monomorphized.insert(mono_name, idx);
                                for arg in &call.args {
                                    self.compile_expression(arg, out, locals)?;
                                }
                                out.push(Instruction::Call(idx));
                            }
                        }
                    } else {
                        return Err(Error::InternalError(format!(
                            "cannot infer type parameters for generic function: {}",
                            ident.name
                        )));
                    }
                } else if let Some(&func_idx) = self.global_func_vars.get(&ident.name) {
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }
                    out.push(Instruction::Call(func_idx));
                } else if let Some((cap_func_idx, cap_env_local, cap_env_captures)) = self.find_captured_closure(&ident.name) {
                    if !cap_env_captures.is_empty() {
                        for (cap_name, env_offset, vt) in &cap_env_captures {
                            if let Some(outer_local) = locals.find(cap_name) {
                                out.push(Instruction::LocalGet(cap_env_local));
                                out.push(Instruction::LocalGet(outer_local));
                                let (_, align) = Self::elem_size_and_align(*vt);
                                Self::emit_typed_store(*vt, *env_offset as u64, align, out);
                            }
                        }
                    }
                    if cap_env_local != u32::MAX {
                        out.push(Instruction::LocalGet(cap_env_local));
                    } else {
                        out.push(Instruction::I32Const(0));
                    }
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }
                    out.push(Instruction::Call(cap_func_idx));
                    if !cap_env_captures.is_empty() {
                        for (cap_name, env_offset, vt) in &cap_env_captures {
                            if let Some(outer_local) = locals.find(cap_name) {
                                out.push(Instruction::LocalGet(cap_env_local));
                                let (_, align) = Self::elem_size_and_align(*vt);
                                Self::emit_typed_load(*vt, *env_offset as u64, align, out);
                                out.push(Instruction::LocalSet(outer_local));
                            }
                        }
                    }
                } else {
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }
                    return Err(Error::InternalError(format!(
                        "undefined function: {}",
                        ident.name
                    )));
                }
            }
            ast::Expression::Selector(sel) => {
                if let ast::Expression::Ident(pkg_ident) = sel.x.as_ref() {
                    // Handle context method calls (ctx.Log, ctx.QueryID, etc.)
                    if locals.get_var_struct_type(&pkg_ident.name) == Some("__context") {
                        let ctx_host_idx: Option<u32> = match sel.sel.name.as_str() {
                            "Log" => Some(0),
                            "QueryID" => Some(1),
                            "Database" => Some(2),
                            "Schema" => Some(3),
                            "User" => Some(4),
                            "Config" => Some(5),
                            _ => None,
                        };
                        if let Some(host_idx) = ctx_host_idx {
                            if sel.sel.name == "Log" {
                                if let Some(arg) = call.args.first() {
                                    self.compile_expression(arg, out, locals)?;
                                }
                                out.push(Instruction::Call(host_idx));
                                return Ok(());
                            } else if sel.sel.name == "Config" {
                                if let Some(arg) = call.args.first() {
                                    self.compile_expression(arg, out, locals)?;
                                }
                                let key_len = locals.add_local(
                                    &format!("__cfg_key_len_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                let key_ptr = locals.add_local(
                                    &format!("__cfg_key_ptr_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(key_len));
                                out.push(Instruction::LocalSet(key_ptr));

                                let buf_size = 256i32;
                                out.push(Instruction::I32Const(buf_size));
                                out.push(Instruction::Call(self.alloc_func_idx()?));
                                let buf_local = locals.add_local(
                                    &format!("__cfg_buf_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(buf_local));

                                out.push(Instruction::LocalGet(key_ptr));
                                out.push(Instruction::LocalGet(key_len));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::Call(host_idx));
                                let len_local = locals.add_local(
                                    &format!("__cfg_len_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(len_local));

                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::LocalGet(len_local));
                                return Ok(());
                            } else {
                                let buf_size = 256i32;
                                out.push(Instruction::I32Const(buf_size));
                                out.push(Instruction::Call(self.alloc_func_idx()?));
                                let buf_local = locals.add_local(
                                    &format!("__ctx_buf_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(buf_local));

                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::Call(host_idx));
                                let len_local = locals.add_local(
                                    &format!("__ctx_len_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(len_local));

                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::LocalGet(len_local));
                                return Ok(());
                            }
                        }
                    }

                    match (pkg_ident.name.as_str(), sel.sel.name.as_str()) {
                        ("fmt", "Errorf" | "Sprintf") => {
                            return Err(Error::InternalError(format!(
                                "fmt.{} is not yet available; stdlib will be provided as host functions",
                                sel.sel.name
                            )));
                        }
                        ("math", "Sqrt") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Sqrt requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::F64Sqrt);
                            return Ok(());
                        }
                        ("math", "Abs") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Abs requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::F64Abs);
                            return Ok(());
                        }
                        ("math", "Floor") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Floor requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::F64Floor);
                            return Ok(());
                        }
                        ("math", "Ceil") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Ceil requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::F64Ceil);
                            return Ok(());
                        }
                        ("math", "Min") => {
                            if call.args.len() < 2 {
                                return Err(Error::InternalError(
                                    "math.Min requires 2 arguments".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            self.compile_expression(&call.args[1], out, locals)?;
                            out.push(Instruction::F64Min);
                            return Ok(());
                        }
                        ("math", "Max") => {
                            if call.args.len() < 2 {
                                return Err(Error::InternalError(
                                    "math.Max requires 2 arguments".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            self.compile_expression(&call.args[1], out, locals)?;
                            out.push(Instruction::F64Max);
                            return Ok(());
                        }
                        ("math", "Trunc") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Trunc requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::F64Trunc);
                            return Ok(());
                        }
                        ("math", "Round") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Round requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::F64Nearest);
                            return Ok(());
                        }
                        ("math", func_name) => {
                            return Err(Error::InternalError(format!(
                                "unsupported math function: math.{}",
                                func_name
                            )));
                        }
                        (pkg, func_name)
                            if matches!(
                                pkg,
                                "strings" | "strconv" | "sort" | "unicode"
                                    | "bytes" | "errors" | "encoding" | "fmt"
                            ) =>
                        {
                            return Err(Error::InternalError(format!(
                                "{}.{} is not yet implemented; stdlib functions will be available in a future release",
                                pkg, func_name
                            )));
                        }
                        _ => {
                            if locals.find(&pkg_ident.name).is_none()
                                && !self.struct_defs.contains_key(&pkg_ident.name)
                                && !self.type_registry.contains_key(&pkg_ident.name)
                                && !self.is_interface_var(&pkg_ident.name, locals)
                            {
                                return Err(Error::InternalError(format!(
                                    "unsupported package function: {}.{}",
                                    pkg_ident.name, sel.sel.name
                                )));
                            }
                        }
                    }

                    // Check if the receiver is an interface variable
                    if self.is_interface_var(&pkg_ident.name, locals) {
                        return self.compile_interface_method_call(
                            &pkg_ident.name,
                            &sel.sel.name,
                            &call.args,
                            out,
                            locals,
                        );
                    }

                    // Method expression: TypeName.Method(receiver, args...)
                    if (self.struct_defs.contains_key(&pkg_ident.name) || self.type_registry.contains_key(&pkg_ident.name))
                        && locals.find(&pkg_ident.name).is_none()
                    {
                        let qualified = format!("{}.{}", pkg_ident.name, sel.sel.name);
                        if let Some(fi_idx) = self.functions.iter().position(|f| f.name == qualified && f.recv_type.is_some()) {
                            let func_idx = self.functions[fi_idx].wasm_func_idx;
                            for arg in &call.args {
                                self.compile_expression(arg, out, locals)?;
                            }
                            out.push(Instruction::Call(func_idx));
                            return Ok(());
                        }
                    }

                    // Method call on a receiver
                    self.compile_expression(sel.x.as_ref(), out, locals)?;
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }

                    // Resolve the receiver's struct type name for qualified lookup
                    let recv_type_name = locals
                        .get_var_struct_type(&pkg_ident.name)
                        .map(|s| s.to_string());

                    let found = if let Some(ref type_name) = recv_type_name {
                        let qualified = format!("{}.{}", type_name, sel.sel.name);
                        self.functions.iter().find(|f| f.name == qualified).map(|f| f.wasm_func_idx)
                    } else {
                        None
                    };

                    let method_suffix = format!(".{}", sel.sel.name);
                    let func_idx = found
                        .or_else(|| {
                            let qualified = format!("{}.{}", pkg_ident.name, sel.sel.name);
                            self.functions.iter().find(|f| f.name == qualified).map(|f| f.wasm_func_idx)
                        })
                        .or_else(|| {
                            if let Some(ref type_name) = recv_type_name {
                                let qualified = format!("{}{}", type_name, method_suffix);
                                self.functions.iter().find(|f| f.recv_type.is_some() && f.name == qualified).map(|f| f.wasm_func_idx)
                            } else {
                                let matches: Vec<_> = self.functions.iter()
                                    .filter(|f| f.recv_type.is_some() && f.name.ends_with(&method_suffix))
                                    .collect();
                                if matches.len() == 1 {
                                    Some(matches[0].wasm_func_idx)
                                } else {
                                    None
                                }
                            }
                        });

                    if let Some(idx) = func_idx {
                        out.push(Instruction::Call(idx));
                    } else {
                        // Check embedded types for promoted methods
                        let mut embed_found = false;
                        if let Some(ref type_name) = recv_type_name {
                            if let Some(sdef) = self.struct_defs.get(type_name).cloned() {
                                for (embed_type, embed_offset) in &sdef.embedded_types {
                                    let qualified = format!("{}.{}", embed_type, sel.sel.name);
                                    if let Some(fi) = self.functions.iter().find(|f| f.name == qualified) {
                                        // Adjust receiver: add embedding offset
                                        // The receiver is already on the stack (pushed for args earlier)
                                        // We need to insert the offset adjustment before the call
                                        // The receiver was the first arg pushed. Rewrite: pop all args,
                                        // adjust receiver, push args back, call.
                                        // Simplification: insert I32Const(offset) + I32Add before the args
                                        // were pushed. Since args are already on stack, we can't easily do this.
                                        // Instead, let's re-emit: receiver was pushed as first thing.
                                        // We'll adjust by emitting before args were compiled above.
                                        // Actually, since we already compiled args, we need to work with what's on the stack.
                                        // For now, handle the common case: no extra args besides receiver.
                                        let embed_func_idx = fi.wasm_func_idx;
                                        let wasm_params: Vec<_> = fi.params.iter()
                                            .skip(1) // skip receiver
                                            .map(|(_, wt)| wt.to_val_type())
                                            .collect();
                                        if wasm_params.is_empty() && *embed_offset == 0 {
                                            out.push(Instruction::Call(embed_func_idx));
                                        } else {
                                            let mut saved = Vec::new();
                                            for (j, vt) in wasm_params.iter().enumerate().rev() {
                                                let tmp = locals.add_local(
                                                    &format!("__embed_arg_{}_{}", j, locals.locals.len()),
                                                    *vt
                                                );
                                                out.push(Instruction::LocalSet(tmp));
                                                saved.push(tmp);
                                            }
                                            if *embed_offset > 0 {
                                                out.push(Instruction::I32Const(*embed_offset as i32));
                                                out.push(Instruction::I32Add);
                                            }
                                            for tmp in saved.iter().rev() {
                                                out.push(Instruction::LocalGet(*tmp));
                                            }
                                            out.push(Instruction::Call(embed_func_idx));
                                        }
                                        embed_found = true;
                                        break;
                                    }
                                }
                            }
                        }
                        if !embed_found {
                            return Err(Error::InternalError(format!(
                                "undefined method: {}.{}",
                                pkg_ident.name, sel.sel.name
                            )));
                        }
                    }
                } else {
                    // Non-ident receiver: method chaining (e.g., b.Add(10).Add(20))
                    let recv_type = self.infer_struct_type_from_expr(sel.x.as_ref(), locals);
                    if let Some(type_name) = recv_type {
                        self.compile_expression(sel.x.as_ref(), out, locals)?;
                        for arg in &call.args {
                            self.compile_expression(arg, out, locals)?;
                        }
                        let qualified = format!("{}.{}", type_name, sel.sel.name);
                        let method_suffix = format!(".{}", sel.sel.name);
                        let func_idx = self.functions.iter()
                            .find(|f| f.name == qualified)
                            .or_else(|| {
                                self.functions.iter()
                                    .find(|f| f.recv_type.is_some() && f.name.ends_with(&method_suffix))
                            })
                            .map(|f| f.wasm_func_idx);
                        if let Some(idx) = func_idx {
                            out.push(Instruction::Call(idx));
                        } else {
                            return Err(Error::InternalError(format!(
                                "undefined method: {}.{}",
                                type_name, sel.sel.name
                            )));
                        }
                    } else {
                        return Err(Error::InternalError(format!(
                            "cannot infer receiver type for method call .{}",
                            sel.sel.name
                        )));
                    }
                }
            }
            ast::Expression::TypeSlice(slice_type) => {
                // Type conversion: []byte(str) or []rune(str)
                if let ast::Expression::Ident(elem_ident) = slice_type.typ.as_ref() {
                    if let Some(arg) = call.args.first() {
                        if self.is_string_expr(arg, locals) && (elem_ident.name == "byte" || elem_ident.name == "uint8") {
                            // []byte(str): unpack string bytes into 4-byte I32 slots
                            self.compile_expression(arg, out, locals)?;
                            let str_len = locals.add_local(&format!("__sb_len_{}", locals.locals.len()), ValType::I32);
                            let str_ptr = locals.add_local(&format!("__sb_ptr_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalSet(str_len));
                            out.push(Instruction::LocalSet(str_ptr));

                            out.push(Instruction::I32Const(12));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            let hdr = locals.add_local(&format!("__sb_hdr_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalSet(hdr));

                            // Allocate str_len * 4 bytes (each byte stored as I32)
                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32Const(4));
                            out.push(Instruction::I32Mul);
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            let data = locals.add_local(&format!("__sb_data_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalSet(data));

                            // Loop: unpack each byte into a 4-byte slot
                            let idx_l = locals.add_local(&format!("__sb_idx_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::LocalSet(idx_l));

                            out.push(Instruction::Block(BlockType::Empty));
                            out.push(Instruction::Loop(BlockType::Empty));

                            out.push(Instruction::LocalGet(idx_l));
                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32GeU);
                            out.push(Instruction::BrIf(1));

                            // data[idx*4] = str_ptr[idx] (load byte, store as I32)
                            out.push(Instruction::LocalGet(data));
                            out.push(Instruction::LocalGet(idx_l));
                            out.push(Instruction::I32Const(4));
                            out.push(Instruction::I32Mul);
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalGet(str_ptr));
                            out.push(Instruction::LocalGet(idx_l));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

                            out.push(Instruction::LocalGet(idx_l));
                            out.push(Instruction::I32Const(1));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalSet(idx_l));
                            out.push(Instruction::Br(0));

                            out.push(Instruction::End); // loop
                            out.push(Instruction::End); // block

                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(data));
                            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

                            out.push(Instruction::LocalGet(hdr));
                            return Ok(());
                        }
                        if self.is_string_expr(arg, locals) && (elem_ident.name == "rune" || elem_ident.name == "int32") {
                            // []rune(str): decode UTF-8 into a []int32 slice
                            self.compile_expression(arg, out, locals)?;
                            let str_len = locals.add_local(&format!("__sr_len_{}", locals.locals.len()), ValType::I32);
                            let str_ptr = locals.add_local(&format!("__sr_ptr_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalSet(str_len));
                            out.push(Instruction::LocalSet(str_ptr));

                            // Worst case: each byte is a rune, allocate str_len * 4 bytes
                            out.push(Instruction::I32Const(12));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            let hdr = locals.add_local(&format!("__sr_hdr_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalSet(hdr));

                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32Const(4));
                            out.push(Instruction::I32Mul);
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            let data = locals.add_local(&format!("__sr_data_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalSet(data));

                            // Decode loop: src_idx iterates over bytes, dst_idx over runes
                            let src_idx = locals.add_local(&format!("__sr_si_{}", locals.locals.len()), ValType::I32);
                            let dst_idx = locals.add_local(&format!("__sr_di_{}", locals.locals.len()), ValType::I32);
                            let byte0 = locals.add_local(&format!("__sr_b0_{}", locals.locals.len()), ValType::I32);
                            let addr = locals.add_local(&format!("__sr_addr_{}", locals.locals.len()), ValType::I32);
                            let rune_v = locals.add_local(&format!("__sr_rv_{}", locals.locals.len()), ValType::I32);
                            let rune_w = locals.add_local(&format!("__sr_rw_{}", locals.locals.len()), ValType::I32);

                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::LocalSet(src_idx));
                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::LocalSet(dst_idx));

                            out.push(Instruction::Block(BlockType::Empty));
                            out.push(Instruction::Loop(BlockType::Empty));

                            // Break if src_idx >= str_len
                            out.push(Instruction::LocalGet(src_idx));
                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32GeU);
                            out.push(Instruction::BrIf(1));

                            // addr = str_ptr + src_idx
                            out.push(Instruction::LocalGet(str_ptr));
                            out.push(Instruction::LocalGet(src_idx));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalSet(addr));

                            // byte0 = mem[addr]
                            out.push(Instruction::LocalGet(addr));
                            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                            out.push(Instruction::LocalSet(byte0));

                            // Default: width=1, rune=byte0
                            out.push(Instruction::I32Const(1));
                            out.push(Instruction::LocalSet(rune_w));
                            out.push(Instruction::LocalGet(byte0));
                            out.push(Instruction::LocalSet(rune_v));

                            // Multi-byte UTF-8 decode with boundary checks
                            out.push(Instruction::LocalGet(byte0));
                            out.push(Instruction::I32Const(0x80));
                            out.push(Instruction::I32GeU);
                            out.push(Instruction::If(BlockType::Empty));
                            {
                                // 2-byte: (byte0 & 0xE0) == 0xC0
                                out.push(Instruction::LocalGet(byte0));
                                out.push(Instruction::I32Const(0xE0));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Const(0xC0));
                                out.push(Instruction::I32Eq);
                                out.push(Instruction::If(BlockType::Empty));
                                {
                                    // Boundary check: src_idx + 2 <= str_len
                                    out.push(Instruction::LocalGet(src_idx));
                                    out.push(Instruction::I32Const(2));
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::LocalGet(str_len));
                                    out.push(Instruction::I32LeU);
                                    out.push(Instruction::If(BlockType::Empty));
                                    {
                                        out.push(Instruction::I32Const(2));
                                        out.push(Instruction::LocalSet(rune_w));
                                        out.push(Instruction::LocalGet(byte0));
                                        out.push(Instruction::I32Const(0x1F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Const(6));
                                        out.push(Instruction::I32Shl);
                                        out.push(Instruction::LocalGet(addr));
                                        out.push(Instruction::I32Load8U(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::LocalSet(rune_v));
                                    }
                                    out.push(Instruction::Else);
                                    {
                                        out.push(Instruction::I32Const(0xFFFD));
                                        out.push(Instruction::LocalSet(rune_v));
                                    }
                                    out.push(Instruction::End);
                                }
                                out.push(Instruction::Else);
                                {
                                    // 3-byte: (byte0 & 0xF0) == 0xE0
                                    out.push(Instruction::LocalGet(byte0));
                                    out.push(Instruction::I32Const(0xF0));
                                    out.push(Instruction::I32And);
                                    out.push(Instruction::I32Const(0xE0));
                                    out.push(Instruction::I32Eq);
                                    out.push(Instruction::If(BlockType::Empty));
                                    {
                                        // Boundary check: src_idx + 3 <= str_len
                                        out.push(Instruction::LocalGet(src_idx));
                                        out.push(Instruction::I32Const(3));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalGet(str_len));
                                        out.push(Instruction::I32LeU);
                                        out.push(Instruction::If(BlockType::Empty));
                                        {
                                            out.push(Instruction::I32Const(3));
                                            out.push(Instruction::LocalSet(rune_w));
                                            out.push(Instruction::LocalGet(byte0));
                                            out.push(Instruction::I32Const(0x0F));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Const(12));
                                            out.push(Instruction::I32Shl);
                                            out.push(Instruction::LocalGet(addr));
                                            out.push(Instruction::I32Load8U(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                            out.push(Instruction::I32Const(0x3F));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Const(6));
                                            out.push(Instruction::I32Shl);
                                            out.push(Instruction::I32Or);
                                            out.push(Instruction::LocalGet(addr));
                                            out.push(Instruction::I32Load8U(MemArg { offset: 2, align: 0, memory_index: 0 }));
                                            out.push(Instruction::I32Const(0x3F));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Or);
                                            out.push(Instruction::LocalSet(rune_v));
                                        }
                                        out.push(Instruction::Else);
                                        {
                                            out.push(Instruction::I32Const(0xFFFD));
                                            out.push(Instruction::LocalSet(rune_v));
                                        }
                                        out.push(Instruction::End);
                                    }
                                    out.push(Instruction::Else);
                                    {
                                        // 4-byte: assume (byte0 & 0xF8) == 0xF0
                                        // Boundary check: src_idx + 4 <= str_len
                                        out.push(Instruction::LocalGet(src_idx));
                                        out.push(Instruction::I32Const(4));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalGet(str_len));
                                        out.push(Instruction::I32LeU);
                                        out.push(Instruction::If(BlockType::Empty));
                                        {
                                            out.push(Instruction::I32Const(4));
                                            out.push(Instruction::LocalSet(rune_w));
                                            out.push(Instruction::LocalGet(byte0));
                                            out.push(Instruction::I32Const(0x07));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Const(18));
                                            out.push(Instruction::I32Shl);
                                            out.push(Instruction::LocalGet(addr));
                                            out.push(Instruction::I32Load8U(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                            out.push(Instruction::I32Const(0x3F));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Const(12));
                                            out.push(Instruction::I32Shl);
                                            out.push(Instruction::I32Or);
                                            out.push(Instruction::LocalGet(addr));
                                            out.push(Instruction::I32Load8U(MemArg { offset: 2, align: 0, memory_index: 0 }));
                                            out.push(Instruction::I32Const(0x3F));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Const(6));
                                            out.push(Instruction::I32Shl);
                                            out.push(Instruction::I32Or);
                                            out.push(Instruction::LocalGet(addr));
                                            out.push(Instruction::I32Load8U(MemArg { offset: 3, align: 0, memory_index: 0 }));
                                            out.push(Instruction::I32Const(0x3F));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Or);
                                            out.push(Instruction::LocalSet(rune_v));
                                        }
                                        out.push(Instruction::Else);
                                        {
                                            out.push(Instruction::I32Const(0xFFFD));
                                            out.push(Instruction::LocalSet(rune_v));
                                        }
                                        out.push(Instruction::End);
                                    }
                                    out.push(Instruction::End);
                                }
                                out.push(Instruction::End);
                            }
                            out.push(Instruction::End);

                            // Store rune at data[dst_idx * 4]
                            out.push(Instruction::LocalGet(data));
                            out.push(Instruction::LocalGet(dst_idx));
                            out.push(Instruction::I32Const(4));
                            out.push(Instruction::I32Mul);
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalGet(rune_v));
                            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

                            // src_idx += rune_w, dst_idx += 1
                            out.push(Instruction::LocalGet(src_idx));
                            out.push(Instruction::LocalGet(rune_w));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalSet(src_idx));
                            out.push(Instruction::LocalGet(dst_idx));
                            out.push(Instruction::I32Const(1));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalSet(dst_idx));

                            out.push(Instruction::Br(0));
                            out.push(Instruction::End); // end loop
                            out.push(Instruction::End); // end block

                            // Fill header: data_ptr, len=dst_idx, cap=str_len
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(data));
                            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(dst_idx));
                            out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

                            out.push(Instruction::LocalGet(hdr));
                            return Ok(());
                        }
                    }
                }
                return Err(Error::InternalError(format!(
                    "unsupported slice type conversion: {:?}",
                    call.func
                )));
            }
            ast::Expression::TypeArray(arr_type) => {
                // Slice-to-array conversion: [N]T(slice)
                if let Some(arg) = call.args.first() {
                    let arr_len = if let ast::Expression::BasicLit(lit) = arr_type.len.as_ref() {
                        Self::parse_go_int(&lit.value)
                            .map_err(|e| Error::SyntaxError(e))? as u32
                    } else {
                        return Err(Error::InternalError(
                            "slice-to-array conversion requires a constant array length".to_string(),
                        ));
                    };
                    let elem_vt = Self::infer_array_elem_vt(&arr_type.typ);
                    let (elem_size, _) = Self::elem_size_and_align(elem_vt);

                    self.compile_expression(arg, out, locals)?;
                    let slice_hdr = locals.add_local(&format!("__s2a_hdr_{}", locals.locals.len()), ValType::I32);
                    out.push(Instruction::LocalSet(slice_hdr));

                    // Load slice length and check >= arr_len
                    out.push(Instruction::LocalGet(slice_hdr));
                    out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                    out.push(Instruction::I32Const(arr_len as i32));
                    out.push(Instruction::I32LtU);
                    out.push(Instruction::If(BlockType::Empty));
                    out.push(Instruction::Unreachable);
                    out.push(Instruction::End);

                    // Allocate array memory
                    let total_bytes = elem_size * arr_len as i32;
                    out.push(Instruction::I32Const(total_bytes));
                    out.push(Instruction::Call(self.alloc_func_idx()?));
                    let arr_ptr = locals.add_local(&format!("__s2a_ptr_{}", locals.locals.len()), ValType::I32);
                    out.push(Instruction::LocalSet(arr_ptr));

                    // Copy elements: memory.copy(arr_ptr, slice_data_ptr, total_bytes)
                    out.push(Instruction::LocalGet(arr_ptr));
                    out.push(Instruction::LocalGet(slice_hdr));
                    out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                    out.push(Instruction::I32Const(total_bytes));
                    out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

                    out.push(Instruction::LocalGet(arr_ptr));
                    return Ok(());
                }
                return Err(Error::InternalError(
                    "slice-to-array conversion requires exactly one argument".to_string(),
                ));
            }
            ast::Expression::Index(idx_expr)
                if idx_expr.left.as_ref().map_or(false, |l| {
                    if let ast::Expression::Ident(id) = l.as_ref() {
                        self.generic_funcs.contains_key(&id.name)
                    } else {
                        false
                    }
                }) =>
            {
                // Generic function call with single type arg: F[T](args)
                let func_ident = if let Some(ast::Expression::Ident(id)) = idx_expr.left.as_deref() {
                    id
                } else {
                    return Err(Error::InternalError(
                        "generic function call requires an identifier as the function name".to_string(),
                    ));
                };
                let type_arg = if let ast::Expression::Ident(ti) = idx_expr.index.as_ref() {
                    ti.name.clone()
                } else {
                    format!("{:?}", idx_expr.index)
                };
                let type_args = vec![type_arg];
                let mono_name = format!("{}__mono_{}", func_ident.name, type_args.join("_"));

                if let Some(&func_idx) = self.monomorphized.get(&mono_name) {
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }
                    out.push(Instruction::Call(func_idx));
                    return Ok(());
                }

                let generic_decl = self.generic_funcs.get(&func_ident.name).cloned();
                if let Some(template) = generic_decl {
                    let mut subst: HashMap<String, String> = HashMap::new();
                    for (i, field) in template.typ.typ_params.list.iter().enumerate() {
                        for name_ident in &field.name {
                            if i < type_args.len() {
                                subst.insert(name_ident.name.clone(), type_args[i].clone());
                            }
                        }
                    }
                    Self::validate_type_constraints(&template, &subst)?;
                    let specialized = self.monomorphize_func_decl(&template, &mono_name, &subst);
                    let saved_generic_funcs = self.generic_funcs.clone();
                    self.compile_func_decl(&specialized, true)?;
                    self.generic_funcs = saved_generic_funcs;

                    let func_idx = self.functions.iter()
                        .find(|f| f.name == mono_name)
                        .map(|f| f.wasm_func_idx);
                    if let Some(idx) = func_idx {
                        self.monomorphized.insert(mono_name.clone(), idx);
                        for arg in &call.args {
                            self.compile_expression(arg, out, locals)?;
                        }
                        out.push(Instruction::Call(idx));
                    }
                    return Ok(());
                }

                return Err(Error::InternalError(format!(
                    "undefined generic function: {}",
                    func_ident.name
                )));
            }
            ast::Expression::IndexList(idx_list) => {
                // Generic function call: F[T1, T2, ...](args)
                if let ast::Expression::Ident(func_ident) = idx_list.left.as_ref() {
                    let type_args: Vec<String> = idx_list.indices.iter().map(|idx| {
                        if let ast::Expression::Ident(ti) = idx {
                            ti.name.clone()
                        } else {
                            format!("{:?}", idx)
                        }
                    }).collect();

                    let mono_name = format!("{}__mono_{}", func_ident.name, type_args.join("_"));

                    // Check if already monomorphized
                    if let Some(&func_idx) = self.monomorphized.get(&mono_name) {
                        for arg in &call.args {
                            self.compile_expression(arg, out, locals)?;
                        }
                        out.push(Instruction::Call(func_idx));
                        return Ok(());
                    }

                    // Look up the generic function template
                    let generic_decl = self.generic_funcs.get(&func_ident.name).cloned();
                    if let Some(template) = generic_decl {
                        // Build type parameter substitution map
                        let mut subst: HashMap<String, String> = HashMap::new();
                        for (i, field) in template.typ.typ_params.list.iter().enumerate() {
                            for name_ident in &field.name {
                                if i < type_args.len() {
                                    subst.insert(name_ident.name.clone(), type_args[i].clone());
                                }
                            }
                        }

                        // Validate type constraints before monomorphization
                        Self::validate_type_constraints(&template, &subst)?;
                        let specialized = self.monomorphize_func_decl(&template, &mono_name, &subst);
                        let saved_generic_funcs = self.generic_funcs.clone();
                        self.compile_func_decl(&specialized, true)?;
                        self.generic_funcs = saved_generic_funcs;

                        // Record the monomorphized func_idx
                        let func_idx = self.functions.iter()
                            .find(|f| f.name == mono_name)
                            .map(|f| f.wasm_func_idx);
                        if let Some(idx) = func_idx {
                            self.monomorphized.insert(mono_name.clone(), idx);
                            for arg in &call.args {
                                self.compile_expression(arg, out, locals)?;
                            }
                            out.push(Instruction::Call(idx));
                        }
                        return Ok(());
                    }

                    return Err(Error::InternalError(format!(
                        "undefined generic function: {}",
                        func_ident.name
                    )));
                }
                return Err(Error::InternalError(format!(
                    "unsupported generic call expression: {:?}",
                    call.func
                )));
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported call expression: {:?}",
                    call.func
                )));
            }
        }
        Ok(())
    }

    fn compile_selector(
        &mut self,
        sel: &ast::Selector,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        self.compile_expression(&sel.x, out, locals)?;

        let struct_type_name = self.infer_struct_type_from_expr(sel.x.as_ref(), locals);

        if let Some(type_name) = struct_type_name {
            if let Some(struct_def) = self.struct_defs.get(&type_name) {
                if let Some(field) = struct_def.find_field(&sel.sel.name) {
                    let offset = field.offset as u64;
                    match field.wasm_type {
                        WasmType::I64 => out.push(Instruction::I64Load(MemArg {
                            offset,
                            align: 3,
                            memory_index: 0,
                        })),
                        WasmType::F64 => out.push(Instruction::F64Load(MemArg {
                            offset,
                            align: 3,
                            memory_index: 0,
                        })),
                        WasmType::F32 => out.push(Instruction::F32Load(MemArg {
                            offset,
                            align: 2,
                            memory_index: 0,
                        })),
                        WasmType::I32 => out.push(Instruction::I32Load(MemArg {
                            offset,
                            align: 2,
                            memory_index: 0,
                        })),
                    }
                    return Ok(());
                }
            }
        }

        // Check if this is a method expression (Type.Method used as a value)
        if let ast::Expression::Ident(type_ident) = sel.x.as_ref() {
            let is_type_name = self.struct_defs.contains_key(&type_ident.name)
                || self.type_aliases.contains_key(&type_ident.name);
            if is_type_name {
                let method_qname = format!("{}.{}", type_ident.name, sel.sel.name);
                if let Some(fi) = self.functions.iter().find(|f| f.name == method_qname) {
                    let idx = fi.wasm_func_idx;
                    out.pop(); // Remove the result of compile_expression(sel.x) which tried to load type name as variable
                    self.last_func_value_idx = Some(idx);
                    self.last_is_method_expr = true;
                    out.push(Instruction::I32Const(idx as i32));
                    return Ok(());
                }
            }
        }

        // Check if this is a method value (x.Method used as a value, not called)
        if let ast::Expression::Ident(recv_ident) = sel.x.as_ref() {
            let recv_type = locals.get_var_struct_type(&recv_ident.name)
                .map(|s| s.to_string());
            if let Some(type_name) = recv_type {
                let method_qname = format!("{}.{}", type_name, sel.sel.name);
                if let Some(fi) = self.functions.iter().find(|f| f.name == method_qname) {
                    let idx = fi.wasm_func_idx;
                    let recv_local = locals.add_local(
                        &format!("__mval_recv_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(recv_local));
                    self.last_closure_env = Some(recv_local);
                    self.last_closure_func_idx = Some(idx);
                    out.push(Instruction::I32Const(idx as i32));
                    return Ok(());
                }
            }
        }

        let sel_name = if let ast::Expression::Ident(ident) = sel.x.as_ref() {
            format!("{}.{}", ident.name, sel.sel.name)
        } else {
            format!("<expr>.{}", sel.sel.name)
        };
        Err(Error::InternalError(format!(
            "unresolved selector: {}",
            sel_name
        )))
    }

    fn compile_func_lit(
        &mut self,
        func_lit: &ast::FuncLit,
        out: &mut Vec<Instruction<'static>>,
        outer_locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let func_idx = self.next_func_idx;

        let mut go_param_names: Vec<String> = Vec::new();
        let mut go_param_types: Vec<ValType> = Vec::new();
        for field in &func_lit.typ.params.list {
            let wts = self.field_to_wasm_types(field);
            if field.name.is_empty() {
                for wt in &wts {
                    go_param_types.push(wt.to_val_type());
                    go_param_names.push(format!("_param{}", go_param_names.len()));
                }
            } else {
                for (j, ident) in field.name.iter().enumerate() {
                    if j < wts.len() {
                        go_param_types.push(wts[j].to_val_type());
                    } else if !wts.is_empty() {
                        go_param_types.push(wts[0].to_val_type());
                    }
                    go_param_names.push(ident.name.clone());
                }
            }
        }

        let mut result_types: Vec<ValType> = Vec::new();
        for field in &func_lit.typ.result.list {
            let wts = self.field_to_wasm_types(field);
            for wt in wts {
                result_types.push(wt.to_val_type());
            }
        }

        // Always include env_ptr as hidden first parameter
        let mut full_param_types: Vec<ValType> = vec![ValType::I32];
        full_param_types.extend_from_slice(&go_param_types);

        let type_idx = self.next_type_idx;
        self.type_section
            .ty()
            .function(full_param_types.clone(), result_types.clone());
        self.next_type_idx += 1;

        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let closure_name = format!("__closure_{}", func_idx);
        let wasm_params: Vec<(String, WasmType)> = go_param_names
            .iter()
            .zip(go_param_types.iter())
            .map(|(n, vt)| {
                (
                    n.clone(),
                    match vt {
                        ValType::I32 => WasmType::I32,
                        ValType::I64 => WasmType::I64,
                        ValType::F32 => WasmType::F32,
                        ValType::F64 => WasmType::F64,
                        _ => WasmType::I32,
                    },
                )
            })
            .collect();
        let wasm_results: Vec<WasmType> = result_types
            .iter()
            .map(|vt| match vt {
                ValType::I32 => WasmType::I32,
                ValType::I64 => WasmType::I64,
                ValType::F32 => WasmType::F32,
                ValType::F64 => WasmType::F64,
                _ => WasmType::I32,
            })
            .collect();
        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: closure_name,
            params: wasm_params,
            results: wasm_results,
            result_go_types: vec![],
            is_exported: false,
            recv_type: None,
            is_variadic: false,
            variadic_elem_vt: None,
            iface_param_indices: vec![],
        });

        // Build inner locals: env_ptr + go params
        let mut inner_params: Vec<(String, ValType)> =
            vec![("__env_ptr".to_string(), ValType::I32)];
        for (name, vt) in go_param_names.iter().zip(go_param_types.iter()) {
            inner_params.push((name.clone(), *vt));
        }
        let mut inner_locals = LocalAlloc::new(inner_params);

        // Collect named return variables (same logic as compile_func_decl)
        let mut named_returns: Vec<(String, ValType)> = Vec::new();
        for field in &func_lit.typ.result.list {
            let field_wasm_types = self.field_to_wasm_types(field);
            for (i, ident) in field.name.iter().enumerate() {
                let vt = if i < field_wasm_types.len() {
                    field_wasm_types[i].to_val_type()
                } else if !field_wasm_types.is_empty() {
                    field_wasm_types[0].to_val_type()
                } else {
                    ValType::I64
                };
                let _local_idx = inner_locals.add_local(&ident.name, vt);
                named_returns.push((ident.name.clone(), vt));
            }
        }

        // Set up capture state
        let outer_snapshot = outer_locals.all_entries();
        self.closure_captures = Some(ClosureCaptureState {
            outer_locals: outer_snapshot,
            captures: Vec::new(),
            outer_closure_info: outer_locals.closure_info.clone(),
            outer_closure_env_captures: outer_locals.closure_env_captures.clone(),
        });

        let saved_named_returns = std::mem::replace(&mut self.named_returns, named_returns.clone());
        let saved_result_types = std::mem::replace(&mut self.current_result_types, result_types.clone());
        let saved_result_go_types = std::mem::replace(&mut self.current_result_go_types, vec![]);

        let mut body: Vec<Instruction<'static>> = Vec::new();
        self.deferred_calls.push(Vec::new());
        self.compile_block(&func_lit.body, &mut body, &mut inner_locals, &result_types)?;
        self.emit_deferred_calls(&mut body);
        self.deferred_calls.pop();

        self.named_returns = saved_named_returns;
        self.current_result_types = saved_result_types;
        self.current_result_go_types = saved_result_go_types;

        // Extract captures
        let captures = if let Some(cc) = self.closure_captures.take() {
            cc.captures
        } else {
            Vec::new()
        };

        self.last_closure_captures = captures.clone();

        // In outer function: allocate env and store captures
        if let Some(last) = captures.last() {
            let env_size: i32 =
                (last.env_offset + val_type_byte_size(last.val_type)) as i32;
            out.push(Instruction::I32Const(env_size));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            let env_local = outer_locals.add_local("__env_ptr_outer", ValType::I32);
            out.push(Instruction::LocalSet(env_local));

            for cap in &captures {
                out.push(Instruction::LocalGet(env_local));
                out.push(Instruction::LocalGet(cap.outer_local_idx));
                match cap.val_type {
                    ValType::I64 => out.push(Instruction::I64Store(MemArg {
                        offset: cap.env_offset as u64,
                        align: 3,
                        memory_index: 0,
                    })),
                    ValType::F64 => out.push(Instruction::F64Store(MemArg {
                        offset: cap.env_offset as u64,
                        align: 3,
                        memory_index: 0,
                    })),
                    ValType::F32 => out.push(Instruction::F32Store(MemArg {
                        offset: cap.env_offset as u64,
                        align: 2,
                        memory_index: 0,
                    })),
                    _ => out.push(Instruction::I32Store(MemArg {
                        offset: cap.env_offset as u64,
                        align: 2,
                        memory_index: 0,
                    })),
                }
            }

            self.last_closure_env = Some(env_local);
        } else {
            self.last_closure_env = None;
        }

        self.last_closure_func_idx = Some(func_idx);

        // Termination analysis (mirrors compile_func_decl logic)
        let body_always_returns = Self::block_always_returns(&func_lit.body.list);

        if body_always_returns {
            if !result_types.is_empty()
                && body
                    .last()
                    .map_or(true, |i| !matches!(i, Instruction::Return))
            {
                body.push(Instruction::Unreachable);
            }
        } else if result_types.is_empty()
            || body
                .last()
                .map_or(true, |i| !matches!(i, Instruction::Return))
        {
            if !named_returns.is_empty() {
                for (name, _vt) in &named_returns {
                    if let Some(idx) = inner_locals.find(name) {
                        body.push(Instruction::LocalGet(idx));
                    }
                }
            } else if !result_types.is_empty() {
                return Err(Error::SyntaxError(
                    "missing return in function literal".to_string(),
                ));
            }
        }
        body.push(Instruction::End);

        let mut func = Function::new(inner_locals.local_types());
        for instr in &body {
            func.instruction(instr);
        }
        self.pending_closures.push(func);

        // Push func_idx as the closure value
        out.push(Instruction::I32Const(func_idx as i32));

        Ok(())
    }

    fn infer_array_elem_vt(elem_type: &ast::Expression) -> ValType {
        match elem_type {
            ast::Expression::Ident(id) => match id.name.as_str() {
                "int" | "int64" | "uint" | "uint64" => ValType::I64,
                "float64" => ValType::F64,
                "float32" => ValType::F32,
                "int32" | "uint32" | "byte" | "uint8" | "int16" | "uint16" | "bool" => ValType::I32,
                _ => ValType::I32,
            },
            ast::Expression::TypeSlice(_)
            | ast::Expression::TypeArray(_)
            | ast::Expression::TypeMap(_)
            | ast::Expression::TypePointer(_)
            | ast::Expression::TypeStruct(_)
            | ast::Expression::TypeInterface(_) => ValType::I32,
            _ => ValType::I64,
        }
    }

    fn compile_array_literal(
        &mut self,
        arr_type: &ast::ArrayType,
        lit_val: &ast::LiteralValue,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let arr_len = if let ast::Expression::BasicLit(lit) = arr_type.len.as_ref() {
            Self::parse_go_int(&lit.value).map_err(|e| Error::SyntaxError(e))? as u32
        } else if matches!(arr_type.len.as_ref(), ast::Expression::Ellipsis(_)) {
            lit_val.values.len() as u32
        } else {
            return Err(Error::InternalError("array length must be a constant".to_string()));
        };

        let elem_vt = Self::infer_array_elem_vt(&arr_type.typ);
        let (elem_size, align) = Self::elem_size_and_align(elem_vt);
        let total_bytes = (arr_len as i32).checked_mul(elem_size).ok_or_else(|| {
            Error::InternalError(format!(
                "array too large: [{}]T with element size {} bytes overflows",
                arr_len, elem_size
            ))
        })?;

        out.push(Instruction::I32Const(total_bytes));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let ptr_local = locals.add_local(
            &format!("__arr_ptr_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(ptr_local));

        for (i, kv) in lit_val.values.iter().enumerate() {
            if let ast::Element::Expr(expr) = &kv.val {
                out.push(Instruction::LocalGet(ptr_local));
                self.compile_expression(expr, out, locals)?;
                let val_vt = self.infer_val_type(expr, locals);
                Self::emit_typed_coerce(val_vt, elem_vt, out)?;
                let offset = i as u64 * elem_size as u64;
                Self::emit_typed_store(elem_vt, offset, align, out);
            }
        }

        out.push(Instruction::LocalGet(ptr_local));
        Ok(())
    }

    fn compile_slice_literal(
        &mut self,
        slice_type: &ast::SliceType,
        lit_val: &ast::LiteralValue,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let elem_vt = Self::infer_array_elem_vt(&slice_type.typ);
        let (elem_size, align) = Self::elem_size_and_align(elem_vt);
        let n_elems = lit_val.values.len() as i32;
        let data_bytes = n_elems * elem_size;
        const HEADER_SIZE: i32 = 12;

        out.push(Instruction::I32Const(HEADER_SIZE));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let hdr_local = locals.add_local(
            &format!("__slit_hdr_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(hdr_local));

        out.push(Instruction::I32Const(data_bytes));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let data_local = locals.add_local(
            &format!("__slit_data_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(data_local));

        let is_inner_slice = matches!(slice_type.typ.as_ref(), ast::Expression::TypeSlice(_));

        // Resolve the element type name for composite literal type elision
        let elem_type_name = if let ast::Expression::Ident(id) = slice_type.typ.as_ref() {
            Some(id.name.clone())
        } else {
            None
        };
        let is_struct_elem = elem_type_name.as_ref().map_or(false, |n| self.struct_defs.contains_key(n));

        for (i, kv) in lit_val.values.iter().enumerate() {
            out.push(Instruction::LocalGet(data_local));
            if is_inner_slice {
                let inner_lit_val = match &kv.val {
                    ast::Element::LitValue(lv) => Some(lv),
                    ast::Element::Expr(ast::Expression::CompositeLit(comp)) => Some(&comp.val),
                    _ => None,
                };
                if let (Some(lv), ast::Expression::TypeSlice(inner_st)) = (inner_lit_val, slice_type.typ.as_ref()) {
                    self.compile_slice_literal(inner_st, lv, out, locals)?;
                } else if let ast::Element::Expr(expr) = &kv.val {
                    self.compile_expression(expr, out, locals)?;
                } else {
                    continue;
                }
            } else if is_struct_elem {
                // Type elision: []StructType{{field values...}} — inner literal inherits the element type
                let inner_lit_val = match &kv.val {
                    ast::Element::LitValue(lv) => Some(lv),
                    ast::Element::Expr(ast::Expression::CompositeLit(comp)) => {
                        // Already has explicit type, compile normally
                        self.compile_expression(&ast::Expression::CompositeLit(comp.clone()), out, locals)?;
                        None
                    }
                    ast::Element::Expr(expr) => {
                        self.compile_expression(expr, out, locals)?;
                        let val_vt = self.infer_val_type(expr, locals);
                        Self::emit_typed_coerce(val_vt, elem_vt, out)?;
                        None
                    }
                };
                if let Some(lv) = inner_lit_val {
                    let resolved_name = elem_type_name.clone().ok_or_else(|| Error::InternalError(
                        "slice composite literal: could not determine element type name".to_string(),
                    ))?;
                    let synth_comp = ast::CompositeLit {
                        typ: Box::new(ast::Expression::Ident(ast::Ident {
                            pos: 0,
                            name: resolved_name,
                        })),
                        val: lv.clone(),
                    };
                    self.compile_composite_lit(&synth_comp, out, locals)?;
                }
            } else if let ast::Element::Expr(expr) = &kv.val {
                self.compile_expression(expr, out, locals)?;
                let val_vt = self.infer_val_type(expr, locals);
                Self::emit_typed_coerce(val_vt, elem_vt, out)?;
            } else {
                continue;
            }
            let offset = i as u64 * elem_size as u64;
            Self::emit_typed_store(elem_vt, offset, align, out);
        }

        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::I32Const(n_elems));
        out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::I32Const(n_elems));
        out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

        out.push(Instruction::LocalGet(hdr_local));
        Ok(())
    }

    fn compile_map_literal(
        &mut self,
        map_type: &ast::MapType,
        lit_val: &ast::LiteralValue,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let key_vt = Self::infer_array_elem_vt(&map_type.key);
        let val_vt = Self::infer_array_elem_vt(&map_type.val);
        let is_string_key = matches!(map_type.key.as_ref(), ast::Expression::Ident(id) if id.name == "string");
        let is_string_val = matches!(map_type.val.as_ref(), ast::Expression::Ident(id) if id.name == "string");
        let key_size = if is_string_key { 8u32 } else { val_type_byte_size(key_vt) };
        let val_size = if is_string_val { 8u32 } else { val_type_byte_size(val_vt) };
        let entry_size = Self::map_entry_size(key_size, val_size);
        let initial_cap = std::cmp::max(lit_val.values.len() as i32, 8);

        // Allocate map header (12 bytes: count, capacity, data_ptr)
        let tmp_name = format!("__mlit_{}", locals.locals.len());
        out.push(Instruction::I32Const(12));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let map_local = locals.add_local(&tmp_name, ValType::I32);
        out.push(Instruction::LocalSet(map_local));

        // Allocate data region
        out.push(Instruction::I32Const(initial_cap * entry_size as i32));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let data_local = locals.add_local(
            &format!("__mlit_data_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(data_local));

        // Init header: count=0, capacity, data_ptr
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Const(initial_cap));
        out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

        // Register as a map type so compile_map_set can find it
        let val_struct_type = if let ast::Expression::Ident(id) = map_type.val.as_ref() {
            if self.struct_defs.contains_key(&id.name) {
                Some(id.name.clone())
            } else {
                None
            }
        } else {
            None
        };
        locals.set_var_struct_type(&tmp_name, "__map");
        let nested = self.build_nested_map_type_info(map_type);
        locals.map_types.insert(tmp_name.clone(), MapTypeInfo {
            key_vt,
            val_vt,
            key_size,
            val_size,
            is_string_key,
            is_string_val,
            val_struct_type,
            nested_map_val_type: nested,
        });

        // Insert each key-value pair
        for kv in &lit_val.values {
            let key_expr = match &kv.key {
                Some(ast::Element::Expr(e)) => e,
                _ => return Err(Error::InternalError(
                    "map literal entry must have a key".to_string(),
                )),
            };
            let val_expr = match &kv.val {
                ast::Element::Expr(e) => e,
                _ => return Err(Error::InternalError(
                    "map literal entry must have a value".to_string(),
                )),
            };

            // Compile the value into a temp local
            self.compile_expression(val_expr, out, locals)?;
            let val_vt_actual = self.infer_val_type(val_expr, locals);
            let val_tmp = locals.add_local(
                &format!("__mlit_v_{}", locals.locals.len()),
                val_vt_actual,
            );
            let val_len_tmp = if is_string_val {
                let vl = locals.add_local(
                    &format!("__mlit_vl_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalSet(vl));
                out.push(Instruction::LocalSet(val_tmp));
                Some(vl)
            } else {
                out.push(Instruction::LocalSet(val_tmp));
                None
            };

            self.compile_map_set(
                &tmp_name, key_expr, val_tmp, val_len_tmp,
                val_vt_actual, out, locals,
            )?;
        }

        out.push(Instruction::LocalGet(map_local));
        Ok(())
    }

    fn compile_composite_lit(
        &mut self,
        comp: &ast::CompositeLit,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        // Handle array literals: [N]T{v1, v2, ...}
        if let ast::Expression::TypeArray(arr_type) = comp.typ.as_ref() {
            return self.compile_array_literal(arr_type, &comp.val, out, locals);
        }

        // Handle slice literals: []T{v1, v2, ...}
        if let ast::Expression::TypeSlice(slice_type) = comp.typ.as_ref() {
            return self.compile_slice_literal(slice_type, &comp.val, out, locals);
        }

        // Handle map literals: map[K]V{k1: v1, ...}
        if let ast::Expression::TypeMap(map_type) = comp.typ.as_ref() {
            return self.compile_map_literal(map_type, &comp.val, out, locals);
        }

        // Handle named composite types: MySlice{1,2,3} → resolve to underlying type
        if let ast::Expression::Ident(ident) = comp.typ.as_ref() {
            if let Some(underlying) = self.named_composite_types.get(&ident.name).cloned() {
                match &underlying {
                    ast::Expression::TypeSlice(slice_type) => {
                        return self.compile_slice_literal(slice_type, &comp.val, out, locals);
                    }
                    ast::Expression::TypeMap(map_type) => {
                        return self.compile_map_literal(map_type, &comp.val, out, locals);
                    }
                    ast::Expression::TypeArray(arr_type) => {
                        return self.compile_array_literal(arr_type, &comp.val, out, locals);
                    }
                    _ => {}
                }
            }
        }

        // Handle generic type instantiation in composite literals: Pair[int]{...}
        let type_name = if let ast::Expression::Index(idx) = comp.typ.as_ref() {
            if let Some(ast::Expression::Ident(type_ident)) = idx.left.as_ref().map(|l| l.as_ref()) {
                if self.generic_types.contains_key(&type_ident.name) {
                    let type_arg = Self::type_expr_to_go_string(&idx.index);
                    let mono_name = self.monomorphize_generic_type(&type_ident.name, &[type_arg])?;
                    Some(mono_name)
                } else { Some(type_ident.name.clone()) }
            } else { None }
        } else if let ast::Expression::IndexList(idxl) = comp.typ.as_ref() {
            if let ast::Expression::Ident(type_ident) = idxl.left.as_ref() {
                if self.generic_types.contains_key(&type_ident.name) {
                    let type_args: Vec<String> = idxl.indices.iter()
                        .map(|e| Self::type_expr_to_go_string(e))
                        .collect();
                    let mono_name = self.monomorphize_generic_type(&type_ident.name, &type_args)?;
                    Some(mono_name)
                } else { Some(type_ident.name.clone()) }
            } else { None }
        } else if let ast::Expression::Ident(ident) = comp.typ.as_ref() {
            Some(ident.name.clone())
        } else {
            None
        };

        let struct_def = type_name
            .as_ref()
            .and_then(|n| self.struct_defs.get(n))
            .cloned();

        let total_size = if let Some(ref sd) = struct_def {
            sd.total_size as i32
        } else {
            let field_count = comp.val.values.len();
            ((field_count * 8) as i32).max(8)
        };

        out.push(Instruction::I32Const(total_size));
        out.push(Instruction::Call(self.alloc_func_idx()?));

        let ptr_local = locals.add_local("__comp_ptr", ValType::I32);
        out.push(Instruction::LocalSet(ptr_local));

        for (i, kv) in comp.val.values.iter().enumerate() {
            if let ast::Element::LitValue(nested_lit) = &kv.val {
                // Check if this is an embedded struct field (should be stored inline)
                let embed_info = if let Some(ref key) = kv.key {
                    if let ast::Element::Expr(ast::Expression::Ident(key_ident)) = key {
                        struct_def.as_ref().and_then(|sd|
                            sd.embedded_types.iter()
                                .find(|(name, _)| name == &key_ident.name)
                                .map(|(name, off)| (name.clone(), *off))
                        )
                    } else { None }
                } else if let Some(ref sd) = struct_def {
                    if i < sd.fields.len() {
                        sd.embedded_types.iter()
                            .find(|(name, _)| name == &sd.fields[i].name)
                            .map(|(name, off)| (name.clone(), *off))
                    } else { None }
                } else { None };

                if let Some((embed_type_name, embed_offset)) = embed_info {
                    // Embedded struct: write fields inline into the parent struct
                    let inner_struct_def = self.struct_defs.get(&embed_type_name).cloned();
                    self.compile_embedded_lit_inline(
                        nested_lit, ptr_local, embed_offset as u64,
                        inner_struct_def.as_ref(), out, locals,
                    )?;
                    continue;
                }

                // Non-embedded nested composite literal: allocate separately
                let inner_type_name = if let Some(ref key) = kv.key {
                    if let ast::Element::Expr(ast::Expression::Ident(key_ident)) = key {
                        struct_def.as_ref().and_then(|sd| sd.find_field(&key_ident.name)).and_then(|_| {
                            locals.get_var_struct_type(&key_ident.name).map(|s| s.to_string())
                        })
                    } else { None }
                } else if let Some(ref sd) = struct_def {
                    if i < sd.fields.len() {
                        self.struct_defs.iter().find_map(|(name, def)| {
                            if def.total_size == sd.fields[i].wasm_type.byte_size() || sd.fields[i].wasm_type == WasmType::I32 {
                                Some(name.clone())
                            } else { None }
                        })
                    } else { None }
                } else { None };

                let inner_total = if let Some(ref itn) = inner_type_name {
                    self.struct_defs.get(itn).map_or(8, |sd| sd.total_size) as i32
                } else {
                    (nested_lit.values.len() * 8).max(8) as i32
                };

                out.push(Instruction::I32Const(inner_total));
                out.push(Instruction::Call(self.alloc_func_idx()?));
                let inner_ptr = locals.add_local(
                    &format!("__nested_ptr_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalSet(inner_ptr));

                let inner_struct_def = inner_type_name
                    .as_ref()
                    .and_then(|n| self.struct_defs.get(n))
                    .cloned();

                for (j, inner_kv) in nested_lit.values.iter().enumerate() {
                    if let ast::Element::Expr(inner_expr) = &inner_kv.val {
                        let (off, fwt) = if let Some(ref ikey) = inner_kv.key {
                            if let ast::Element::Expr(ast::Expression::Ident(kid)) = ikey {
                                if let Some(ref isd) = inner_struct_def {
                                    if let Some(f) = isd.find_field(&kid.name) {
                                        (f.offset as u64, Some(f.wasm_type))
                                    } else { (j as u64 * 8, None) }
                                } else { (j as u64 * 8, None) }
                            } else { (j as u64 * 8, None) }
                        } else if let Some(ref isd) = inner_struct_def {
                            if j < isd.fields.len() {
                                (isd.fields[j].offset as u64, Some(isd.fields[j].wasm_type))
                            } else { (j as u64 * 8, None) }
                        } else { (j as u64 * 8, None) };

                        out.push(Instruction::LocalGet(inner_ptr));
                        self.compile_expression(inner_expr, out, locals)?;
                        let vt = fwt.map(|wt| wt.to_val_type())
                            .unwrap_or_else(|| self.infer_val_type(inner_expr, locals));
                        Self::emit_typed_store(vt, off, if vt == ValType::I64 || vt == ValType::F64 { 3 } else { 2 }, out);
                    }
                }

                // Store inner pointer into outer struct field
                let (offset, _) = if let Some(ref key) = kv.key {
                    if let ast::Element::Expr(ast::Expression::Ident(key_ident)) = key {
                        if let Some(ref sd) = struct_def {
                            if let Some(field) = sd.find_field(&key_ident.name) {
                                (field.offset as u64, Some(field.wasm_type))
                            } else { (i as u64 * 8, None) }
                        } else { (i as u64 * 8, None) }
                    } else { (i as u64 * 8, None) }
                } else if let Some(ref sd) = struct_def {
                    if i < sd.fields.len() {
                        (sd.fields[i].offset as u64, Some(sd.fields[i].wasm_type))
                    } else { (i as u64 * 8, None) }
                } else { (i as u64 * 8, None) };

                out.push(Instruction::LocalGet(ptr_local));
                out.push(Instruction::LocalGet(inner_ptr));
                out.push(Instruction::I32Store(MemArg {
                    offset,
                    align: 2,
                    memory_index: 0,
                }));
                continue;
            }

            let elem_expr = match &kv.val {
                ast::Element::Expr(e) => e,
                _ => {
                    return Err(Error::InternalError(
                        "struct composite literal requires expression values".to_string(),
                    ));
                }
            };

            // Check if this is an embedded struct via CompositeLit expression
            if let ast::Expression::CompositeLit(inner_comp) = elem_expr {
                let embed_info = if let Some(ref key) = kv.key {
                    if let ast::Element::Expr(ast::Expression::Ident(key_ident)) = key {
                        struct_def.as_ref().and_then(|sd|
                            sd.embedded_types.iter()
                                .find(|(name, _)| name == &key_ident.name)
                                .map(|(name, off)| (name.clone(), *off))
                        )
                    } else { None }
                } else if let Some(ref sd) = struct_def {
                    if i < sd.fields.len() {
                        sd.embedded_types.iter()
                            .find(|(name, _)| name == &sd.fields[i].name)
                            .map(|(name, off)| (name.clone(), *off))
                    } else { None }
                } else { None };

                if let Some((embed_type_name, embed_offset)) = embed_info {
                    let inner_struct_def = self.struct_defs.get(&embed_type_name).cloned();
                    self.compile_embedded_lit_inline(
                        &inner_comp.val, ptr_local, embed_offset as u64,
                        inner_struct_def.as_ref(), out, locals,
                    )?;
                    continue;
                }
            }

            // Determine offset and type from struct layout
            let (offset, field_wasm_type) = if let Some(ref key) = kv.key {
                if let ast::Element::Expr(ast::Expression::Ident(key_ident)) = key {
                    if let Some(ref sd) = struct_def {
                        if let Some(field) = sd.find_field(&key_ident.name) {
                            (field.offset as u64, Some(field.wasm_type))
                        } else {
                            return Err(Error::InternalError(format!(
                                "unknown field '{}' in struct literal",
                                key_ident.name
                            )));
                        }
                    } else {
                        return Err(Error::InternalError(
                            "struct definition not found for composite literal".to_string(),
                        ));
                    }
                } else {
                    return Err(Error::InternalError(
                        "unsupported key expression in composite literal".to_string(),
                    ));
                }
            } else if let Some(ref sd) = struct_def {
                if i < sd.fields.len() {
                    (
                        sd.fields[i].offset as u64,
                        Some(sd.fields[i].wasm_type),
                    )
                } else {
                    return Err(Error::InternalError(format!(
                        "too many fields in struct literal: got {}, struct has {}",
                        i + 1,
                        sd.fields.len()
                    )));
                }
            } else {
                return Err(Error::InternalError(
                    "struct definition not found for composite literal".to_string(),
                ));
            };

            out.push(Instruction::LocalGet(ptr_local));
            self.compile_expression(elem_expr, out, locals)?;

            let target_vt = field_wasm_type
                .map(|wt| wt.to_val_type())
                .unwrap_or_else(|| self.infer_val_type(elem_expr, locals));
            let expr_vt = self.infer_val_type(elem_expr, locals);

            // Coerce expression type to field type if needed
            if expr_vt != target_vt {
                Self::emit_typed_coerce(expr_vt, target_vt, out)?;
            }

            match target_vt {
                ValType::I64 => out.push(Instruction::I64Store(MemArg {
                    offset,
                    align: 3,
                    memory_index: 0,
                })),
                ValType::F64 => out.push(Instruction::F64Store(MemArg {
                    offset,
                    align: 3,
                    memory_index: 0,
                })),
                ValType::I32 => out.push(Instruction::I32Store(MemArg {
                    offset,
                    align: 2,
                    memory_index: 0,
                })),
                ValType::F32 => out.push(Instruction::F32Store(MemArg {
                    offset,
                    align: 2,
                    memory_index: 0,
                })),
                _ => out.push(Instruction::I32Store(MemArg {
                    offset,
                    align: 2,
                    memory_index: 0,
                })),
            }
        }

        out.push(Instruction::LocalGet(ptr_local));

        Ok(())
    }

    fn compile_slice_expr(
        &mut self,
        slice: &ast::Slice,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let is_string = self.is_string_expr(&slice.left, locals);

        // Detect __slice header variables so we can load data_ptr/len/cap from header
        let is_slice_header = if let ast::Expression::Ident(ident) = &*slice.left {
            locals.get_var_struct_type(&ident.name) == Some("__slice")
        } else {
            false
        };

        self.compile_expression(&slice.left, out, locals)?;

        let base_local = locals.add_local(
            &format!("__slice_base_{}", locals.locals.len()),
            ValType::I32,
        );
        let orig_len_local = locals.add_local(
            &format!("__slice_len_{}", locals.locals.len()),
            ValType::I32,
        );
        let orig_cap_local = locals.add_local(
            &format!("__slice_cap_{}", locals.locals.len()),
            ValType::I32,
        );

        if is_string {
            out.push(Instruction::LocalSet(orig_len_local));
            out.push(Instruction::LocalSet(base_local));
            out.push(Instruction::LocalGet(orig_len_local));
            out.push(Instruction::LocalSet(orig_cap_local));
        } else if is_slice_header {
            // __slice header: load data_ptr, len, cap from header
            let hdr_tmp = locals.add_local(
                &format!("__slx_hdr_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalSet(hdr_tmp));
            out.push(Instruction::LocalGet(hdr_tmp));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(base_local));
            out.push(Instruction::LocalGet(hdr_tmp));
            out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(orig_len_local));
            out.push(Instruction::LocalGet(hdr_tmp));
            out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(orig_cap_local));
        } else {
            let expr_count = self.expression_result_count(&slice.left, Some(locals));
            if expr_count >= 3 {
                out.push(Instruction::LocalSet(orig_cap_local));
                out.push(Instruction::LocalSet(orig_len_local));
                out.push(Instruction::LocalSet(base_local));
            } else if expr_count >= 2 {
                out.push(Instruction::LocalSet(orig_len_local));
                out.push(Instruction::LocalSet(base_local));
                out.push(Instruction::LocalGet(orig_len_local));
                out.push(Instruction::LocalSet(orig_cap_local));
            } else {
                out.push(Instruction::LocalSet(base_local));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(orig_len_local));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(orig_cap_local));
            }
        }

        let low_local = locals.add_local(
            &format!("__slice_lo_{}", locals.locals.len()),
            ValType::I32,
        );
        if let Some(ref lo) = slice.index[0] {
            self.compile_expression(lo, out, locals)?;
            let vt = self.infer_val_type(lo, locals);
            if vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }
        } else {
            out.push(Instruction::I32Const(0));
        }
        out.push(Instruction::LocalSet(low_local));

        let high_local = locals.add_local(
            &format!("__slice_hi_{}", locals.locals.len()),
            ValType::I32,
        );
        if let Some(ref hi) = slice.index[1] {
            self.compile_expression(hi, out, locals)?;
            let vt = self.infer_val_type(hi, locals);
            if vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }
        } else {
            out.push(Instruction::LocalGet(orig_len_local));
        }
        out.push(Instruction::LocalSet(high_local));

        // Bounds checks: 0 <= low <= high <= len (strings/arrays) or cap (slices)
        // Check low <= high
        out.push(Instruction::LocalGet(low_local));
        out.push(Instruction::LocalGet(high_local));
        out.push(Instruction::I32GtU);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::Unreachable);
        out.push(Instruction::End);

        if is_string {
            // For strings: high <= len
            out.push(Instruction::LocalGet(high_local));
            out.push(Instruction::LocalGet(orig_len_local));
            out.push(Instruction::I32GtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
        } else {
            // For slices: high <= cap
            out.push(Instruction::LocalGet(high_local));
            out.push(Instruction::LocalGet(orig_cap_local));
            out.push(Instruction::I32GtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
        }

        let three_index_max_local = if let Some(ref max_expr) = slice.index[2] {
            // Three-index slice: additionally check high <= max <= cap
            let max_local = locals.add_local(
                &format!("__slice_max_{}", locals.locals.len()),
                ValType::I32,
            );
            self.compile_expression(max_expr, out, locals)?;
            let vt = self.infer_val_type(max_expr, locals);
            if vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }
            out.push(Instruction::LocalSet(max_local));

            // high <= max
            out.push(Instruction::LocalGet(high_local));
            out.push(Instruction::LocalGet(max_local));
            out.push(Instruction::I32GtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);

            // max <= cap
            out.push(Instruction::LocalGet(max_local));
            out.push(Instruction::LocalGet(orig_cap_local));
            out.push(Instruction::I32GtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);

            Some(max_local)
        } else {
            None
        };

        let slice_elem_vt = if let ast::Expression::Ident(ident) = &*slice.left {
            locals
                .slice_elem_types
                .get(&ident.name)
                .copied()
                .unwrap_or(ValType::I64)
        } else {
            ValType::I64
        };
        let elem_size = if is_string { 1i32 } else { Self::elem_size_and_align(slice_elem_vt).0 };

        if is_string {
            // String slice: push (new_ptr, new_len)
            // new_ptr = base + low
            out.push(Instruction::LocalGet(base_local));
            out.push(Instruction::LocalGet(low_local));
            out.push(Instruction::I32Add);

            // new_len = high - low
            out.push(Instruction::LocalGet(high_local));
            out.push(Instruction::LocalGet(low_local));
            out.push(Instruction::I32Sub);
        } else {
            // Non-string slice: allocate a 12-byte slice header and push header pointer
            let new_hdr = locals.add_local(
                &format!("__slx_nhdr_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::I32Const(12));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(new_hdr));

            // new_ptr = base + low * elem_size
            out.push(Instruction::LocalGet(new_hdr));
            out.push(Instruction::LocalGet(base_local));
            out.push(Instruction::LocalGet(low_local));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

            // new_len = high - low
            out.push(Instruction::LocalGet(new_hdr));
            out.push(Instruction::LocalGet(high_local));
            out.push(Instruction::LocalGet(low_local));
            out.push(Instruction::I32Sub);
            out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

            // new_cap = max - low (three-index) or orig_cap - low (two-index)
            out.push(Instruction::LocalGet(new_hdr));
            if let Some(max_local) = three_index_max_local {
                out.push(Instruction::LocalGet(max_local));
            } else {
                out.push(Instruction::LocalGet(orig_cap_local));
            }
            out.push(Instruction::LocalGet(low_local));
            out.push(Instruction::I32Sub);
            out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

            out.push(Instruction::LocalGet(new_hdr));
        }

        Ok(())
    }

    fn emit_i64_to_string(
        &mut self,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        // Converts an i64 on the stack to a string (ptr, len).
        // Algorithm: write digits in reverse, then reverse in place.
        // Max i64 has 20 digits + sign = 21 chars.
        let val = locals.add_local(&format!("__itos_v_{}", locals.locals.len()), ValType::I64);
        let buf = locals.add_local(&format!("__itos_buf_{}", locals.locals.len()), ValType::I32);
        let pos = locals.add_local(&format!("__itos_pos_{}", locals.locals.len()), ValType::I32);
        let neg = locals.add_local(&format!("__itos_neg_{}", locals.locals.len()), ValType::I32);
        let slen = locals.add_local(&format!("__itos_len_{}", locals.locals.len()), ValType::I32);
        let final_ptr = locals.add_local(&format!("__itos_fp_{}", locals.locals.len()), ValType::I32);

        out.push(Instruction::LocalSet(val));

        // Allocate 24-byte temp buffer
        out.push(Instruction::I32Const(24));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        out.push(Instruction::LocalSet(buf));

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(pos));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(neg));

        // Handle zero
        out.push(Instruction::LocalGet(val));
        out.push(Instruction::I64Eqz);
        out.push(Instruction::If(BlockType::Empty));
        {
            out.push(Instruction::LocalGet(buf));
            out.push(Instruction::I32Const(48)); // '0'
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::LocalSet(pos));
        }
        out.push(Instruction::Else);
        {
            // Handle negative
            out.push(Instruction::LocalGet(val));
            out.push(Instruction::I64Const(0));
            out.push(Instruction::I64LtS);
            out.push(Instruction::If(BlockType::Empty));
            {
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(neg));
                out.push(Instruction::I64Const(0));
                out.push(Instruction::LocalGet(val));
                out.push(Instruction::I64Sub);
                out.push(Instruction::LocalSet(val));
            }
            out.push(Instruction::End);

            // Extract digits in reverse
            out.push(Instruction::Block(BlockType::Empty));
            out.push(Instruction::Loop(BlockType::Empty));
            out.push(Instruction::LocalGet(val));
            out.push(Instruction::I64Eqz);
            out.push(Instruction::BrIf(1));

            // digit = val % 10
            out.push(Instruction::LocalGet(buf));
            out.push(Instruction::LocalGet(pos));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(val));
            out.push(Instruction::I64Const(10));
            out.push(Instruction::I64RemU);
            out.push(Instruction::I32WrapI64);
            out.push(Instruction::I32Const(48)); // '0'
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

            // val = val / 10
            out.push(Instruction::LocalGet(val));
            out.push(Instruction::I64Const(10));
            out.push(Instruction::I64DivU);
            out.push(Instruction::LocalSet(val));

            out.push(Instruction::LocalGet(pos));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(pos));
            out.push(Instruction::Br(0));
            out.push(Instruction::End);
            out.push(Instruction::End);

            // Reverse the digits in buf[0..pos]
            let lo = locals.add_local(&format!("__itos_lo_{}", locals.locals.len()), ValType::I32);
            let hi = locals.add_local(&format!("__itos_hi_{}", locals.locals.len()), ValType::I32);
            let tmp = locals.add_local(&format!("__itos_tmp_{}", locals.locals.len()), ValType::I32);

            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(lo));
            out.push(Instruction::LocalGet(pos));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Sub);
            out.push(Instruction::LocalSet(hi));

            out.push(Instruction::Block(BlockType::Empty));
            out.push(Instruction::Loop(BlockType::Empty));
            out.push(Instruction::LocalGet(lo));
            out.push(Instruction::LocalGet(hi));
            out.push(Instruction::I32GeU);
            out.push(Instruction::BrIf(1));

            // swap buf[lo] and buf[hi]
            out.push(Instruction::LocalGet(buf));
            out.push(Instruction::LocalGet(lo));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::LocalSet(tmp));

            out.push(Instruction::LocalGet(buf));
            out.push(Instruction::LocalGet(lo));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(buf));
            out.push(Instruction::LocalGet(hi));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

            out.push(Instruction::LocalGet(buf));
            out.push(Instruction::LocalGet(hi));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(tmp));
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

            out.push(Instruction::LocalGet(lo));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(lo));
            out.push(Instruction::LocalGet(hi));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Sub);
            out.push(Instruction::LocalSet(hi));
            out.push(Instruction::Br(0));
            out.push(Instruction::End);
            out.push(Instruction::End);
        }
        out.push(Instruction::End); // end if zero/nonzero

        // Build final string: if negative, prepend '-'
        out.push(Instruction::LocalGet(neg));
        out.push(Instruction::LocalGet(pos));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(slen));

        out.push(Instruction::LocalGet(slen));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        out.push(Instruction::LocalSet(final_ptr));

        out.push(Instruction::LocalGet(neg));
        out.push(Instruction::If(BlockType::Empty));
        {
            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::I32Const(45)); // '-'
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
        }
        out.push(Instruction::End);

        // Copy digits from buf to final_ptr+neg
        out.push(Instruction::LocalGet(final_ptr));
        out.push(Instruction::LocalGet(neg));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalGet(buf));
        out.push(Instruction::LocalGet(pos));
        out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

        // Push (ptr, len)
        out.push(Instruction::LocalGet(final_ptr));
        out.push(Instruction::LocalGet(slen));

        Ok(())
    }

    fn emit_f64_to_string(
        &mut self,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let fval = locals.add_local(&format!("__ftos_v_{}", locals.locals.len()), ValType::F64);
        let abs_val = locals.add_local(&format!("__ftos_abs_{}", locals.locals.len()), ValType::F64);
        let is_neg = locals.add_local(&format!("__ftos_neg_{}", locals.locals.len()), ValType::I32);
        let int_part = locals.add_local(&format!("__ftos_ip_{}", locals.locals.len()), ValType::I64);
        let frac_val = locals.add_local(&format!("__ftos_fv_{}", locals.locals.len()), ValType::F64);
        let frac_int = locals.add_local(&format!("__ftos_fi_{}", locals.locals.len()), ValType::I64);
        let int_ptr = locals.add_local(&format!("__ftos_iptr_{}", locals.locals.len()), ValType::I32);
        let int_len = locals.add_local(&format!("__ftos_ilen_{}", locals.locals.len()), ValType::I32);
        let frac_buf = locals.add_local(&format!("__ftos_fb_{}", locals.locals.len()), ValType::I32);
        let frac_pos = locals.add_local(&format!("__ftos_fp_{}", locals.locals.len()), ValType::I32);
        let final_ptr = locals.add_local(&format!("__ftos_rp_{}", locals.locals.len()), ValType::I32);
        let total_len = locals.add_local(&format!("__ftos_tl_{}", locals.locals.len()), ValType::I32);

        out.push(Instruction::LocalSet(fval));

        // Check sign
        out.push(Instruction::LocalGet(fval));
        out.push(Instruction::F64Const(0.0));
        out.push(Instruction::F64Lt);
        out.push(Instruction::LocalSet(is_neg));

        // abs_val = abs(fval)
        out.push(Instruction::LocalGet(fval));
        out.push(Instruction::F64Abs);
        out.push(Instruction::LocalSet(abs_val));

        // int_part = i64(trunc(abs_val))
        out.push(Instruction::LocalGet(abs_val));
        out.push(Instruction::F64Floor);
        out.push(Instruction::I64TruncF64S);
        out.push(Instruction::LocalSet(int_part));

        // frac_val = abs_val - f64(int_part)
        out.push(Instruction::LocalGet(abs_val));
        out.push(Instruction::LocalGet(int_part));
        out.push(Instruction::F64ConvertI64S);
        out.push(Instruction::F64Sub);
        out.push(Instruction::LocalSet(frac_val));

        // Convert int_part to string using existing helper
        out.push(Instruction::LocalGet(int_part));
        self.emit_i64_to_string(out, locals)?;
        out.push(Instruction::LocalSet(int_len));
        out.push(Instruction::LocalSet(int_ptr));

        // Check if frac is zero (frac_val < 1e-9)
        out.push(Instruction::LocalGet(frac_val));
        out.push(Instruction::F64Const(1e-9));
        out.push(Instruction::F64Lt);
        out.push(Instruction::If(BlockType::Empty));
        {
            // No fractional part: build sign + int_str
            out.push(Instruction::LocalGet(is_neg));
            out.push(Instruction::LocalGet(int_len));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(total_len));
            out.push(Instruction::LocalGet(total_len));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(final_ptr));

            // If negative, write '-'
            out.push(Instruction::LocalGet(is_neg));
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::I32Const(45)); // '-'
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::End);

            // Copy int digits
            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::LocalGet(is_neg));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(int_ptr));
            out.push(Instruction::LocalGet(int_len));
            out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });
        }
        out.push(Instruction::Else);
        {
            // Has fractional part: always generate 6 fractional digits then strip trailing '0's
            // frac_int = i64(round(frac_val * 1e6))
            out.push(Instruction::LocalGet(frac_val));
            out.push(Instruction::F64Const(1e6));
            out.push(Instruction::F64Mul);
            out.push(Instruction::F64Const(0.5));
            out.push(Instruction::F64Add);
            out.push(Instruction::F64Floor);
            out.push(Instruction::I64TruncF64S);
            out.push(Instruction::LocalSet(frac_int));

            // Allocate 8-byte buffer and write exactly 6 digits (right-to-left)
            out.push(Instruction::I32Const(8));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(frac_buf));

            {
                let digit_idx = locals.add_local(&format!("__ftos_di_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::I32Const(5));
                out.push(Instruction::LocalSet(digit_idx));

                // Write 6 digits right-to-left
                out.push(Instruction::Block(BlockType::Empty));
                out.push(Instruction::Loop(BlockType::Empty));
                out.push(Instruction::LocalGet(digit_idx));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::I32LtS);
                out.push(Instruction::BrIf(1));

                out.push(Instruction::LocalGet(frac_buf));
                out.push(Instruction::LocalGet(digit_idx));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalGet(frac_int));
                out.push(Instruction::I64Const(10));
                out.push(Instruction::I64RemU);
                out.push(Instruction::I32WrapI64);
                out.push(Instruction::I32Const(48));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

                out.push(Instruction::LocalGet(frac_int));
                out.push(Instruction::I64Const(10));
                out.push(Instruction::I64DivU);
                out.push(Instruction::LocalSet(frac_int));

                out.push(Instruction::LocalGet(digit_idx));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Sub);
                out.push(Instruction::LocalSet(digit_idx));
                out.push(Instruction::Br(0));
                out.push(Instruction::End); // loop
                out.push(Instruction::End); // block
            }

            // frac_pos = 6 initially
            out.push(Instruction::I32Const(6));
            out.push(Instruction::LocalSet(frac_pos));

            // Strip trailing '0' characters from frac_buf
            out.push(Instruction::Block(BlockType::Empty));
            out.push(Instruction::Loop(BlockType::Empty));
            out.push(Instruction::LocalGet(frac_pos));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32LeS);
            out.push(Instruction::BrIf(1)); // keep at least 1 digit

            out.push(Instruction::LocalGet(frac_buf));
            out.push(Instruction::LocalGet(frac_pos));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Sub);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::I32Const(48)); // '0'
            out.push(Instruction::I32Ne);
            out.push(Instruction::BrIf(1)); // stop if not '0'

            out.push(Instruction::LocalGet(frac_pos));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Sub);
            out.push(Instruction::LocalSet(frac_pos));
            out.push(Instruction::Br(0));
            out.push(Instruction::End); // loop
            out.push(Instruction::End); // block

            // Build final: sign + int_str + '.' + frac_str
            // total = is_neg + int_len + 1 + frac_pos
            out.push(Instruction::LocalGet(is_neg));
            out.push(Instruction::LocalGet(int_len));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Const(1)); // for '.'
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(frac_pos));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(total_len));

            out.push(Instruction::LocalGet(total_len));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(final_ptr));

            let cursor = locals.add_local(&format!("__ftos_cur_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(cursor));

            // Write '-' if negative
            out.push(Instruction::LocalGet(is_neg));
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::I32Const(45)); // '-'
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::LocalGet(is_neg));
            out.push(Instruction::LocalSet(cursor));
            out.push(Instruction::End);

            // Copy integer digits
            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::LocalGet(cursor));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(int_ptr));
            out.push(Instruction::LocalGet(int_len));
            out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });
            out.push(Instruction::LocalGet(cursor));
            out.push(Instruction::LocalGet(int_len));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(cursor));

            // Write '.'
            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::LocalGet(cursor));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Const(46)); // '.'
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::LocalGet(cursor));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(cursor));

            // Copy fractional digits
            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::LocalGet(cursor));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(frac_buf));
            out.push(Instruction::LocalGet(frac_pos));
            out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });
        }
        out.push(Instruction::End); // end if frac == 0

        // Push result (ptr, len)
        out.push(Instruction::LocalGet(final_ptr));
        out.push(Instruction::LocalGet(total_len));

        Ok(())
    }

    fn emit_map_key_eq(
        &self,
        mti: &MapTypeInfo,
        entry_local: u32,
        key_local: u32,
        key_len_local: Option<u32>,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) {
        if mti.is_string_key {
            let kl = key_len_local.expect("key_len_local must be set for string-keyed maps");
            let eq_result = locals.add_local(&format!("__mkeq_{}", locals.locals.len()), ValType::I32);
            let cmp_idx = locals.add_local(&format!("__mkcidx_{}", locals.locals.len()), ValType::I32);

            // Load stored key len, compare with search key len
            out.push(Instruction::LocalGet(entry_local));
            out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalGet(kl));
            out.push(Instruction::I32Ne);
            out.push(Instruction::If(BlockType::Result(ValType::I32)));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::Else);
            {
                // Lengths equal, compare bytes
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(eq_result));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(cmp_idx));
                out.push(Instruction::Block(BlockType::Empty));
                out.push(Instruction::Loop(BlockType::Empty));
                out.push(Instruction::LocalGet(cmp_idx));
                out.push(Instruction::LocalGet(kl));
                out.push(Instruction::I32GeU);
                out.push(Instruction::BrIf(1));
                // byte at stored_ptr + idx
                out.push(Instruction::LocalGet(entry_local));
                out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalGet(cmp_idx));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                // byte at search_ptr + idx
                out.push(Instruction::LocalGet(key_local));
                out.push(Instruction::LocalGet(cmp_idx));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                out.push(Instruction::I32Ne);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(eq_result));
                out.push(Instruction::Br(2));
                out.push(Instruction::End);
                out.push(Instruction::LocalGet(cmp_idx));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(cmp_idx));
                out.push(Instruction::Br(0));
                out.push(Instruction::End); // loop
                out.push(Instruction::End); // block
                out.push(Instruction::LocalGet(eq_result));
            }
            out.push(Instruction::End); // if/else
        } else {
            out.push(Instruction::LocalGet(entry_local));
            let (_, key_align) = Self::elem_size_and_align(mti.key_vt);
            Self::emit_typed_load(mti.key_vt, 4, key_align, out);
            out.push(Instruction::LocalGet(key_local));
            match mti.key_vt {
                ValType::I64 => out.push(Instruction::I64Eq),
                ValType::F64 => out.push(Instruction::F64Eq),
                ValType::F32 => out.push(Instruction::F32Eq),
                _ => out.push(Instruction::I32Eq),
            }
        }
    }

    fn emit_map_key_store(
        mti: &MapTypeInfo,
        entry_local: u32,
        key_local: u32,
        key_len_local: Option<u32>,
        out: &mut Vec<Instruction<'static>>,
    ) {
        if mti.is_string_key {
            out.push(Instruction::LocalGet(entry_local));
            out.push(Instruction::LocalGet(key_local));
            out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalGet(entry_local));
            out.push(Instruction::LocalGet(key_len_local.expect("key_len_local must be set for string-keyed maps")));
            out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));
        } else {
            let (_, key_align) = Self::elem_size_and_align(mti.key_vt);
            out.push(Instruction::LocalGet(entry_local));
            out.push(Instruction::LocalGet(key_local));
            Self::emit_typed_store(mti.key_vt, 4, key_align, out);
        }
    }

    fn emit_map_val_store(
        mti: &MapTypeInfo,
        entry_local: u32,
        val_local: u32,
        val_len_local: Option<u32>,
        val_vt_actual: ValType,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        let val_offset = 4u64 + mti.key_size as u64;
        if mti.is_string_val {
            out.push(Instruction::LocalGet(entry_local));
            out.push(Instruction::LocalGet(val_local));
            out.push(Instruction::I32Store(MemArg { offset: val_offset, align: 2, memory_index: 0 }));
            if let Some(vl) = val_len_local {
                out.push(Instruction::LocalGet(entry_local));
                out.push(Instruction::LocalGet(vl));
                out.push(Instruction::I32Store(MemArg { offset: val_offset + 4, align: 2, memory_index: 0 }));
            }
        } else {
            let (_, val_align) = Self::elem_size_and_align(mti.val_vt);
            out.push(Instruction::LocalGet(entry_local));
            out.push(Instruction::LocalGet(val_local));
            Self::emit_typed_coerce(val_vt_actual, mti.val_vt, out)?;
            Self::emit_typed_store(mti.val_vt, val_offset, val_align, out);
        }
        Ok(())
    }

    fn compile_map_get(
        &mut self,
        map_name: &str,
        key_expr: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let mti = locals.map_types.get(map_name).ok_or_else(|| {
            Error::InternalError(format!("map type info not found for '{}'", map_name))
        })?.clone();
        let key_vt = mti.key_vt;
        let val_vt = mti.val_vt;
        let key_size = mti.key_size;
        let entry_size = Self::map_entry_size(key_size, mti.val_size) as i32;

        let map_local = locals.find(map_name).ok_or_else(|| {
            Error::InternalError(format!("map variable '{}' not found", map_name))
        })?;

        // Compile key
        self.compile_expression(key_expr, out, locals)?;
        let key_local;
        let key_len_local;
        if mti.is_string_key {
            let kl = locals.add_local(&format!("__mg_klen_{}", locals.locals.len()), ValType::I32);
            key_len_local = Some(kl);
            out.push(Instruction::LocalSet(kl));
            key_local = locals.add_local(&format!("__mg_kptr_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalSet(key_local));
        } else {
            let key_vt_actual = self.infer_val_type(key_expr, locals);
            Self::emit_typed_coerce(key_vt_actual, key_vt, out)?;
            key_local = locals.add_local(&format!("__mg_key_{}", locals.locals.len()), key_vt);
            out.push(Instruction::LocalSet(key_local));
            key_len_local = None;
        }

        // Load map header
        let cap_local = locals.add_local(&format!("__mg_cap_{}", locals.locals.len()), ValType::I32);
        let data_local = locals.add_local(&format!("__mg_data_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(cap_local));
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(data_local));

        let i_local = locals.add_local(&format!("__mg_i_{}", locals.locals.len()), ValType::I32);
        let entry_local = locals.add_local(&format!("__mg_e_{}", locals.locals.len()), ValType::I32);
        let found_local = locals.add_local(&format!("__mg_found_{}", locals.locals.len()), ValType::I32);
        let result_local = if mti.is_string_val {
            locals.add_local(&format!("__mg_res_{}", locals.locals.len()), ValType::I32)
        } else {
            locals.add_local(&format!("__mg_res_{}", locals.locals.len()), val_vt)
        };
        let result_len_local = if mti.is_string_val {
            Some(locals.add_local(&format!("__mg_reslen_{}", locals.locals.len()), ValType::I32))
        } else {
            None
        };

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(found_local));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(entry_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(entry_local));

        out.push(Instruction::LocalGet(entry_local));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Eq);
        out.push(Instruction::If(BlockType::Empty));
        {
            // Compare key
            self.emit_map_key_eq(&mti, entry_local, key_local, key_len_local, out, locals);
            out.push(Instruction::If(BlockType::Empty));
            {
                let val_offset = 4u64 + key_size as u64;
                if mti.is_string_val {
                    out.push(Instruction::LocalGet(entry_local));
                    out.push(Instruction::I32Load(MemArg { offset: val_offset, align: 2, memory_index: 0 }));
                    out.push(Instruction::LocalSet(result_local));
                    out.push(Instruction::LocalGet(entry_local));
                    out.push(Instruction::I32Load(MemArg { offset: val_offset + 4, align: 2, memory_index: 0 }));
                    out.push(Instruction::LocalSet(result_len_local.unwrap()));
                } else {
                    out.push(Instruction::LocalGet(entry_local));
                    let (_, val_align) = Self::elem_size_and_align(val_vt);
                    Self::emit_typed_load(val_vt, val_offset, val_align, out);
                    out.push(Instruction::LocalSet(result_local));
                }
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(found_local));
                out.push(Instruction::Br(3));
            }
            out.push(Instruction::End);
        }
        out.push(Instruction::End);

        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::Br(0));
        out.push(Instruction::End);
        out.push(Instruction::End);

        out.push(Instruction::LocalGet(result_local));
        if mti.is_string_val {
            out.push(Instruction::LocalGet(result_len_local.unwrap()));
        }
        Ok(())
    }

    fn compile_map_get_ok(
        &mut self,
        map_name: &str,
        key_expr: &ast::Expression,
        val_var: &str,
        ok_var: &str,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let mti = locals.map_types.get(map_name).ok_or_else(|| {
            Error::InternalError(format!("map type info not found for '{}'", map_name))
        })?.clone();
        let key_vt = mti.key_vt;
        let val_vt = mti.val_vt;
        let key_size = mti.key_size;
        let entry_size = Self::map_entry_size(key_size, mti.val_size) as i32;

        let map_local = locals.find(map_name).ok_or_else(|| {
            Error::InternalError(format!("map variable '{}' not found", map_name))
        })?;

        self.compile_expression(key_expr, out, locals)?;
        let key_local;
        let key_len_local;
        if mti.is_string_key {
            let kl = locals.add_local(&format!("__mgok_klen_{}", locals.locals.len()), ValType::I32);
            key_len_local = Some(kl);
            out.push(Instruction::LocalSet(kl));
            key_local = locals.add_local(&format!("__mgok_kptr_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalSet(key_local));
        } else {
            let key_vt_actual = self.infer_val_type(key_expr, locals);
            Self::emit_typed_coerce(key_vt_actual, key_vt, out)?;
            key_local = locals.add_local(&format!("__mgok_key_{}", locals.locals.len()), key_vt);
            out.push(Instruction::LocalSet(key_local));
            key_len_local = None;
        }

        let cap_local = locals.add_local(&format!("__mgok_cap_{}", locals.locals.len()), ValType::I32);
        let data_local = locals.add_local(&format!("__mgok_data_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(cap_local));
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(data_local));

        let i_local = locals.add_local(&format!("__mgok_i_{}", locals.locals.len()), ValType::I32);
        let entry_local = locals.add_local(&format!("__mgok_e_{}", locals.locals.len()), ValType::I32);

        let val_local = if mti.is_string_val {
            locals.add_local(val_var, ValType::I32)
        } else {
            locals.add_local(val_var, val_vt)
        };
        let val_len_local = if mti.is_string_val {
            let vl = locals.add_local(&format!("{}__str_len", val_var), ValType::I32);
            locals.set_var_struct_type(val_var, "__string");
            locals.string_locals.insert(val_var.to_string(), (val_local, vl));
            Some(vl)
        } else {
            None
        };
        let ok_local = locals.add_local(ok_var, ValType::I32);

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(ok_local));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(entry_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(entry_local));

        out.push(Instruction::LocalGet(entry_local));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Eq);
        out.push(Instruction::If(BlockType::Empty));
        {
            self.emit_map_key_eq(&mti, entry_local, key_local, key_len_local, out, locals);
            out.push(Instruction::If(BlockType::Empty));
            {
                let val_offset = 4u64 + key_size as u64;
                if mti.is_string_val {
                    out.push(Instruction::LocalGet(entry_local));
                    out.push(Instruction::I32Load(MemArg { offset: val_offset, align: 2, memory_index: 0 }));
                    out.push(Instruction::LocalSet(val_local));
                    out.push(Instruction::LocalGet(entry_local));
                    out.push(Instruction::I32Load(MemArg { offset: val_offset + 4, align: 2, memory_index: 0 }));
                    out.push(Instruction::LocalSet(val_len_local.unwrap()));
                } else {
                    out.push(Instruction::LocalGet(entry_local));
                    let (_, val_align) = Self::elem_size_and_align(val_vt);
                    Self::emit_typed_load(val_vt, val_offset, val_align, out);
                    out.push(Instruction::LocalSet(val_local));
                }
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(ok_local));
                out.push(Instruction::Br(3));
            }
            out.push(Instruction::End);
        }
        out.push(Instruction::End);

        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::Br(0));
        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block

        Ok(())
    }

    fn compile_map_set(
        &mut self,
        map_name: &str,
        key_expr: &ast::Expression,
        val_already_in_tmp: u32,
        val_len_tmp: Option<u32>,
        val_vt_actual: ValType,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let mti = locals.map_types.get(map_name).ok_or_else(|| {
            Error::InternalError(format!("map type info not found for '{}'", map_name))
        })?.clone();
        let key_size = mti.key_size;
        let entry_size = Self::map_entry_size(key_size, mti.val_size) as i32;

        let map_local = locals.find(map_name).ok_or_else(|| {
            Error::InternalError(format!("map variable '{}' not found", map_name))
        })?;

        // Nil map check: writing to a nil map panics
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        {
            let msg = b"assignment to entry in nil map";
            let msg_len = msg.len() as i32;
            out.push(Instruction::I32Const(msg_len));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            let msg_ptr = locals.add_local(
                &format!("__nil_map_ptr_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalSet(msg_ptr));
            for (i, &byte) in msg.iter().enumerate() {
                out.push(Instruction::LocalGet(msg_ptr));
                out.push(Instruction::I32Const(byte as i32));
                out.push(Instruction::I32Store8(MemArg {
                    offset: i as u64,
                    align: 0,
                    memory_index: 0,
                }));
            }
            out.push(Instruction::LocalGet(msg_ptr));
            out.push(Instruction::I32Const(msg_len));
            out.push(Instruction::Call(0)); // ctx_log
            out.push(Instruction::Unreachable);
        }
        out.push(Instruction::End);

        // Compile key
        self.compile_expression(key_expr, out, locals)?;
        let key_local;
        let key_len_local;
        if mti.is_string_key {
            let kl = locals.add_local(&format!("__ms_klen_{}", locals.locals.len()), ValType::I32);
            key_len_local = Some(kl);
            out.push(Instruction::LocalSet(kl));
            key_local = locals.add_local(&format!("__ms_kptr_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalSet(key_local));
        } else {
            let key_vt_e = self.infer_val_type(key_expr, locals);
            Self::emit_typed_coerce(key_vt_e, mti.key_vt, out)?;
            key_local = locals.add_local(&format!("__ms_key_{}", locals.locals.len()), mti.key_vt);
            out.push(Instruction::LocalSet(key_local));
            key_len_local = None;
        }

        // Load map header
        let cap_local = locals.add_local(&format!("__ms_cap_{}", locals.locals.len()), ValType::I32);
        let data_local = locals.add_local(&format!("__ms_data_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(cap_local));
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(data_local));

        let i_local = locals.add_local(&format!("__ms_i_{}", locals.locals.len()), ValType::I32);
        let entry_local = locals.add_local(&format!("__ms_e_{}", locals.locals.len()), ValType::I32);
        let done_local = locals.add_local(&format!("__ms_done_{}", locals.locals.len()), ValType::I32);

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(done_local));

        // Pass 1: look for existing key
        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(entry_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(entry_local));

        out.push(Instruction::LocalGet(entry_local));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Eq);
        out.push(Instruction::If(BlockType::Empty));
        {
            self.emit_map_key_eq(&mti, entry_local, key_local, key_len_local, out, locals);
            out.push(Instruction::If(BlockType::Empty));
            {
                Self::emit_map_val_store(&mti, entry_local, val_already_in_tmp, val_len_tmp, val_vt_actual, out)?;
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(done_local));
                out.push(Instruction::Br(3));
            }
            out.push(Instruction::End);
        }
        out.push(Instruction::End);

        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::Br(0));
        out.push(Instruction::End);
        out.push(Instruction::End);

        // If not found, insert into first empty slot
        out.push(Instruction::LocalGet(done_local));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        {
            // Resize if load factor >= 75%
            out.push(Instruction::LocalGet(map_local));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::I32Const(4));
            out.push(Instruction::I32Mul);
            out.push(Instruction::LocalGet(cap_local));
            out.push(Instruction::I32Const(3));
            out.push(Instruction::I32Mul);
            out.push(Instruction::I32GeU);
            out.push(Instruction::If(BlockType::Empty));
            {
                let new_cap_l = locals.add_local(&format!("__ms_ncap_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalGet(cap_local));
                out.push(Instruction::I32Const(2));
                out.push(Instruction::I32Mul);
                out.push(Instruction::LocalSet(new_cap_l));

                let new_dsz = locals.add_local(&format!("__ms_ndsz_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalGet(new_cap_l));
                out.push(Instruction::I32Const(entry_size));
                out.push(Instruction::I32Mul);
                out.push(Instruction::LocalTee(new_dsz));

                out.push(Instruction::Call(self.alloc_func_idx()?));
                let new_data_l = locals.add_local(&format!("__ms_nd_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalTee(new_data_l));

                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalGet(new_dsz));
                out.push(Instruction::MemoryFill(0));

                out.push(Instruction::LocalGet(new_data_l));
                out.push(Instruction::LocalGet(data_local));
                out.push(Instruction::LocalGet(cap_local));
                out.push(Instruction::I32Const(entry_size));
                out.push(Instruction::I32Mul);
                out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

                out.push(Instruction::LocalGet(map_local));
                out.push(Instruction::LocalGet(new_cap_l));
                out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalGet(map_local));
                out.push(Instruction::LocalGet(new_data_l));
                out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

                out.push(Instruction::LocalGet(new_cap_l));
                out.push(Instruction::LocalSet(cap_local));
                out.push(Instruction::LocalGet(new_data_l));
                out.push(Instruction::LocalSet(data_local));
            }
            out.push(Instruction::End);

            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(i_local));

            out.push(Instruction::Block(BlockType::Empty));
            out.push(Instruction::Loop(BlockType::Empty));
            out.push(Instruction::LocalGet(i_local));
            out.push(Instruction::LocalGet(cap_local));
            out.push(Instruction::I32GeU);
            out.push(Instruction::BrIf(1));

            out.push(Instruction::LocalGet(data_local));
            out.push(Instruction::LocalGet(i_local));
            out.push(Instruction::I32Const(entry_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(entry_local));

            out.push(Instruction::LocalGet(entry_local));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Ne);
            out.push(Instruction::If(BlockType::Empty));
            {
                out.push(Instruction::LocalGet(entry_local));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                Self::emit_map_key_store(&mti, entry_local, key_local, key_len_local, out);
                Self::emit_map_val_store(&mti, entry_local, val_already_in_tmp, val_len_tmp, val_vt_actual, out)?;
                // Increment count
                out.push(Instruction::LocalGet(map_local));
                out.push(Instruction::LocalGet(map_local));
                out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::Br(3));
            }
            out.push(Instruction::End);

            out.push(Instruction::LocalGet(i_local));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(i_local));
            out.push(Instruction::Br(0));
            out.push(Instruction::End);
            out.push(Instruction::End);
        }
        out.push(Instruction::End);

        Ok(())
    }

    fn compile_map_delete(
        &mut self,
        map_name: &str,
        key_expr: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let mti = locals.map_types.get(map_name).ok_or_else(|| {
            Error::InternalError(format!("map type info not found for '{}'", map_name))
        })?.clone();
        let entry_size = Self::map_entry_size(mti.key_size, mti.val_size) as i32;

        let map_local = locals.find(map_name).ok_or_else(|| {
            Error::InternalError(format!("map variable '{}' not found", map_name))
        })?;

        self.compile_expression(key_expr, out, locals)?;
        let key_local;
        let key_len_local;
        if mti.is_string_key {
            let kl = locals.add_local(&format!("__md_klen_{}", locals.locals.len()), ValType::I32);
            key_len_local = Some(kl);
            out.push(Instruction::LocalSet(kl));
            key_local = locals.add_local(&format!("__md_kptr_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalSet(key_local));
        } else {
            let key_vt_e = self.infer_val_type(key_expr, locals);
            Self::emit_typed_coerce(key_vt_e, mti.key_vt, out)?;
            key_local = locals.add_local(&format!("__md_key_{}", locals.locals.len()), mti.key_vt);
            out.push(Instruction::LocalSet(key_local));
            key_len_local = None;
        }

        let cap_local = locals.add_local(&format!("__md_cap_{}", locals.locals.len()), ValType::I32);
        let data_local = locals.add_local(&format!("__md_data_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(cap_local));
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(data_local));

        let i_local = locals.add_local(&format!("__md_i_{}", locals.locals.len()), ValType::I32);
        let entry_local = locals.add_local(&format!("__md_e_{}", locals.locals.len()), ValType::I32);

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(i_local));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(entry_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(entry_local));

        out.push(Instruction::LocalGet(entry_local));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Eq);
        out.push(Instruction::If(BlockType::Empty));
        {
            self.emit_map_key_eq(&mti, entry_local, key_local, key_len_local, out, locals);
            out.push(Instruction::If(BlockType::Empty));
            {
                out.push(Instruction::LocalGet(entry_local));
                out.push(Instruction::I32Const(2));
                out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalGet(map_local));
                out.push(Instruction::LocalGet(map_local));
                out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Sub);
                out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::Br(3));
            }
            out.push(Instruction::End);
        }
        out.push(Instruction::End);

        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::Br(0));
        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block

        Ok(())
    }

    fn compile_index(
        &mut self,
        idx: &ast::Index,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let left = idx.left.as_deref().ok_or_else(|| {
            Error::InternalError("index expression missing left operand".to_string())
        })?;

        // Map indexing
        if let ast::Expression::Ident(ident) = left {
            if locals.get_var_struct_type(&ident.name) == Some("__map") {
                return self.compile_map_get(&ident.name.clone(), &idx.index, out, locals);
            }
        }

        // Chained map indexing: m[k1][k2] where m is map[K]map[K2]V2
        if let ast::Expression::Index(outer_idx) = left {
            if let Some(ast::Expression::Ident(outer_ident)) = outer_idx.left.as_deref() {
                if locals.get_var_struct_type(&outer_ident.name) == Some("__map") {
                    if let Some(inner_mti) = locals.map_types.get(&outer_ident.name)
                        .and_then(|mti| mti.nested_map_val_type.clone())
                    {
                        self.compile_map_get(&outer_ident.name.clone(), &outer_idx.index, out, locals)?;
                        let tmp_name = format!("__chained_map_{}", locals.locals.len());
                        let tmp_local = locals.add_local(&tmp_name, ValType::I32);
                        out.push(Instruction::LocalSet(tmp_local));
                        locals.set_var_struct_type(&tmp_name, "__map");
                        locals.map_types.insert(tmp_name.clone(), *inner_mti);
                        return self.compile_map_get(&tmp_name, &idx.index, out, locals);
                    }
                }
            }
        }

        // String indexing: s[i] returns a byte (i32)
        if self.is_string_expr(left, locals) {
            self.compile_expression(left, out, locals)?;
            let str_len = locals.add_local(&format!("__sidx_len_{}", locals.locals.len()), ValType::I32);
            let str_ptr = locals.add_local(&format!("__sidx_ptr_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalSet(str_len));
            out.push(Instruction::LocalSet(str_ptr));

            self.compile_expression(&idx.index, out, locals)?;
            let idx_vt = self.infer_val_type(&idx.index, locals);
            if idx_vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }
            let idx_local = locals.add_local(&format!("__sidx_i_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalTee(idx_local));

            // Bounds check: if idx >= len, trap
            out.push(Instruction::LocalGet(str_len));
            out.push(Instruction::I32GeU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);

            out.push(Instruction::LocalGet(str_ptr));
            out.push(Instruction::LocalGet(idx_local));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load8U(MemArg {
                offset: 0,
                align: 0,
                memory_index: 0,
            }));
            return Ok(());
        }

        // Multi-dimensional array indexing: a[i][j] where a is [M][N]T
        if let ast::Expression::Index(outer_idx) = left {
            if let Some(ast::Expression::Ident(outer_ident)) = outer_idx.left.as_deref() {
                if let Some(&(inner_elem_vt, inner_len)) = locals.nested_array_inner_info.get(&outer_ident.name) {
                    if let Some(&(_, outer_len)) = locals.array_info.get(&outer_ident.name) {
                        let (inner_elem_size, inner_align) = Self::elem_size_and_align(inner_elem_vt);
                        let inner_array_bytes = inner_len as i32 * inner_elem_size;

                        self.compile_expression(&ast::Expression::Ident(outer_ident.clone()), out, locals)?;
                        let base = locals.add_local(&format!("__mdarr_b_{}", locals.locals.len()), ValType::I32);
                        out.push(Instruction::LocalSet(base));

                        // Compile outer index
                        self.compile_expression(&outer_idx.index, out, locals)?;
                        let oidx_vt = self.infer_val_type(&outer_idx.index, locals);
                        if oidx_vt == ValType::I64 { out.push(Instruction::I32WrapI64); }
                        let oidx_local = locals.add_local(&format!("__mdarr_oi_{}", locals.locals.len()), ValType::I32);
                        out.push(Instruction::LocalTee(oidx_local));

                        // Bounds check outer
                        out.push(Instruction::I32Const(outer_len as i32));
                        out.push(Instruction::I32GeU);
                        out.push(Instruction::If(BlockType::Empty));
                        out.push(Instruction::Unreachable);
                        out.push(Instruction::End);

                        // Compile inner index
                        self.compile_expression(&idx.index, out, locals)?;
                        let iidx_vt = self.infer_val_type(&idx.index, locals);
                        if iidx_vt == ValType::I64 { out.push(Instruction::I32WrapI64); }
                        let iidx_local = locals.add_local(&format!("__mdarr_ii_{}", locals.locals.len()), ValType::I32);
                        out.push(Instruction::LocalTee(iidx_local));

                        // Bounds check inner
                        out.push(Instruction::I32Const(inner_len as i32));
                        out.push(Instruction::I32GeU);
                        out.push(Instruction::If(BlockType::Empty));
                        out.push(Instruction::Unreachable);
                        out.push(Instruction::End);

                        // addr = base + outer_idx * inner_array_bytes + inner_idx * elem_size
                        out.push(Instruction::LocalGet(base));
                        out.push(Instruction::LocalGet(oidx_local));
                        out.push(Instruction::I32Const(inner_array_bytes));
                        out.push(Instruction::I32Mul);
                        out.push(Instruction::I32Add);
                        out.push(Instruction::LocalGet(iidx_local));
                        out.push(Instruction::I32Const(inner_elem_size));
                        out.push(Instruction::I32Mul);
                        out.push(Instruction::I32Add);

                        Self::emit_typed_load(inner_elem_vt, 0, inner_align, out);
                        return Ok(());
                    }
                }
            }
        }

        // Array indexing: a[i] with bounds check
        if let ast::Expression::Ident(ident) = left {
            if let Some(&(arr_elem_vt, arr_len)) = locals.array_info.get(&ident.name) {
                let (arr_elem_size, arr_align) = Self::elem_size_and_align(arr_elem_vt);
                self.compile_expression(left, out, locals)?;
                let base = locals.add_local(&format!("__arri_b_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalSet(base));

                self.compile_expression(&idx.index, out, locals)?;
                let idx_vt = self.infer_val_type(&idx.index, locals);
                if idx_vt == ValType::I64 {
                    out.push(Instruction::I32WrapI64);
                }
                let idx_local = locals.add_local(&format!("__arri_i_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalTee(idx_local));

                out.push(Instruction::I32Const(arr_len as i32));
                out.push(Instruction::I32GeU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::Unreachable);
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(base));
                out.push(Instruction::LocalGet(idx_local));
                out.push(Instruction::I32Const(arr_elem_size));
                out.push(Instruction::I32Mul);
                out.push(Instruction::I32Add);

                match arr_elem_vt {
                    ValType::I32 => out.push(Instruction::I32Load(MemArg { offset: 0, align: arr_align, memory_index: 0 })),
                    ValType::F32 => out.push(Instruction::F32Load(MemArg { offset: 0, align: arr_align, memory_index: 0 })),
                    ValType::F64 => out.push(Instruction::F64Load(MemArg { offset: 0, align: arr_align, memory_index: 0 })),
                    _ => out.push(Instruction::I64Load(MemArg { offset: 0, align: arr_align, memory_index: 0 })),
                }
                return Ok(());
            }
        }

        // Handle nested slice indexing: matrix[i][j] where matrix is [][]T
        if let ast::Expression::Index(outer_idx) = left {
            if let Some(outer_left) = outer_idx.left.as_deref() {
                if let ast::Expression::Ident(outer_ident) = outer_left {
                    if locals.get_var_struct_type(&outer_ident.name) == Some("__slice") {
                        let outer_elem_vt = locals.slice_elem_types
                            .get(&outer_ident.name).copied().unwrap_or(ValType::I64);
                        if outer_elem_vt == ValType::I32 {
                            self.compile_expression(left, out, locals)?;
                            let inner_hdr = locals.add_local(&format!("__nslix_hdr_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalTee(inner_hdr));
                            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                            let inner_data = locals.add_local(&format!("__nslix_dp_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalSet(inner_data));

                            let inner_len = locals.add_local(&format!("__nslix_len_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalGet(inner_hdr));
                            out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalSet(inner_len));

                            self.compile_expression(&idx.index, out, locals)?;
                            let idx_vt = self.infer_val_type(&idx.index, locals);
                            if idx_vt == ValType::I64 {
                                out.push(Instruction::I32WrapI64);
                            }
                            let idx_local = locals.add_local(&format!("__nslix_i_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalTee(idx_local));

                            out.push(Instruction::LocalGet(inner_len));
                            out.push(Instruction::I32GeU);
                            out.push(Instruction::If(BlockType::Empty));
                            out.push(Instruction::Unreachable);
                            out.push(Instruction::End);

                            let inner_elem_vt = locals.nested_slice_inner_elem_types
                                .get(&outer_ident.name).copied().unwrap_or(ValType::I64);
                            let (inner_elem_size, inner_align) = Self::elem_size_and_align(inner_elem_vt);
                            out.push(Instruction::LocalGet(inner_data));
                            out.push(Instruction::LocalGet(idx_local));
                            out.push(Instruction::I32Const(inner_elem_size));
                            out.push(Instruction::I32Mul);
                            out.push(Instruction::I32Add);
                            Self::emit_typed_load(inner_elem_vt, 0, inner_align, out);
                            return Ok(());
                        }
                    }
                }
            }
        }

        let is_slice_header = if let ast::Expression::Ident(ident) = left {
            locals.get_var_struct_type(&ident.name) == Some("__slice")
        } else {
            false
        };

        let elem_vt = if let ast::Expression::Ident(ident) = left {
            locals
                .slice_elem_types
                .get(&ident.name)
                .copied()
                .unwrap_or(ValType::I64)
        } else {
            ValType::I64
        };

        let (elem_size, align) = match elem_vt {
            ValType::I32 | ValType::F32 => (4i32, 2u32),
            _ => (8i32, 3u32),
        };

        if is_slice_header {
            self.compile_expression(left, out, locals)?;
            let hdr = locals.add_local(&format!("__slix_hdr_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalTee(hdr));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            let data_local = locals.add_local(&format!("__slix_dp_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalSet(data_local));

            let slice_len = locals.add_local(&format!("__slix_len_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalGet(hdr));
            out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(slice_len));

            self.compile_expression(&idx.index, out, locals)?;
            let idx_vt = self.infer_val_type(&idx.index, locals);
            if idx_vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }
            let idx_local = locals.add_local(&format!("__slix_i_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalTee(idx_local));

            out.push(Instruction::LocalGet(slice_len));
            out.push(Instruction::I32GeU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);

            out.push(Instruction::LocalGet(data_local));
            out.push(Instruction::LocalGet(idx_local));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::I32Add);
        } else {
            self.compile_expression(left, out, locals)?;
            self.compile_expression(&idx.index, out, locals)?;

            let idx_vt = self.infer_val_type(&idx.index, locals);
            if idx_vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }

            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::I32Add);
        }

        match elem_vt {
            ValType::I32 => out.push(Instruction::I32Load(MemArg {
                offset: 0,
                align,
                memory_index: 0,
            })),
            ValType::F32 => out.push(Instruction::F32Load(MemArg {
                offset: 0,
                align,
                memory_index: 0,
            })),
            ValType::F64 => out.push(Instruction::F64Load(MemArg {
                offset: 0,
                align,
                memory_index: 0,
            })),
            _ => out.push(Instruction::I64Load(MemArg {
                offset: 0,
                align,
                memory_index: 0,
            })),
        }
        Ok(())
    }

    fn compile_index_store_addr(
        &mut self,
        idx: &ast::Index,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(ValType, u32), Error> {
        let left = idx.left.as_deref().ok_or_else(|| {
            Error::InternalError("index expression missing left operand".to_string())
        })?;

        // Multi-dimensional array element store: a[i][j] = val
        if let ast::Expression::Index(outer_idx) = left {
            if let Some(ast::Expression::Ident(outer_ident)) = outer_idx.left.as_deref() {
                if let Some(&(inner_elem_vt, inner_len)) = locals.nested_array_inner_info.get(&outer_ident.name) {
                    if let Some(&(_, outer_len)) = locals.array_info.get(&outer_ident.name) {
                        let (inner_elem_size, inner_align) = Self::elem_size_and_align(inner_elem_vt);
                        let inner_array_bytes = inner_len as i32 * inner_elem_size;

                        self.compile_expression(&ast::Expression::Ident(outer_ident.clone()), out, locals)?;
                        let base = locals.add_local(&format!("__mdarrs_b_{}", locals.locals.len()), ValType::I32);
                        out.push(Instruction::LocalSet(base));

                        self.compile_expression(&outer_idx.index, out, locals)?;
                        let oidx_vt = self.infer_val_type(&outer_idx.index, locals);
                        if oidx_vt == ValType::I64 { out.push(Instruction::I32WrapI64); }
                        let oidx_local = locals.add_local(&format!("__mdarrs_oi_{}", locals.locals.len()), ValType::I32);
                        out.push(Instruction::LocalTee(oidx_local));

                        out.push(Instruction::I32Const(outer_len as i32));
                        out.push(Instruction::I32GeU);
                        out.push(Instruction::If(BlockType::Empty));
                        out.push(Instruction::Unreachable);
                        out.push(Instruction::End);

                        self.compile_expression(&idx.index, out, locals)?;
                        let iidx_vt = self.infer_val_type(&idx.index, locals);
                        if iidx_vt == ValType::I64 { out.push(Instruction::I32WrapI64); }
                        let iidx_local = locals.add_local(&format!("__mdarrs_ii_{}", locals.locals.len()), ValType::I32);
                        out.push(Instruction::LocalTee(iidx_local));

                        out.push(Instruction::I32Const(inner_len as i32));
                        out.push(Instruction::I32GeU);
                        out.push(Instruction::If(BlockType::Empty));
                        out.push(Instruction::Unreachable);
                        out.push(Instruction::End);

                        out.push(Instruction::LocalGet(base));
                        out.push(Instruction::LocalGet(oidx_local));
                        out.push(Instruction::I32Const(inner_array_bytes));
                        out.push(Instruction::I32Mul);
                        out.push(Instruction::I32Add);
                        out.push(Instruction::LocalGet(iidx_local));
                        out.push(Instruction::I32Const(inner_elem_size));
                        out.push(Instruction::I32Mul);
                        out.push(Instruction::I32Add);
                        return Ok((inner_elem_vt, inner_align));
                    }
                }
            }
        }

        // Array element store
        if let ast::Expression::Ident(ident) = left {
            if let Some(&(arr_elem_vt, arr_len)) = locals.array_info.get(&ident.name) {
                let (arr_elem_size, arr_align) = Self::elem_size_and_align(arr_elem_vt);
                self.compile_expression(left, out, locals)?;
                let base = locals.add_local(&format!("__arrs_b_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalSet(base));

                self.compile_expression(&idx.index, out, locals)?;
                let idx_vt = self.infer_val_type(&idx.index, locals);
                if idx_vt == ValType::I64 {
                    out.push(Instruction::I32WrapI64);
                }
                let idx_local = locals.add_local(&format!("__arrs_i_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalTee(idx_local));

                out.push(Instruction::I32Const(arr_len as i32));
                out.push(Instruction::I32GeU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::Unreachable);
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(base));
                out.push(Instruction::LocalGet(idx_local));
                out.push(Instruction::I32Const(arr_elem_size));
                out.push(Instruction::I32Mul);
                out.push(Instruction::I32Add);
                return Ok((arr_elem_vt, arr_align));
            }
        }

        let is_slice_header = if let ast::Expression::Ident(ident) = left {
            locals.get_var_struct_type(&ident.name) == Some("__slice")
        } else {
            false
        };

        let elem_vt = if let ast::Expression::Ident(ident) = left {
            locals
                .slice_elem_types
                .get(&ident.name)
                .copied()
                .unwrap_or(ValType::I64)
        } else {
            ValType::I64
        };

        let (elem_size, align) = Self::elem_size_and_align(elem_vt);

        if is_slice_header {
            self.compile_expression(left, out, locals)?;
            let hdr = locals.add_local(&format!("__slis_hdr_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalTee(hdr));
            out.push(Instruction::I32Load(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
            let data_local = locals.add_local(&format!("__slis_dp_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalSet(data_local));

            let slice_len = locals.add_local(&format!("__slis_len_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalGet(hdr));
            out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(slice_len));

            self.compile_expression(&idx.index, out, locals)?;
            let idx_vt = self.infer_val_type(&idx.index, locals);
            if idx_vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }
            let idx_local = locals.add_local(&format!("__slis_i_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalTee(idx_local));

            out.push(Instruction::LocalGet(slice_len));
            out.push(Instruction::I32GeU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);

            out.push(Instruction::LocalGet(data_local));
            out.push(Instruction::LocalGet(idx_local));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::I32Add);
        } else {
            self.compile_expression(left, out, locals)?;

            self.compile_expression(&idx.index, out, locals)?;

            let idx_vt = self.infer_val_type(&idx.index, locals);
            if idx_vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }

            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::I32Add);
        }

        Ok((elem_vt, align))
    }

    fn compile_selector_store_addr(
        &mut self,
        sel: &ast::Selector,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(u64, ValType), Error> {
        self.compile_expression(&sel.x, out, locals)?;

        let struct_type_name = self.infer_struct_type_from_expr(sel.x.as_ref(), locals);

        if let Some(type_name) = struct_type_name {
            if let Some(struct_def) = self.struct_defs.get(&type_name) {
                if let Some(field) = struct_def.find_field(&sel.sel.name) {
                    let offset = field.offset as u64;
                    let vt = field.wasm_type.to_val_type();
                    return Ok((offset, vt));
                }
            }
        }

        let sel_name = if let ast::Expression::Ident(ident) = sel.x.as_ref() {
            format!("{}.{}", ident.name, sel.sel.name)
        } else {
            format!("<expr>.{}", sel.sel.name)
        };
        Err(Error::InternalError(format!(
            "unresolved selector for store: {}",
            sel_name
        )))
    }

    fn compile_embedded_lit_inline(
        &mut self,
        lit: &ast::LiteralValue,
        ptr_local: u32,
        base_offset: u64,
        struct_def: Option<&StructDef>,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        for (j, inner_kv) in lit.values.iter().enumerate() {
            match &inner_kv.val {
                ast::Element::LitValue(sub_lit) => {
                    let sub_embed_info = if let Some(ref ikey) = inner_kv.key {
                        if let ast::Element::Expr(ast::Expression::Ident(kid)) = ikey {
                            struct_def.and_then(|sd|
                                sd.embedded_types.iter()
                                    .find(|(name, _)| name == &kid.name)
                                    .map(|(name, off)| (name.clone(), *off))
                            )
                        } else { None }
                    } else {
                        struct_def.and_then(|sd| {
                            if j < sd.fields.len() {
                                sd.embedded_types.iter()
                                    .find(|(name, _)| name == &sd.fields[j].name)
                                    .map(|(name, off)| (name.clone(), *off))
                            } else { None }
                        })
                    };

                    if let Some((sub_type_name, sub_offset)) = sub_embed_info {
                        let sub_def = self.struct_defs.get(&sub_type_name).cloned();
                        self.compile_embedded_lit_inline(
                            sub_lit, ptr_local, base_offset + sub_offset as u64,
                            sub_def.as_ref(), out, locals,
                        )?;
                    } else {
                        let inner_field_name = if let Some(ref ikey) = inner_kv.key {
                            if let ast::Element::Expr(ast::Expression::Ident(kid)) = ikey {
                                Some(kid.name.clone())
                            } else { None }
                        } else { None };

                        let sub_type = inner_field_name.and_then(|n| {
                            struct_def.and_then(|sd| sd.find_field(&n))
                                .and_then(|f| f.go_type_tag.as_ref())
                                .and_then(|tag| self.struct_defs.get(tag))
                                .cloned()
                        });

                        let sub_off = if let Some(ref ikey) = inner_kv.key {
                            if let ast::Element::Expr(ast::Expression::Ident(kid)) = ikey {
                                struct_def.and_then(|sd| sd.find_field(&kid.name))
                                    .map(|f| f.offset as u64)
                                    .unwrap_or(j as u64 * 8)
                            } else { j as u64 * 8 }
                        } else {
                            struct_def.map(|sd| {
                                if j < sd.fields.len() { sd.fields[j].offset as u64 }
                                else { j as u64 * 8 }
                            }).unwrap_or(j as u64 * 8)
                        };

                        self.compile_embedded_lit_inline(
                            sub_lit, ptr_local, base_offset + sub_off,
                            sub_type.as_ref(), out, locals,
                        )?;
                    }
                }
                ast::Element::Expr(inner_expr) => {
                    // Handle CompositeLit for embedded sub-structs
                    if let ast::Expression::CompositeLit(inner_comp) = inner_expr {
                        let sub_embed_info = if let Some(ref ikey) = inner_kv.key {
                            if let ast::Element::Expr(ast::Expression::Ident(kid)) = ikey {
                                struct_def.and_then(|sd|
                                    sd.embedded_types.iter()
                                        .find(|(name, _)| name == &kid.name)
                                        .map(|(name, off)| (name.clone(), *off))
                                )
                            } else { None }
                        } else {
                            struct_def.and_then(|sd| {
                                if j < sd.fields.len() {
                                    sd.embedded_types.iter()
                                        .find(|(name, _)| name == &sd.fields[j].name)
                                        .map(|(name, off)| (name.clone(), *off))
                                } else { None }
                            })
                        };

                        if let Some((sub_type_name, sub_offset)) = sub_embed_info {
                            let sub_def = self.struct_defs.get(&sub_type_name).cloned();
                            self.compile_embedded_lit_inline(
                                &inner_comp.val, ptr_local, base_offset + sub_offset as u64,
                                sub_def.as_ref(), out, locals,
                            )?;
                            continue;
                        }
                    }

                    let (inner_off, fwt): (u64, Option<WasmType>) = if let Some(ref ikey) = inner_kv.key {
                        if let ast::Element::Expr(ast::Expression::Ident(kid)) = ikey {
                            if let Some(sd) = struct_def {
                                if let Some(f) = sd.find_field(&kid.name) {
                                    (f.offset as u64, Some(f.wasm_type))
                                } else { (j as u64 * 8, None) }
                            } else { (j as u64 * 8, None) }
                        } else { (j as u64 * 8, None) }
                    } else if let Some(sd) = struct_def {
                        if j < sd.fields.len() {
                            (sd.fields[j].offset as u64, Some(sd.fields[j].wasm_type))
                        } else { (j as u64 * 8, None) }
                    } else { (j as u64 * 8, None) };

                    let abs_off = base_offset + inner_off;
                    out.push(Instruction::LocalGet(ptr_local));
                    self.compile_expression(inner_expr, out, locals)?;
                    let vt = fwt.map(|wt: WasmType| wt.to_val_type())
                        .unwrap_or_else(|| self.infer_val_type(inner_expr, locals));

                    let target_vt = fwt.map(|wt: WasmType| wt.to_val_type()).unwrap_or(vt);
                    if vt != target_vt {
                        Self::emit_typed_coerce(vt, target_vt, out)?;
                    }

                    Self::emit_typed_store(target_vt, abs_off, if target_vt == ValType::I64 || target_vt == ValType::F64 { 3 } else { 2 }, out);
                }
            }
        }
        Ok(())
    }

    fn emit_typed_store(vt: ValType, offset: u64, align: u32, out: &mut Vec<Instruction<'static>>) {
        match vt {
            ValType::I32 => out.push(Instruction::I32Store(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
            ValType::F32 => out.push(Instruction::F32Store(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
            ValType::F64 => out.push(Instruction::F64Store(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
            _ => out.push(Instruction::I64Store(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
        }
    }

    fn emit_typed_load(vt: ValType, offset: u64, align: u32, out: &mut Vec<Instruction<'static>>) {
        match vt {
            ValType::I32 => out.push(Instruction::I32Load(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
            ValType::F32 => out.push(Instruction::F32Load(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
            ValType::F64 => out.push(Instruction::F64Load(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
            _ => out.push(Instruction::I64Load(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
        }
    }

    fn reject_unsupported_type(typ: &ast::Expression) -> Result<(), Error> {
        match typ {
            ast::Expression::TypeChannel(_) => Err(Error::InternalError(
                "channels are not supported in WASM UDFs".to_string(),
            )),
            _ => Ok(()),
        }
    }

    fn emit_typed_coerce(from: ValType, to: ValType, out: &mut Vec<Instruction<'static>>) -> Result<(), Error> {
        if from == to {
            return Ok(());
        }
        match (from, to) {
            (ValType::I64, ValType::I32) => out.push(Instruction::I32WrapI64),
            (ValType::I32, ValType::I64) => out.push(Instruction::I64ExtendI32S),
            (ValType::F64, ValType::F32) => out.push(Instruction::F32DemoteF64),
            (ValType::F32, ValType::F64) => out.push(Instruction::F64PromoteF32),
            (ValType::I64, ValType::F64) => out.push(Instruction::F64ConvertI64S),
            (ValType::I32, ValType::F64) => out.push(Instruction::F64ConvertI32S),
            (ValType::I64, ValType::F32) => out.push(Instruction::F32ConvertI64S),
            (ValType::I32, ValType::F32) => out.push(Instruction::F32ConvertI32S),
            (ValType::F64, ValType::I64) => out.push(Instruction::I64TruncF64S),
            (ValType::F64, ValType::I32) => out.push(Instruction::I32TruncF64S),
            (ValType::F32, ValType::I64) => out.push(Instruction::I64TruncF32S),
            (ValType::F32, ValType::I32) => out.push(Instruction::I32TruncF32S),
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported type coercion from {:?} to {:?}",
                    from, to
                )));
            }
        }
        Ok(())
    }

    fn emit_compound_op(
        &self,
        op: &Operator,
        vt: ValType,
        is_unsigned: bool,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        match op {
            Operator::AddAssign => out.push(Self::typed_add(vt)),
            Operator::SubAssign => out.push(Self::typed_sub(vt)),
            Operator::MulAssign => out.push(Self::typed_mul(vt)),
            Operator::QuoAssign => out.push(Self::typed_div(vt, is_unsigned)),
            Operator::RemAssign
            | Operator::AndAssign
            | Operator::OrAssign
            | Operator::XorAssign
            | Operator::ShlAssign
            | Operator::ShrAssign
            | Operator::AndNotAssign
                if matches!(vt, ValType::F32 | ValType::F64) =>
            {
                return Err(Error::InternalError(format!(
                    "operator {:?} is not valid on floating-point type {:?}",
                    op, vt
                )));
            }
            Operator::RemAssign => match vt {
                ValType::I32 => out.push(if is_unsigned { Instruction::I32RemU } else { Instruction::I32RemS }),
                _ => out.push(if is_unsigned { Instruction::I64RemU } else { Instruction::I64RemS }),
            },
            Operator::AndAssign => match vt {
                ValType::I32 => out.push(Instruction::I32And),
                _ => out.push(Instruction::I64And),
            },
            Operator::OrAssign => match vt {
                ValType::I32 => out.push(Instruction::I32Or),
                _ => out.push(Instruction::I64Or),
            },
            Operator::XorAssign => match vt {
                ValType::I32 => out.push(Instruction::I32Xor),
                _ => out.push(Instruction::I64Xor),
            },
            Operator::ShlAssign => match vt {
                ValType::I32 => out.push(Instruction::I32Shl),
                _ => out.push(Instruction::I64Shl),
            },
            Operator::ShrAssign => match vt {
                ValType::I32 => out.push(if is_unsigned { Instruction::I32ShrU } else { Instruction::I32ShrS }),
                _ => out.push(if is_unsigned { Instruction::I64ShrU } else { Instruction::I64ShrS }),
            },
            Operator::AndNotAssign => match vt {
                ValType::I32 => {
                    out.push(Instruction::I32Const(-1));
                    out.push(Instruction::I32Xor);
                    out.push(Instruction::I32And);
                }
                _ => {
                    out.push(Instruction::I64Const(-1));
                    out.push(Instruction::I64Xor);
                    out.push(Instruction::I64And);
                }
            },
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported compound operator: {:?}",
                    op
                )));
            }
        }
        Ok(())
    }

    fn emit_deferred_calls(&self, out: &mut Vec<Instruction<'static>>) {
        if let Some(scope) = self.deferred_calls.last() {
            for call in scope.iter().rev() {
                // Write current named return locals into the closure environment
                // so the deferred closure sees the values set by the return statement.
                for nrc in &call.named_return_captures {
                    out.push(Instruction::LocalGet(nrc.env_local));
                    out.push(Instruction::LocalGet(nrc.named_return_local));
                    match nrc.val_type {
                        ValType::I64 => out.push(Instruction::I64Store(MemArg {
                            offset: nrc.env_offset as u64, align: 3, memory_index: 0,
                        })),
                        ValType::F64 => out.push(Instruction::F64Store(MemArg {
                            offset: nrc.env_offset as u64, align: 3, memory_index: 0,
                        })),
                        ValType::F32 => out.push(Instruction::F32Store(MemArg {
                            offset: nrc.env_offset as u64, align: 2, memory_index: 0,
                        })),
                        _ => out.push(Instruction::I32Store(MemArg {
                            offset: nrc.env_offset as u64, align: 2, memory_index: 0,
                        })),
                    }
                }

                for (local_idx, _vt) in &call.arg_locals {
                    out.push(Instruction::LocalGet(*local_idx));
                }
                out.push(Instruction::Call(call.func_idx));
                let n_results = self.functions.iter()
                    .find(|f| f.wasm_func_idx == call.func_idx)
                    .map_or(0, |f| f.results.len());
                for _ in 0..n_results {
                    out.push(Instruction::Drop);
                }

                for nrc in &call.named_return_captures {
                    out.push(Instruction::LocalGet(nrc.env_local));
                    match nrc.val_type {
                        ValType::I64 => out.push(Instruction::I64Load(MemArg {
                            offset: nrc.env_offset as u64, align: 3, memory_index: 0,
                        })),
                        ValType::F64 => out.push(Instruction::F64Load(MemArg {
                            offset: nrc.env_offset as u64, align: 3, memory_index: 0,
                        })),
                        ValType::F32 => out.push(Instruction::F32Load(MemArg {
                            offset: nrc.env_offset as u64, align: 2, memory_index: 0,
                        })),
                        _ => out.push(Instruction::I32Load(MemArg {
                            offset: nrc.env_offset as u64, align: 2, memory_index: 0,
                        })),
                    }
                    out.push(Instruction::LocalSet(nrc.named_return_local));
                }
            }
        }
    }

    fn is_comparable_type(type_name: &str) -> bool {
        matches!(
            type_name,
            "int" | "int8" | "int16" | "int32" | "int64"
            | "uint" | "uint8" | "uint16" | "uint32" | "uint64"
            | "uintptr" | "float32" | "float64" | "complex64" | "complex128"
            | "bool" | "string" | "byte" | "rune"
        )
    }

    fn validate_type_constraints(
        template: &ast::FuncDecl,
        subst: &HashMap<String, String>,
    ) -> Result<(), Error> {
        for field in &template.typ.typ_params.list {
            let constraint_name = match &field.typ {
                ast::Expression::Ident(id) => Some(id.name.as_str()),
                _ => None,
            };
            if constraint_name == Some("comparable") {
                for name_ident in &field.name {
                    if let Some(concrete) = subst.get(&name_ident.name) {
                        if !Self::is_comparable_type(concrete) {
                            return Err(Error::TypeError(format!(
                                "{} does not satisfy comparable constraint",
                                concrete
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn monomorphize_func_decl(
        &self,
        template: &ast::FuncDecl,
        mono_name: &str,
        subst: &HashMap<String, String>,
    ) -> ast::FuncDecl {
        let mut decl = template.clone();
        decl.name = ast::Ident {
            name: mono_name.to_string(),
            pos: template.name.pos,
        };
        // Clear type params so the specialized function is not treated as generic
        decl.typ.typ_params = ast::FieldList { pos: None, list: vec![] };

        // Substitute type parameters in function parameter types
        for field in &mut decl.typ.params.list {
            Self::subst_type_in_expr(&mut field.typ, subst);
        }
        // Substitute in return types
        for field in &mut decl.typ.result.list {
            Self::subst_type_in_expr(&mut field.typ, subst);
        }
        // Substitute in body
        if let Some(ref mut body) = decl.body {
            Self::subst_type_in_block(body, subst);
        }

        decl
    }

    fn subst_type_in_expr(expr: &mut ast::Expression, subst: &HashMap<String, String>) {
        match expr {
            ast::Expression::Ident(ident) => {
                if let Some(replacement) = subst.get(&ident.name) {
                    ident.name = replacement.clone();
                }
            }
            ast::Expression::Call(call) => {
                Self::subst_type_in_expr(&mut call.func, subst);
                for arg in &mut call.args {
                    Self::subst_type_in_expr(arg, subst);
                }
            }
            ast::Expression::Operation(op) => {
                Self::subst_type_in_expr(&mut op.x, subst);
                if let Some(ref mut y) = op.y {
                    Self::subst_type_in_expr(y, subst);
                }
            }
            ast::Expression::Paren(p) => {
                Self::subst_type_in_expr(&mut p.expr, subst);
            }
            ast::Expression::Index(idx) => {
                if let Some(ref mut left) = idx.left {
                    Self::subst_type_in_expr(left, subst);
                }
                Self::subst_type_in_expr(&mut idx.index, subst);
            }
            ast::Expression::Selector(sel) => {
                Self::subst_type_in_expr(&mut sel.x, subst);
            }
            ast::Expression::Star(star) => {
                Self::subst_type_in_expr(&mut star.right, subst);
            }
            ast::Expression::TypeSlice(sl) => {
                Self::subst_type_in_expr(&mut sl.typ, subst);
            }
            ast::Expression::TypeArray(arr) => {
                Self::subst_type_in_expr(&mut arr.typ, subst);
                Self::subst_type_in_expr(&mut arr.len, subst);
            }
            ast::Expression::TypeMap(m) => {
                Self::subst_type_in_expr(&mut m.key, subst);
                Self::subst_type_in_expr(&mut m.val, subst);
            }
            ast::Expression::TypePointer(p) => {
                Self::subst_type_in_expr(&mut p.typ, subst);
            }
            ast::Expression::CompositeLit(cl) => {
                Self::subst_type_in_expr(&mut cl.typ, subst);
            }
            ast::Expression::TypeAssert(ta) => {
                Self::subst_type_in_expr(&mut ta.left, subst);
                if let Some(ref mut right) = ta.right {
                    Self::subst_type_in_expr(right, subst);
                }
            }
            _ => {}
        }
    }

    fn subst_type_in_stmt(stmt: &mut ast::Statement, subst: &HashMap<String, String>) {
        match stmt {
            ast::Statement::Expr(es) => Self::subst_type_in_expr(&mut es.expr, subst),
            ast::Statement::Assign(a) => {
                for e in &mut a.left {
                    Self::subst_type_in_expr(e, subst);
                }
                for e in &mut a.right {
                    Self::subst_type_in_expr(e, subst);
                }
            }
            ast::Statement::Return(r) => {
                for e in &mut r.ret {
                    Self::subst_type_in_expr(e, subst);
                }
            }
            ast::Statement::If(i) => {
                if let Some(ref mut init) = i.init {
                    Self::subst_type_in_stmt(init, subst);
                }
                Self::subst_type_in_expr(&mut i.cond, subst);
                Self::subst_type_in_block(&mut i.body, subst);
                if let Some(ref mut els) = i.else_ {
                    Self::subst_type_in_stmt(els, subst);
                }
            }
            ast::Statement::For(f) => {
                if let Some(ref mut init) = f.init {
                    Self::subst_type_in_stmt(init, subst);
                }
                if let Some(ref mut cond) = f.cond {
                    Self::subst_type_in_stmt(cond, subst);
                }
                if let Some(ref mut post) = f.post {
                    Self::subst_type_in_stmt(post, subst);
                }
                Self::subst_type_in_block(&mut f.body, subst);
            }
            ast::Statement::Range(r) => {
                if let Some(ref mut key) = r.key {
                    Self::subst_type_in_expr(key, subst);
                }
                if let Some(ref mut val) = r.value {
                    Self::subst_type_in_expr(val, subst);
                }
                Self::subst_type_in_expr(&mut r.expr, subst);
                Self::subst_type_in_block(&mut r.body, subst);
            }
            ast::Statement::Block(b) => Self::subst_type_in_block(b, subst),
            ast::Statement::Switch(sw) => {
                if let Some(ref mut init) = sw.init {
                    Self::subst_type_in_stmt(init, subst);
                }
                if let Some(ref mut tag) = sw.tag {
                    Self::subst_type_in_expr(tag, subst);
                }
                for case in &mut sw.block.body {
                    for e in &mut case.list {
                        Self::subst_type_in_expr(e, subst);
                    }
                    for s in case.body.as_mut().iter_mut() {
                        Self::subst_type_in_stmt(s, subst);
                    }
                }
            }
            ast::Statement::IncDec(id) => Self::subst_type_in_expr(&mut id.expr, subst),
            ast::Statement::Declaration(d) => {
                match d {
                    ast::DeclStmt::Variable(v) => {
                        for spec in &mut v.specs {
                            if let Some(ref mut typ) = spec.typ {
                                Self::subst_type_in_expr(typ, subst);
                            }
                            for val in &mut spec.values {
                                Self::subst_type_in_expr(val, subst);
                            }
                        }
                    }
                    ast::DeclStmt::Const(c) => {
                        for spec in &mut c.specs {
                            if let Some(ref mut typ) = spec.typ {
                                Self::subst_type_in_expr(typ, subst);
                            }
                            for val in &mut spec.values {
                                Self::subst_type_in_expr(val, subst);
                            }
                        }
                    }
                    _ => {}
                }
            }
            ast::Statement::Defer(d) => {
                Self::subst_type_in_expr(&mut d.call.func, subst);
                for arg in &mut d.call.args {
                    Self::subst_type_in_expr(arg, subst);
                }
            }
            _ => {}
        }
    }

    fn subst_type_in_block(block: &mut ast::BlockStmt, subst: &HashMap<String, String>) {
        for stmt in &mut block.list {
            Self::subst_type_in_stmt(stmt, subst);
        }
    }

    fn infer_struct_type_from_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> Option<String> {
        match expr {
            ast::Expression::Ident(ident) => {
                locals.get_var_struct_type(&ident.name).map(|s| s.to_string())
            }
            ast::Expression::Selector(sel) => {
                let parent_type = self.infer_struct_type_from_expr(sel.x.as_ref(), locals)?;
                let struct_def = self.struct_defs.get(&parent_type)?;
                let field = struct_def.find_field(&sel.sel.name)?;
                let tag = field.go_type_tag.as_deref()?;
                if tag.starts_with("__") {
                    None
                } else {
                    Some(tag.to_string())
                }
            }
            ast::Expression::Index(idx) => {
                if let Some(ast::Expression::Ident(ident)) = idx.left.as_deref() {
                    if let Some(mti) = locals.map_types.get(&ident.name) {
                        if let Some(ref st) = mti.val_struct_type {
                            return Some(st.clone());
                        }
                    }
                    if let Some(st) = locals.slice_elem_struct_types.get(&ident.name) {
                        return Some(st.clone());
                    }
                }
                None
            }
            ast::Expression::Call(call) => {
                self.infer_return_struct_type(call, locals)
            }
            _ => None,
        }
    }

    fn infer_return_struct_type(&self, call: &ast::Call, locals: &LocalAlloc) -> Option<String> {
        match call.func.as_ref() {
            ast::Expression::Ident(ident) => {
                let fi = self.functions.iter().find(|f| f.name == ident.name && f.recv_type.is_none())?;
                fi.result_go_types.first().cloned()
                    .filter(|t| self.struct_defs.contains_key(t))
            }
            ast::Expression::Selector(sel) => {
                if let ast::Expression::Ident(recv_ident) = sel.x.as_ref() {
                    let recv_type = locals.get_var_struct_type(&recv_ident.name)
                        .map(|s| s.to_string())
                        .or_else(|| {
                            if self.struct_defs.contains_key(&recv_ident.name) {
                                Some(recv_ident.name.clone())
                            } else {
                                None
                            }
                        });
                    if let Some(type_name) = recv_type {
                        let qualified = format!("{}.{}", type_name, sel.sel.name);
                        let fi = self.functions.iter().find(|f| f.name == qualified)?;
                        return fi.result_go_types.first().cloned()
                            .filter(|t| self.struct_defs.contains_key(t));
                    }
                }
                let parent_type = self.infer_struct_type_from_expr(sel.x.as_ref(), locals)?;
                let qualified = format!("{}.{}", parent_type, sel.sel.name);
                let fi = self.functions.iter().find(|f| f.name == qualified)?;
                fi.result_go_types.first().cloned()
                    .filter(|t| self.struct_defs.contains_key(t))
            }
            _ => None,
        }
    }

    fn infer_go_type_from_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> Option<String> {
        match expr {
            ast::Expression::BasicLit(lit) => match lit.kind {
                LitKind::Integer => Some("int".to_string()),
                LitKind::Float => Some("float64".to_string()),
                LitKind::String => Some("string".to_string()),
                LitKind::Char => Some("int32".to_string()),
                _ => None,
            },
            ast::Expression::Ident(ident) => match ident.name.as_str() {
                "true" | "false" => Some("bool".to_string()),
                "nil" => None,
                _ => {
                    if locals.string_locals.contains_key(&ident.name) {
                        return Some("string".to_string());
                    }
                    if locals.slice_elem_types.contains_key(&ident.name) {
                        return None;
                    }
                    let vt = locals.find_val_type(&ident.name);
                    match vt {
                        Some(ValType::I64) => Some("int".to_string()),
                        Some(ValType::F64) => Some("float64".to_string()),
                        Some(ValType::F32) => Some("float32".to_string()),
                        Some(ValType::I32) => Some("int32".to_string()),
                        _ => None,
                    }
                }
            },
            ast::Expression::CompositeLit(cl) => Some(Self::type_expr_to_go_string(&cl.typ)),
            ast::Expression::Operation(op) => self.infer_go_type_from_expr(&op.x, locals),
            _ => None,
        }
    }

    fn type_expr_to_go_string(expr: &ast::Expression) -> String {
        match expr {
            ast::Expression::Ident(id) => id.name.clone(),
            ast::Expression::TypeSlice(sl) => format!("[]{}", Self::type_expr_to_go_string(&sl.typ)),
            ast::Expression::TypeArray(arr) => format!("[?]{}", Self::type_expr_to_go_string(&arr.typ)),
            ast::Expression::TypeMap(m) => format!(
                "map[{}]{}",
                Self::type_expr_to_go_string(&m.key),
                Self::type_expr_to_go_string(&m.val),
            ),
            ast::Expression::TypePointer(p) => format!("*{}", Self::type_expr_to_go_string(&p.typ)),
            _ => String::new(),
        }
    }

    fn try_unify_type_param(
        param_type: &ast::Expression,
        arg_go_type: &str,
        type_param_names: &std::collections::HashSet<String>,
        subst: &mut HashMap<String, String>,
    ) {
        match param_type {
            ast::Expression::Ident(id) if type_param_names.contains(&id.name) => {
                subst.entry(id.name.clone()).or_insert_with(|| arg_go_type.to_string());
            }
            ast::Expression::TypeSlice(sl) => {
                if let Some(elem) = arg_go_type.strip_prefix("[]") {
                    Self::try_unify_type_param(&sl.typ, elem, type_param_names, subst);
                }
            }
            ast::Expression::TypeMap(m) => {
                if let Some(rest) = arg_go_type.strip_prefix("map[") {
                    if let Some(bracket_end) = rest.find(']') {
                        let key = &rest[..bracket_end];
                        let val = &rest[bracket_end + 1..];
                        Self::try_unify_type_param(&m.key, key, type_param_names, subst);
                        Self::try_unify_type_param(&m.val, val, type_param_names, subst);
                    }
                }
            }
            ast::Expression::TypePointer(p) => {
                if let Some(inner) = arg_go_type.strip_prefix('*') {
                    Self::try_unify_type_param(&p.typ, inner, type_param_names, subst);
                }
            }
            _ => {}
        }
    }

    fn setup_named_composite_var(
        &self,
        var_name: &str,
        underlying: &ast::Expression,
        locals: &mut LocalAlloc,
    ) {
        match underlying {
            ast::Expression::TypeSlice(slice_type) => {
                locals.set_var_struct_type(var_name, "__slice");
                let elem_vt = Self::infer_array_elem_vt(&slice_type.typ);
                locals.slice_elem_types.insert(var_name.to_string(), elem_vt);
                if let ast::Expression::TypeSlice(inner_st) = slice_type.typ.as_ref() {
                    let inner_vt = Self::infer_array_elem_vt(&inner_st.typ);
                    locals.nested_slice_inner_elem_types.insert(var_name.to_string(), inner_vt);
                }
                if let ast::Expression::Ident(el_id) = slice_type.typ.as_ref() {
                    if self.struct_defs.contains_key(&el_id.name) {
                        locals.slice_elem_struct_types.insert(var_name.to_string(), el_id.name.clone());
                    }
                    if el_id.name == "rune" || el_id.name == "int32" {
                        locals.rune_slices.insert(var_name.to_string());
                    }
                }
            }
            ast::Expression::TypeMap(map_type) => {
                locals.set_var_struct_type(var_name, "__map");
                let key_vt = Self::infer_array_elem_vt(&map_type.key);
                let val_vt = Self::infer_array_elem_vt(&map_type.val);
                let is_string_key = matches!(map_type.key.as_ref(), ast::Expression::Ident(id) if id.name == "string");
                let is_string_val = matches!(map_type.val.as_ref(), ast::Expression::Ident(id) if id.name == "string");
                let key_size = if is_string_key { 8u32 } else { val_type_byte_size(key_vt) };
                let val_size = if is_string_val { 8u32 } else { val_type_byte_size(val_vt) };
                let val_struct_type = if let ast::Expression::Ident(vid) = map_type.val.as_ref() {
                    if self.struct_defs.contains_key(&vid.name) { Some(vid.name.clone()) } else { None }
                } else { None };
                let nested = self.build_nested_map_type_info(map_type);
                locals.map_types.insert(var_name.to_string(), MapTypeInfo {
                    key_vt,
                    val_vt,
                    key_size,
                    val_size,
                    is_string_key,
                    is_string_val,
                    val_struct_type,
                    nested_map_val_type: nested,
                });
            }
            ast::Expression::TypeArray(arr_type) => {
                let arr_len = if let ast::Expression::BasicLit(lit) = arr_type.len.as_ref() {
                    lit.value.parse::<u32>().unwrap_or(0)
                } else { 0 };
                let elem_vt = Self::infer_array_elem_vt(&arr_type.typ);
                locals.set_var_struct_type(var_name, "__array");
                locals.array_info.insert(var_name.to_string(), (elem_vt, arr_len));
            }
            _ => {}
        }
    }

    /// Monomorphize a generic type definition with concrete type arguments.
    /// Returns the monomorphized struct name (e.g., "Pair__int_string").
    fn monomorphize_generic_type(
        &mut self,
        type_name: &str,
        type_args: &[String],
    ) -> Result<String, Error> {
        let mono_name = format!("{}__{}", type_name, type_args.join("_"));

        if self.struct_defs.contains_key(&mono_name) {
            return Ok(mono_name);
        }

        let template = self.generic_types.get(type_name).cloned()
            .ok_or_else(|| Error::InternalError(format!("generic type '{}' not found", type_name)))?;

        let mut subst: HashMap<String, String> = HashMap::new();
        let mut idx = 0;
        for field in &template.params.list {
            for name_ident in &field.name {
                if idx < type_args.len() {
                    subst.insert(name_ident.name.clone(), type_args[idx].clone());
                }
                idx += 1;
            }
        }

        let mut specialized_type = template.typ.clone();
        Self::subst_type_in_expr(&mut specialized_type, &subst);

        if let ast::Expression::TypeStruct(struct_type) = &specialized_type {
            let struct_def = self.compute_struct_def(&struct_type.fields);
            self.struct_defs.insert(mono_name.clone(), struct_def);
            self.register_struct_field_map_types(&mono_name, &struct_type.fields);
        }

        Ok(mono_name)
    }

    fn resolve_type_name<'a>(&'a self, name: &'a str) -> &'a str {
        let mut resolved = name;
        for _ in 0..10 {
            if let Some((base, _is_alias)) = self.type_aliases.get(resolved) {
                resolved = base;
            } else {
                break;
            }
        }
        resolved
    }

    fn _is_type_alias(&self, name: &str) -> bool {
        self.type_aliases
            .get(name)
            .map_or(false, |(_base, is_alias)| *is_alias)
    }

    fn field_to_wasm_types(&self, field: &ast::Field) -> Vec<WasmType> {
        match &field.typ {
            ast::Expression::Ident(ident) => match self.resolve_type_name(&ident.name) {
                "bool" | "byte" | "uint8" | "int8" | "int16" | "uint16" | "int32"
                | "uint32" | "rune" | "uintptr" => vec![WasmType::I32],
                "int" | "int64" | "uint" | "uint64" => vec![WasmType::I64],
                "float32" => vec![WasmType::F32],
                "float64" => vec![WasmType::F64],
                "string" => vec![WasmType::I32, WasmType::I32],
                "error" => vec![WasmType::I32],
                "Context" => vec![WasmType::I32],
                _ => vec![WasmType::I32],
            },
            ast::Expression::TypePointer(_) => vec![WasmType::I32],
            ast::Expression::TypeSlice(_) => {
                vec![WasmType::I32, WasmType::I32, WasmType::I32]
            }
            ast::Expression::TypeArray(_) => vec![WasmType::I32],
            ast::Expression::TypeMap(_) => vec![WasmType::I32],
            ast::Expression::TypeFunction(_) => vec![WasmType::I32, WasmType::I32],
            ast::Expression::TypeStruct(_) => vec![WasmType::I32],
            ast::Expression::TypeInterface(_) => vec![WasmType::I32],
            ast::Expression::Ellipsis(_) => vec![WasmType::I32], // variadic → slice header ptr
            _ => vec![WasmType::I32],
        }
    }

    fn infer_val_type(&self, expr: &ast::Expression, locals: &LocalAlloc) -> ValType {
        match expr {
            ast::Expression::BasicLit(lit) => match lit.kind {
                LitKind::Integer => ValType::I64,
                LitKind::Float => ValType::F64,
                LitKind::String => ValType::I32,
                LitKind::Char => ValType::I32,
                LitKind::Imag => ValType::I32,
                _ => ValType::I64,
            },
            ast::Expression::Ident(ident) => match ident.name.as_str() {
                "true" | "false" => ValType::I32,
                "nil" => ValType::I32,
                _ => {
                    if let Some(vt) = locals.find_type(&ident.name) {
                        return vt;
                    }
                    if let Some(ref cc) = self.closure_captures {
                        if let Some(cap) = cc.captures.iter().find(|c| c.name == ident.name) {
                            return cap.val_type;
                        }
                        if let Some((_, vt)) = cc.outer_locals.iter().find(|(n, _)| n == &ident.name) {
                            return *vt;
                        }
                    }
                    if let Some(cv) = self.constants.get(&ident.name) {
                        return match cv {
                            ConstValue::I64(_) => ValType::I64,
                            ConstValue::F64(_) => ValType::F64,
                            ConstValue::Bool(_) => ValType::I32,
                            ConstValue::Str(_) => ValType::I32,
                            ConstValue::Complex128(_, _) => ValType::I32,
                        };
                    }
                    if let Some(&(_idx, vt)) = self.global_vars.get(&ident.name) {
                        return vt;
                    }
                    ValType::I64
                }
            },
            ast::Expression::Operation(op) => {
                if matches!(
                    op.op,
                    Operator::Equal
                        | Operator::NotEqual
                        | Operator::Less
                        | Operator::LessEqual
                        | Operator::Greater
                        | Operator::GreaterEqual
                        | Operator::AndAnd
                        | Operator::OrOr
                        | Operator::Not
                ) {
                    return ValType::I32;
                }
                if op.y.is_some() {
                    let lhs = self.infer_val_type(&op.x, locals);
                    let rhs = self.infer_val_type(op.y.as_ref().unwrap(), locals);
                    if lhs == ValType::F64 || rhs == ValType::F64 {
                        ValType::F64
                    } else {
                        lhs
                    }
                } else {
                    if op.op == Operator::And {
                        ValType::I32
                    } else if op.op == Operator::Star {
                        self.infer_deref_type(&op.x, locals)
                    } else {
                        self.infer_val_type(&op.x, locals)
                    }
                }
            }
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    match ident.name.as_str() {
                        "float64" => ValType::F64,
                        "float32" => ValType::F32,
                        "int" | "int64" | "uint" | "uint64" => ValType::I64,
                        "int8" | "int16" | "int32" | "rune"
                        | "byte" | "uint8" | "uint16" | "uint32" | "bool" | "uintptr" => ValType::I32,
                        "len" | "cap" => ValType::I64,
                        "make" | "append" | "new" | "complex" | "recover" => ValType::I32,
                        "copy" => ValType::I64,
                        "string" => ValType::I32,
                        "real" => {
                            if let Some(arg) = call.args.first() {
                                if self.is_complex64_expr(arg, locals) { ValType::F32 } else { ValType::F64 }
                            } else {
                                ValType::F64
                            }
                        }
                        "imag" => {
                            if let Some(arg) = call.args.first() {
                                if self.is_complex64_expr(arg, locals) { ValType::F32 } else { ValType::F64 }
                            } else {
                                ValType::F64
                            }
                        }
                        "min" | "max" => {
                            if let Some(first_arg) = call.args.first() {
                                self.infer_val_type(first_arg, locals)
                            } else {
                                ValType::I64
                            }
                        }
                        _ => {
                            if let Some(fi) = self.functions.iter().find(|f| f.name == ident.name) {
                                fi.results.first().map_or(ValType::I64, |wt| wt.to_val_type())
                            } else {
                                ValType::I64
                            }
                        }
                    }
                } else if let ast::Expression::TypeSlice(_) = call.func.as_ref() {
                    ValType::I32
                } else if let ast::Expression::TypeArray(_) = call.func.as_ref() {
                    ValType::I32
                } else if let ast::Expression::Selector(sel) = call.func.as_ref() {
                    if let ast::Expression::Ident(receiver) = sel.x.as_ref() {
                        match (receiver.name.as_str(), sel.sel.name.as_str()) {
                            ("math", _) => ValType::F64,
                            _ => {
                                let method_name = &sel.sel.name;
                                // Check if receiver is an interface variable
                                if self.is_interface_var(&receiver.name, locals) {
                                    if let Some(fi) = self.functions.iter().find(|f| {
                                        f.recv_type.is_some()
                                            && f.name.ends_with(&format!(".{}", method_name))
                                    }) {
                                        return fi.results.first().map_or(ValType::I64, |wt| wt.to_val_type());
                                    }
                                }
                                // Try qualified method name (Type.Method)
                                if let Some(type_name) = locals.get_var_struct_type(&receiver.name) {
                                    let qname = format!("{}.{}", type_name, method_name);
                                    if let Some(fi) = self.functions.iter().find(|f| f.name == qname) {
                                        return fi.results.first().map_or(ValType::I64, |wt| wt.to_val_type());
                                    }
                                }
                                // Try bare method name
                                if let Some(fi) = self.functions.iter().find(|f| f.name == *method_name) {
                                    fi.results.first().map_or(ValType::I64, |wt| wt.to_val_type())
                                } else {
                                    ValType::I64
                                }
                            }
                        }
                    } else {
                        ValType::I64
                    }
                } else {
                    ValType::I64
                }
            }
            ast::Expression::Paren(p) => self.infer_val_type(&p.expr, locals),
            ast::Expression::Selector(sel) => {
                if let Some(type_name) = self.infer_struct_type_from_expr(sel.x.as_ref(), locals) {
                    if let Some(sd) = self.struct_defs.get(&type_name) {
                        if let Some(field) = sd.find_field(&sel.sel.name) {
                            return field.wasm_type.to_val_type();
                        }
                    }
                }
                ValType::I32
            }
            ast::Expression::CompositeLit(_) => ValType::I32,
            ast::Expression::Index(idx) => {
                if let Some(left) = idx.left.as_deref() {
                    if self.is_string_expr(left, locals) {
                        return ValType::I32;
                    }
                }
                if let Some(ast::Expression::Ident(ident)) = idx.left.as_deref() {
                    if let Some(mti) = locals.map_types.get(&ident.name) {
                        return mti.val_vt;
                    }
                    locals
                        .slice_elem_types
                        .get(&ident.name)
                        .copied()
                        .unwrap_or(ValType::I64)
                } else if let Some(ast::Expression::Index(outer_idx)) = idx.left.as_deref() {
                    if let Some(ast::Expression::Ident(outer_ident)) = outer_idx.left.as_deref() {
                        if let Some(&inner_vt) = locals.nested_slice_inner_elem_types.get(&outer_ident.name) {
                            return inner_vt;
                        }
                    }
                    ValType::I64
                } else {
                    ValType::I64
                }
            }
            ast::Expression::Slice(slice) => {
                if self.is_string_expr(&slice.left, locals) {
                    ValType::I32
                } else {
                    ValType::I32
                }
            }
            ast::Expression::FuncLit(_) => ValType::I32,
            ast::Expression::TypeAssert(ta) => {
                if let Some(ref target) = ta.right {
                    if let ast::Expression::Ident(type_ident) = target.as_ref() {
                        return Self::val_type_for_type_name(&type_ident.name);
                    }
                }
                ValType::I64
            }
            _ => ValType::I64,
        }
    }

    fn expr_to_val_type(&self, expr: &ast::Expression) -> ValType {
        match expr {
            ast::Expression::Ident(ident) => match self.resolve_type_name(&ident.name) {
                "bool" | "byte" | "uint8" | "int8" | "int16" | "uint16" | "int32"
                | "uint32" | "rune" | "uintptr" => ValType::I32,
                "int" | "int64" | "uint" | "uint64" => ValType::I64,
                "float32" => ValType::F32,
                "float64" => ValType::F64,
                "string" | "error" | "any" => ValType::I32,
                name if self.iface_defs.contains_key(name) => ValType::I32,
                _ => ValType::I32,
            },
            ast::Expression::TypePointer(_) => ValType::I32,
            ast::Expression::TypeSlice(_) => ValType::I32,
            ast::Expression::TypeInterface(_) => ValType::I32,
            ast::Expression::TypeMap(_) => ValType::I32,
            ast::Expression::TypeFunction(_) => ValType::I32,
            ast::Expression::TypeArray(_) => ValType::I32,
            ast::Expression::TypeStruct(_) => ValType::I32,
            _ => ValType::I64,
        }
    }

    fn block_always_returns(stmts: &[ast::Statement]) -> bool {
        stmts
            .iter()
            .rev()
            .find(|s| !matches!(s, ast::Statement::Empty(_)))
            .map_or(false, |s| Self::stmt_always_returns(s))
    }

    fn stmt_always_returns(stmt: &ast::Statement) -> bool {
        match stmt {
            ast::Statement::Return(_) => true,
            ast::Statement::Expr(expr_stmt) => Self::expr_is_panic_call(&expr_stmt.expr),
            ast::Statement::If(if_stmt) => {
                let then_returns = Self::block_always_returns(&if_stmt.body.list);
                let else_returns =
                    if_stmt
                        .else_
                        .as_ref()
                        .map_or(false, |els| match els.as_ref() {
                            ast::Statement::Block(block) => {
                                Self::block_always_returns(&block.list)
                            }
                            other => Self::stmt_always_returns(other),
                        });
                then_returns && else_returns
            }
            ast::Statement::Block(block) => Self::block_always_returns(&block.list),
            ast::Statement::For(for_stmt) => {
                for_stmt.cond.is_none()
                    && !Self::block_contains_break(&for_stmt.body.list, None)
            }
            ast::Statement::Switch(sw) => {
                Self::switch_is_terminating(&sw.block.body, None)
            }
            ast::Statement::TypeSwitch(ts) => {
                let has_default = ts.block.body.iter().any(|c| c.list.is_empty());
                if !has_default {
                    return false;
                }
                let cases_terminate = ts.block.body.iter().all(|c| Self::case_body_terminates(&c.body));
                if !cases_terminate {
                    return false;
                }
                !Self::switch_cases_contain_break(
                    ts.block.body.iter().map(|c| c.body.as_slice()),
                    None,
                )
            }
            ast::Statement::Range(_) => false,
            ast::Statement::Label(labeled) => {
                match labeled.stmt.as_ref() {
                    ast::Statement::For(for_stmt) => {
                        for_stmt.cond.is_none()
                            && !Self::block_contains_break(
                                &for_stmt.body.list,
                                Some(&labeled.name.name),
                            )
                    }
                    ast::Statement::Range(_) => false,
                    ast::Statement::Switch(sw) => {
                        Self::switch_is_terminating(&sw.block.body, Some(&labeled.name.name))
                    }
                    ast::Statement::TypeSwitch(ts) => {
                        let has_default = ts.block.body.iter().any(|c| c.list.is_empty());
                        if !has_default {
                            return false;
                        }
                        let cases_terminate = ts.block.body.iter().all(|c| Self::case_body_terminates(&c.body));
                        if !cases_terminate {
                            return false;
                        }
                        !Self::switch_cases_contain_break(
                            ts.block.body.iter().map(|c| c.body.as_slice()),
                            Some(&labeled.name.name),
                        )
                    }
                    other => Self::stmt_always_returns(other),
                }
            }
            _ => false,
        }
    }

    fn expr_is_panic_call(expr: &ast::Expression) -> bool {
        if let ast::Expression::Call(call) = expr {
            if let ast::Expression::Ident(ident) = call.func.as_ref() {
                return ident.name == "panic";
            }
        }
        false
    }

    fn block_contains_break(stmts: &[ast::Statement], for_label: Option<&str>) -> bool {
        for stmt in stmts {
            match stmt {
                ast::Statement::Branch(b) if b.key == Keyword::Break => {
                    if b.ident.is_none() {
                        return true;
                    }
                    if let (Some(brk_label), Some(fl)) = (&b.ident, for_label) {
                        if brk_label.name == fl {
                            return true;
                        }
                    }
                }
                ast::Statement::If(if_stmt) => {
                    if Self::block_contains_break(&if_stmt.body.list, for_label) {
                        return true;
                    }
                    if let Some(ref els) = if_stmt.else_ {
                        if let ast::Statement::Block(block) = els.as_ref() {
                            if Self::block_contains_break(&block.list, for_label) {
                                return true;
                            }
                        }
                    }
                }
                ast::Statement::Block(block) => {
                    if Self::block_contains_break(&block.list, for_label) {
                        return true;
                    }
                }
                ast::Statement::Switch(sw) if for_label.is_some() => {
                    for case in &sw.block.body {
                        if Self::block_contains_labeled_break(&case.body, for_label.unwrap()) {
                            return true;
                        }
                    }
                }
                ast::Statement::TypeSwitch(ts) if for_label.is_some() => {
                    for case in &ts.block.body {
                        if Self::block_contains_labeled_break(&case.body, for_label.unwrap()) {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn block_contains_labeled_break(stmts: &[ast::Statement], label: &str) -> bool {
        for stmt in stmts {
            match stmt {
                ast::Statement::Branch(b)
                    if b.key == Keyword::Break
                        && b.ident.as_ref().map_or(false, |id| id.name == label) =>
                {
                    return true;
                }
                ast::Statement::If(if_stmt) => {
                    if Self::block_contains_labeled_break(&if_stmt.body.list, label) {
                        return true;
                    }
                    if let Some(ref els) = if_stmt.else_ {
                        if let ast::Statement::Block(block) = els.as_ref() {
                            if Self::block_contains_labeled_break(&block.list, label) {
                                return true;
                            }
                        }
                    }
                }
                ast::Statement::Block(block) => {
                    if Self::block_contains_labeled_break(&block.list, label) {
                        return true;
                    }
                }
                ast::Statement::Switch(sw) => {
                    for case in &sw.block.body {
                        if Self::block_contains_labeled_break(&case.body, label) {
                            return true;
                        }
                    }
                }
                ast::Statement::TypeSwitch(ts) => {
                    for case in &ts.block.body {
                        if Self::block_contains_labeled_break(&case.body, label) {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn switch_is_terminating(cases: &[ast::CaseClause], label: Option<&str>) -> bool {
        let has_default = cases.iter().any(|c| c.tok == Keyword::Default);
        if !has_default {
            return false;
        }
        let cases_terminate = cases.iter().all(|c| Self::case_body_terminates(&c.body));
        if !cases_terminate {
            return false;
        }
        !Self::switch_cases_contain_break(
            cases.iter().map(|c| c.body.as_slice()),
            label,
        )
    }

    fn switch_cases_contain_break<'a>(
        cases: impl Iterator<Item = &'a [ast::Statement]>,
        label: Option<&str>,
    ) -> bool {
        for case_body in cases {
            if Self::case_stmts_contain_unlabeled_break(case_body, label) {
                return true;
            }
        }
        false
    }

    fn case_stmts_contain_unlabeled_break(stmts: &[ast::Statement], switch_label: Option<&str>) -> bool {
        for stmt in stmts {
            match stmt {
                ast::Statement::Branch(b) if b.key == Keyword::Break => {
                    if b.ident.is_none() {
                        return true;
                    }
                    if let Some(ref brk_label) = b.ident {
                        if let Some(sw_label) = switch_label {
                            if brk_label.name == sw_label {
                                return true;
                            }
                        }
                    }
                }
                ast::Statement::If(if_stmt) => {
                    if Self::case_stmts_contain_unlabeled_break(&if_stmt.body.list, switch_label) {
                        return true;
                    }
                    if let Some(ref els) = if_stmt.else_ {
                        if let ast::Statement::Block(block) = els.as_ref() {
                            if Self::case_stmts_contain_unlabeled_break(&block.list, switch_label) {
                                return true;
                            }
                        }
                    }
                }
                ast::Statement::Block(block) => {
                    if Self::case_stmts_contain_unlabeled_break(&block.list, switch_label) {
                        return true;
                    }
                }
                // Don't recurse into for/switch/select - break inside those refers to them, not the outer switch
                _ => {}
            }
        }
        false
    }

    fn case_body_terminates(stmts: &[ast::Statement]) -> bool {
        let last = stmts
            .iter()
            .rev()
            .find(|s| !matches!(s, ast::Statement::Empty(_)));
        if let Some(last) = last {
            if Self::stmt_always_returns(last) {
                return true;
            }
            if Self::is_fallthrough_stmt(last) {
                return true;
            }
        }
        false
    }

    fn is_fallthrough_stmt(stmt: &ast::Statement) -> bool {
        match stmt {
            ast::Statement::Branch(b) => b.key == Keyword::FallThrough,
            ast::Statement::Label(labeled) => Self::is_fallthrough_stmt(&labeled.stmt),
            _ => false,
        }
    }

    fn contains_fallthrough(stmt: &ast::Statement) -> bool {
        match stmt {
            ast::Statement::Branch(b) => b.key == Keyword::FallThrough,
            ast::Statement::Label(labeled) => Self::contains_fallthrough(&labeled.stmt),
            ast::Statement::Block(block) => block.list.iter().any(|s| Self::contains_fallthrough(s)),
            ast::Statement::If(if_stmt) => {
                if_stmt.body.list.iter().any(|s| Self::contains_fallthrough(s))
                    || if_stmt.else_.as_ref().map_or(false, |e| Self::contains_fallthrough(e))
            }
            ast::Statement::For(for_stmt) => {
                for_stmt.body.list.iter().any(|s| Self::contains_fallthrough(s))
            }
            ast::Statement::Range(range_stmt) => {
                range_stmt.body.list.iter().any(|s| Self::contains_fallthrough(s))
            }
            _ => false,
        }
    }

    fn infer_selector_struct_type(&self, expr: &ast::Expression, locals: &LocalAlloc) -> Option<String> {
        match expr {
            ast::Expression::Ident(ident) => {
                locals.get_var_struct_type(&ident.name).map(|s| s.to_string())
            }
            ast::Expression::Selector(sel) => {
                let parent_type = self.infer_selector_struct_type(sel.x.as_ref(), locals)?;
                let sdef = self.struct_defs.get(&parent_type)?;
                let field = sdef.fields.iter().find(|f| f.name == sel.sel.name)?;
                field.go_type_tag.clone()
            }
            _ => None,
        }
    }

    fn infer_deref_type(&self, expr: &ast::Expression, locals: &LocalAlloc) -> ValType {
        if let ast::Expression::Ident(ident) = expr {
            if let Some(type_name) = locals.get_var_struct_type(&ident.name) {
                if self.struct_defs.contains_key(type_name)
                    || type_name == "__context"
                    || type_name == "__slice"
                    || type_name == "__string"
                {
                    return ValType::I32;
                }
                return match type_name {
                    "__ptr_i64" => ValType::I64,
                    "__ptr_f32" => ValType::F32,
                    "__ptr_f64" => ValType::F64,
                    _ => ValType::I32,
                };
            }
        }
        ValType::I32
    }

    fn elem_size_and_align(vt: ValType) -> (i32, u32) {
        match vt {
            ValType::I32 | ValType::F32 => (4, 2),
            _ => (8, 3),
        }
    }

    fn expression_result_count(&self, expr: &ast::Expression, locals: Option<&LocalAlloc>) -> usize {
        match expr {
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    match ident.name.as_str() {
                        "panic" | "println" | "print" | "delete" | "clear" => 0,
                        "recover" => 2,
                        "len" | "cap" | "copy" | "make" | "append"
                        | "int" | "int64" | "uint" | "uint64"
                        | "float64" | "float32"
                        | "int8" | "int16" | "int32" | "rune"
                        | "byte" | "uint8" | "uint16" | "uint32"
                        | "bool"
                        | "new" | "min" | "max"
                        | "complex" | "real" | "imag" => 1,
                        "string" => 2,
                        _ => {
                            if let Some(loc) = locals {
                                if let Some(&(func_idx, _)) = loc.closure_info.get(&ident.name) {
                                    if let Some(fi) = self.functions.iter().find(|f| f.wasm_func_idx == func_idx) {
                                        return fi.results.len();
                                    }
                                }
                            }
                            if let Some(fi) =
                                self.functions.iter().find(|f| f.name == ident.name)
                            {
                                fi.results.len()
                            } else {
                                1
                            }
                        }
                    }
                } else if let ast::Expression::Selector(sel) = call.func.as_ref() {
                    if let ast::Expression::Ident(_recv_ident) = sel.x.as_ref() {
                        match sel.sel.name.as_str() {
                            "Log" => 0,
                            "QueryID" | "Database" | "Schema" | "User" | "Config" => 2,
                            _ => {
                                if let Some(loc) = locals {
                                    if let Some(qualified) = self.resolve_selector_method_name(sel, loc) {
                                        if let Some(fi) = self.functions.iter().find(|f| f.name == qualified) {
                                            return fi.results.len();
                                        }
                                    }
                                }
                                let method_name = &sel.sel.name;
                                if let Some(fi) = self.functions.iter().find(|f| {
                                    f.name.ends_with(&format!(".{}", method_name))
                                }) {
                                    fi.results.len()
                                } else {
                                    1
                                }
                            }
                        }
                    } else {
                        1
                    }
                } else {
                    1
                }
            }
            ast::Expression::BasicLit(lit) => match lit.kind {
                LitKind::String => 2,
                _ => 1,
            },
            ast::Expression::FuncLit(_) => 1,
            ast::Expression::Slice(_) => {
                // Non-string slices now produce a slice header (1 value).
                // String slices still produce (ptr, len) = 2 values, but
                // callers handle strings separately before reaching this.
                1
            }
            _ => 1,
        }
    }

    fn call_return_val_types(&self, expr: &ast::Expression, locals: &LocalAlloc) -> Vec<ValType> {
        if let ast::Expression::Call(call) = expr {
            if let ast::Expression::Ident(ident) = call.func.as_ref() {
                if let Some(fi) = self.functions.iter().find(|f| f.name == ident.name) {
                    return fi.results.iter().map(|wt| wt.to_val_type()).collect();
                }
            }
            if let ast::Expression::Selector(sel) = call.func.as_ref() {
                if let Some(qualified) = self.resolve_selector_method_name(sel, locals) {
                    if let Some(fi) = self.functions.iter().find(|f| f.name == qualified) {
                        return fi.results.iter().map(|wt| wt.to_val_type()).collect();
                    }
                }
            }
        }
        vec![]
    }

    fn call_return_go_types(&self, expr: &ast::Expression, locals: &LocalAlloc) -> Vec<String> {
        if let ast::Expression::Call(call) = expr {
            if let ast::Expression::Ident(ident) = call.func.as_ref() {
                if let Some(fi) = self.functions.iter().find(|f| f.name == ident.name) {
                    return fi.result_go_types.clone();
                }
            }
            if let ast::Expression::Selector(sel) = call.func.as_ref() {
                if let Some(qualified) = self.resolve_selector_method_name(sel, locals) {
                    if let Some(fi) = self.functions.iter().find(|f| f.name == qualified) {
                        return fi.result_go_types.clone();
                    }
                }
            }
        }
        vec![]
    }

    fn resolve_selector_method_name(&self, sel: &ast::Selector, locals: &LocalAlloc) -> Option<String> {
        if let ast::Expression::Ident(recv_ident) = sel.x.as_ref() {
            if let Some(type_name) = locals.get_var_struct_type(&recv_ident.name) {
                let resolved = self.resolve_type_name(type_name);
                let qualified = format!("{}.{}", resolved, sel.sel.name);
                if self.functions.iter().any(|f| f.name == qualified) {
                    return Some(qualified);
                }
            }
            let qualified = format!("{}.{}", recv_ident.name, sel.sel.name);
            if self.functions.iter().any(|f| f.name == qualified) {
                return Some(qualified);
            }
        }
        None
    }

    fn compile_multi_return_define(
        &mut self,
        assign: &ast::AssignStmt,
        ret_types: &[ValType],
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let go_types = self.call_return_go_types(&assign.right[0], locals);

        self.compile_expression(&assign.right[0], out, locals)?;

        // Pop return values into temp locals in reverse order (WASM stack is LIFO)
        let mut temps: Vec<(usize, u32, ValType)> = Vec::new();
        for i in (0..assign.left.len()).rev() {
            let vt = ret_types[i];
            let tmp = locals.add_local(&format!("__mret_tmp_{}", i), vt);
            out.push(Instruction::LocalSet(tmp));
            temps.push((i, tmp, vt));
        }
        temps.reverse();

        // Assign from temps into named locals in forward order
        let is_define = assign.op == Operator::Define;
        for (i, left) in assign.left.iter().enumerate() {
            if let ast::Expression::Ident(ident) = left {
                if ident.name == "_" {
                    continue;
                }
                let vt = ret_types[i];
                let (_, tmp, _) = temps[i];

                let is_iface = go_types.get(i).map_or(false, |gt| self.is_iface_go_type(gt));
                if is_iface {
                    let iface_tag = go_types.get(i)
                        .map(|gt| format!("__iface_{}", gt))
                        .unwrap_or_else(|| "__interface".to_string());
                    let local_idx = if is_define {
                        if let Some(existing) = locals.find_at_current_scope(&ident.name) {
                            existing
                        } else {
                            locals.add_local(&ident.name, vt)
                        }
                    } else {
                        locals.find(&ident.name).unwrap_or_else(|| locals.add_local(&ident.name, vt))
                    };
                    locals.set_var_struct_type(&ident.name, &iface_tag);
                    let tid_local = locals.add_local(
                        &format!("{}_type_id", ident.name),
                        ValType::I32,
                    );
                    self.iface_var_type_ids.insert(ident.name.clone(), tid_local);
                    out.push(Instruction::LocalGet(tmp));
                    out.push(Instruction::LocalSet(local_idx));
                    continue;
                }

                let local_idx = if is_define {
                    if let Some(existing) = locals.find_at_current_scope(&ident.name) {
                        existing
                    } else {
                        locals.add_local(&ident.name, vt)
                    }
                } else {
                    locals.find(&ident.name).unwrap_or_else(|| locals.add_local(&ident.name, vt))
                };
                out.push(Instruction::LocalGet(tmp));
                out.push(Instruction::LocalSet(local_idx));
            }
        }

        Ok(())
    }

    fn build_manifest(&mut self, file: &ast::File) -> Result<(), Error> {
        // Process free functions
        for decl in &file.decl {
            if let ast::Declaration::Function(func_decl) = decl {
                if func_decl.recv.is_some() {
                    continue;
                }
                let name = &func_decl.name.name;
                if !name.chars().next().map_or(false, |c| c.is_uppercase()) {
                    continue;
                }

                let (input_fields, _has_context) =
                    self.extract_input_fields(&func_decl.typ.params);
                let (output, returns_error) =
                    self.extract_output(&func_decl.typ.result);

                self.manifest.functions.push(FunctionDescriptor {
                    name: name.clone(),
                    input: TableDescriptor {
                        fields: input_fields,
                    },
                    output,
                    returns_error,
                });
            }
        }

        // Aggregate detection: find types with both Accumulate and Finalize methods
        let mut methods_by_type: HashMap<String, Vec<&ast::FuncDecl>> = HashMap::new();

        for decl in &file.decl {
            if let ast::Declaration::Function(func_decl) = decl {
                if let Some(recv) = &func_decl.recv {
                    if let Some(recv_type) = self.extract_recv_type_name(recv) {
                        methods_by_type
                            .entry(recv_type)
                            .or_default()
                            .push(func_decl);
                    }
                }
            }
        }

        for (type_name, methods) in &methods_by_type {
            let has_accumulate = methods.iter().any(|m| m.name.name == "Accumulate");
            let has_finalize = methods.iter().any(|m| m.name.name == "Finalize");

            if has_accumulate && has_finalize {
                let accumulate = match methods.iter().find(|m| m.name.name == "Accumulate") {
                    Some(m) => m,
                    None => continue,
                };
                let finalize = match methods.iter().find(|m| m.name.name == "Finalize") {
                    Some(m) => m,
                    None => continue,
                };

                let (input_fields, _) =
                    self.extract_input_fields(&accumulate.typ.params);
                let (output, returns_error) =
                    self.extract_output(&finalize.typ.result);

                // Export aggregate methods under standard names
                let accum_qname = format!("{}.Accumulate", type_name);
                let final_qname = format!("{}.Finalize", type_name);

                if let Some(fi) =
                    self.functions.iter().find(|f| f.name == accum_qname)
                {
                    self.export_section.export(
                        &format!("{}_accumulate", type_name),
                        ExportKind::Func,
                        fi.wasm_func_idx,
                    );
                }
                if let Some(fi) =
                    self.functions.iter().find(|f| f.name == final_qname)
                {
                    self.export_section.export(
                        &format!("{}_finalize", type_name),
                        ExportKind::Func,
                        fi.wasm_func_idx,
                    );
                }

                // Emit init function
                let struct_size = self
                    .struct_defs
                    .get(type_name.as_str())
                    .map_or(64, |sd| sd.total_size);
                let init_name = format!("{}_init", type_name);
                self.emit_aggregate_init(&init_name, struct_size)?;

                self.manifest.aggregates.push(AggregateDescriptor {
                    name: type_name.clone(),
                    input: TableDescriptor {
                        fields: input_fields,
                    },
                    output,
                    returns_error,
                });
            }
        }
        Ok(())
    }

    fn emit_aggregate_init(&mut self, name: &str, struct_size: u32) -> Result<(), Error> {
        let type_idx = self.next_type_idx;
        self.type_section
            .ty()
            .function(vec![], vec![ValType::I32]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let mut func = Function::new(vec![]);
        func.instruction(&Instruction::I32Const(struct_size as i32));
        func.instruction(&Instruction::Call(self.alloc_func_idx()?));
        func.instruction(&Instruction::End);

        self.code_section.function(&func);
        self.export_section
            .export(name, ExportKind::Func, func_idx);

        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: name.to_string(),
            params: vec![],
            results: vec![WasmType::I32],
            result_go_types: vec![],
            is_exported: true,
            recv_type: None,
            is_variadic: false,
            variadic_elem_vt: None,
            iface_param_indices: vec![],
        });
        Ok(())
    }

    fn extract_input_fields(
        &self,
        params: &ast::FieldList,
    ) -> (Vec<FieldDescriptor>, bool) {
        let mut fields = Vec::new();
        let mut has_context = false;

        for field in &params.list {
            if let ast::Expression::Ident(ident) = &field.typ {
                if ident.name == "Context" {
                    has_context = true;
                    continue;
                }
            }

            let type_name = self.expr_type_name(&field.typ);

            if field.name.is_empty() {
                fields.push(FieldDescriptor {
                    name: String::new(),
                    field_type: type_name,
                    nullable: false,
                });
            } else {
                for name in &field.name {
                    fields.push(FieldDescriptor {
                        name: name.name.clone(),
                        field_type: type_name.clone(),
                        nullable: false,
                    });
                }
            }
        }

        (fields, has_context)
    }

    fn extract_output(
        &self,
        result: &ast::FieldList,
    ) -> (OutputDescriptor, bool) {
        let mut returns_error = false;
        let mut output_fields: Vec<FieldDescriptor> = Vec::new();

        for field in &result.list {
            let type_name = self.expr_type_name(&field.typ);
            if type_name == "error" {
                returns_error = true;
                continue;
            }
            output_fields.push(FieldDescriptor {
                name: field
                    .name
                    .first()
                    .map_or(String::new(), |n| n.name.clone()),
                field_type: type_name.clone(),
                nullable: false,
            });
        }

        let output = if output_fields.len() == 1 && output_fields[0].name.is_empty() {
            OutputDescriptor::Scalar {
                scalar_type: output_fields[0].field_type.clone(),
            }
        } else if output_fields.is_empty() {
            OutputDescriptor::Scalar {
                scalar_type: "void".to_string(),
            }
        } else {
            OutputDescriptor::Table {
                fields: output_fields,
            }
        };

        (output, returns_error)
    }

    fn expr_type_name(&self, expr: &ast::Expression) -> String {
        match expr {
            ast::Expression::Ident(ident) => ident.name.clone(),
            ast::Expression::TypePointer(p) => {
                format!("*{}", self.expr_type_name(&p.typ))
            }
            ast::Expression::TypeSlice(s) => {
                format!("[]{}", self.expr_type_name(&s.typ))
            }
            ast::Expression::TypeArray(a) => {
                format!("array:{}", self.expr_type_name(&a.typ))
            }
            ast::Expression::TypeMap(m) => {
                format!(
                    "map[{}]{}",
                    self.expr_type_name(&m.key),
                    self.expr_type_name(&m.val)
                )
            }
            _ => "unknown".to_string(),
        }
    }

    fn build_module(&self) -> Vec<u8> {
        let mut module = Module::new();

        module.section(&self.type_section);
        module.section(&self.import_section);
        module.section(&self.function_section);
        if !self.table_section.is_empty() {
            module.section(&self.table_section);
        }
        module.section(&self.memory_section);
        module.section(&self.global_section);
        module.section(&self.export_section);
        if let Some(start_idx) = self.start_func_idx {
            let start_section = wasm_encoder::StartSection { function_index: start_idx };
            module.section(&start_section);
        }
        if !self.element_section.is_empty() {
            module.section(&self.element_section);
        }
        module.section(&self.code_section);

        module.finish()
    }
}
