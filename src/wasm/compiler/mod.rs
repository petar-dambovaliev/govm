use crate::parser::ast;
use crate::parser::token::{Keyword, LitKind, Operator};
use crate::symbols::{DefineType, Error, Qualifier, SymbolTable};
use crate::wasm::types::WasmType;
use crate::wasm::udf::{
    AggregateDescriptor, FieldDescriptor, FunctionDescriptor, Manifest, OutputDescriptor,
    TableDescriptor,
};
use std::collections::HashMap;
use wasm_encoder::{
    BlockType, CodeSection, CompositeInnerType, CompositeType, ConstExpr, ElementSection, Elements,
    ExportKind, ExportSection, FieldType, Function, FunctionSection, GlobalSection, GlobalType,
    HeapType, ImportSection, Instruction, MemArg, MemorySection, MemoryType, Module, RefType,
    StorageType, StructType, SubType, TableSection, TableType, TypeSection, ValType,
};

pub struct CompileResult {
    pub wasm_bytes: Vec<u8>,
    pub manifest: Manifest,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct FuncInfo {
    pub(crate) wasm_func_idx: u32,
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

pub(crate) struct DeferredCall {
    pub(crate) func_idx: u32,
    arg_locals: Vec<(u32, ValType)>,
    named_return_captures: Vec<NamedReturnCapture>,
}

pub(crate) struct NamedReturnCapture {
    pub(crate) env_local: u32,
    env_offset: u32,
    named_return_local: u32,
    val_type: ValType,
}

#[derive(Debug, Clone)]
pub(crate) struct StructDef {
    pub(crate) fields: Vec<StructFieldDef>,
    total_size: u32,
    embedded_types: Vec<(String, u32)>, // (type_name, offset_in_parent)
    pub(crate) gc_type_idx: Option<u32>,
}

#[derive(Debug, Clone)]
pub(crate) struct StructFieldDef {
    pub(crate) name: String,
    wasm_type: WasmType,
    offset: u32,
    go_type_tag: Option<String>,
    pub(crate) field_index: u32,
    pub(crate) slice_elem_type_tag: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum ConstValue {
    I64(i128),
    F64(f64),
    Bool(bool),
    Str(String),
    Complex128(f64, f64),
}

impl ConstValue {
    pub(crate) fn as_i64(&self) -> Option<i64> {
        match self {
            ConstValue::I64(v) => {
                if *v >= i64::MIN as i128 && *v <= i64::MAX as i128 {
                    Some(*v as i64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub(crate) fn as_u64(&self) -> Option<u64> {
        match self {
            ConstValue::I64(v) => {
                if *v >= 0 && *v <= u64::MAX as i128 {
                    Some(*v as u64)
                } else if *v < 0 && *v >= i64::MIN as i128 {
                    Some(*v as i64 as u64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl StructDef {
    pub(crate) fn find_field(&self, name: &str) -> Option<&StructFieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }
}

#[derive(Clone)]
pub(crate) struct CapturedVar {
    pub(crate) name: String,
    val_type: ValType,
    outer_local_idx: u32,
    env_offset: u32,
}

pub(crate) fn val_type_byte_size(vt: ValType) -> u32 {
    match vt {
        ValType::I64 | ValType::F64 => 8,
        _ => 4,
    }
}

use std::collections::HashSet;

#[derive(Debug, Clone)]
pub(crate) struct StackLocal {
    pub(crate) name: String,
    offset: u32,
    size: u32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct StackFrameInfo {
    pub(crate) locals: Vec<StackLocal>,
    total_size: u32,
    frame_base_local: Option<u32>,
}

impl StackFrameInfo {
    pub(crate) fn find(&self, name: &str) -> Option<&StackLocal> {
        self.locals.iter().find(|l| l.name == name)
    }
}

pub(crate) fn aligned_capture_env_offset(captures: &[CapturedVar], next_vt: ValType) -> u32 {
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

pub(crate) struct ClosureCaptureState {
    pub(crate) outer_locals: Vec<(String, ValType)>,
    captures: Vec<CapturedVar>,
    outer_closure_info: HashMap<String, (u32, u32)>,
    outer_closure_env_captures: HashMap<String, Vec<(String, u32, ValType)>>,
}

#[derive(Clone, Debug)]
pub(crate) struct FuncTypedParamInfo {
    pub(crate) func_idx_local: u32,
    pub(crate) env_ptr_local: u32,
    pub(crate) call_type_idx: u32,
    pub(crate) result_count: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct IterFuncInfo {
    pub(crate) yield_param_types: Vec<ValType>,
    pub(crate) func_idx: u32,
}

#[derive(Clone)]
pub(crate) struct MapTypeInfo {
    pub(crate) key_vt: ValType,
    val_vt: ValType,
    key_size: u32,
    val_size: u32,
    is_string_key: bool,
    is_string_val: bool,
    val_struct_type: Option<String>,
    nested_map_val_type: Option<Box<MapTypeInfo>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum GoType {
    Int8,
    Int16,
    Int32,
    Int64,

    Uint8,
    Uint16,
    Uint32,
    Uint64,

    Float32,
    Float64,

    Bool,

    String,
    Slice(Box<GoType>),
    Array(Box<GoType>, u32),
    Map(Box<GoType>, Box<GoType>),
    Struct(std::string::String),
    Pointer(Box<GoType>),
    Interface,
    Complex64,
    Complex128,
    Func,

    Void,
    UntypedInt,
    UntypedFloat,
}

impl GoType {
    pub(crate) fn wasm_type(&self) -> ValType {
        match self {
            GoType::Int64 | GoType::Uint64 | GoType::UntypedInt | GoType::Func => ValType::I64,
            GoType::Float64 | GoType::UntypedFloat => ValType::F64,
            GoType::Float32 => ValType::F32,
            _ => ValType::I32,
        }
    }

    pub(crate) fn is_unsigned(&self) -> bool {
        matches!(self, GoType::Uint8 | GoType::Uint16 | GoType::Uint32 | GoType::Uint64)
    }

    pub(crate) fn is_string(&self) -> bool {
        matches!(self, GoType::String)
    }

    pub(crate) fn is_interface(&self) -> bool {
        matches!(self, GoType::Interface)
    }

    pub(crate) fn is_complex(&self) -> bool {
        matches!(self, GoType::Complex64 | GoType::Complex128)
    }

    pub(crate) fn is_complex64(&self) -> bool {
        matches!(self, GoType::Complex64)
    }

    pub(crate) fn is_integer(&self) -> bool {
        matches!(
            self,
            GoType::Int8
                | GoType::Int16
                | GoType::Int32
                | GoType::Int64
                | GoType::Uint8
                | GoType::Uint16
                | GoType::Uint32
                | GoType::Uint64
                | GoType::UntypedInt
        )
    }

    pub(crate) fn is_float(&self) -> bool {
        matches!(self, GoType::Float32 | GoType::Float64 | GoType::UntypedFloat)
    }

    pub(crate) fn from_type_name(name: &str) -> GoType {
        match name {
            "int" | "int64" => GoType::Int64,
            "int32" | "rune" => GoType::Int32,
            "int16" => GoType::Int16,
            "int8" => GoType::Int8,
            "uint" | "uint64" => GoType::Uint64,
            "uint32" | "uintptr" => GoType::Uint32,
            "uint16" => GoType::Uint16,
            "uint8" | "byte" => GoType::Uint8,
            "float32" => GoType::Float32,
            "float64" => GoType::Float64,
            "bool" => GoType::Bool,
            "string" => GoType::String,
            "complex64" => GoType::Complex64,
            "complex128" => GoType::Complex128,
            _ => GoType::Struct(name.to_string()),
        }
    }

    pub(crate) fn from_val_type(vt: ValType) -> GoType {
        match vt {
            ValType::I64 => GoType::Int64,
            ValType::F64 => GoType::Float64,
            ValType::F32 => GoType::Float32,
            ValType::I32 => GoType::Int32,
            _ => GoType::Int32,
        }
    }
}

pub(crate) struct LocalAlloc {
    pub(crate) params: Vec<(String, ValType)>,
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
    pub(crate) gc_string_locals: HashMap<String, u32>,
    unsigned_vars: std::collections::HashSet<String>,
    map_types: HashMap<String, MapTypeInfo>,
    array_info: HashMap<String, (ValType, u32, i32, u32)>, // (elem_vt, array_length, go_elem_size, go_elem_align)
    nested_array_inner_info: HashMap<String, (ValType, u32)>, // inner (elem_type, inner_length) for [M][N]T
    rune_slices: std::collections::HashSet<String>,
    memory_backed_vars: HashMap<String, (u32, ValType)>,
    pointer_to_struct_vars: std::collections::HashSet<String>,
    pub(crate) func_typed_params: HashMap<String, FuncTypedParamInfo>,
    pub(crate) iface_type_id_locals: HashMap<String, u32>,
}

impl LocalAlloc {
    pub(crate) fn new(params: Vec<(String, ValType)>) -> Self {
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
            gc_string_locals: HashMap::new(),
            unsigned_vars: std::collections::HashSet::new(),
            map_types: HashMap::new(),
            array_info: HashMap::new(),
            nested_array_inner_info: HashMap::new(),
            rune_slices: std::collections::HashSet::new(),
            memory_backed_vars: HashMap::new(),
            pointer_to_struct_vars: std::collections::HashSet::new(),
            func_typed_params: HashMap::new(),
            iface_type_id_locals: HashMap::new(),
        }
    }

    pub(crate) fn param_count(&self) -> u32 {
        self.params.len() as u32
    }

    pub(crate) fn current_scope_id(&self) -> u32 {
        *self.scope_stack.last().expect("scope_stack must never be empty")
    }

    pub(crate) fn push_scope(&mut self) {
        self.scope_depth += 1;
        let id = self.next_scope_id;
        self.next_scope_id += 1;
        self.scope_stack.push(id);
    }

    pub(crate) fn pop_scope(&mut self) {
        self.scope_depth = self.scope_depth.saturating_sub(1);
        if self.scope_stack.len() > 1 {
            self.scope_stack.pop();
        }
    }

    pub(crate) fn add_local(&mut self, name: &str, vt: ValType) -> u32 {
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

    pub(crate) fn find_val_type(&self, name: &str) -> Option<ValType> {
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

    pub(crate) fn find_at_current_scope(&self, name: &str) -> Option<u32> {
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

    pub(crate) fn find_type(&self, name: &str) -> Option<ValType> {
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

    pub(crate) fn local_types(&self) -> Vec<(u32, ValType)> {
        self.locals.iter().map(|(_, vt, _)| (1, *vt)).collect()
    }

    pub(crate) fn set_var_struct_type(&mut self, name: &str, type_name: &str) {
        self.var_types.insert(name.to_string(), type_name.to_string());
    }

    pub(crate) fn get_var_struct_type(&self, name: &str) -> Option<&str> {
        self.var_types.get(name).map(|s| s.as_str())
    }

    pub(crate) fn all_entries(&self) -> Vec<(String, ValType)> {
        self.params
            .iter()
            .cloned()
            .chain(self.locals.iter().map(|(n, vt, _)| (n.clone(), *vt)))
            .collect()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GcBuiltinTypes {
    pub(crate) byte_array: Option<u32>,
    pub(crate) go_string: Option<u32>,
    pub(crate) complex64: Option<u32>,
    pub(crate) complex128: Option<u32>,
    pub(crate) go_iface: Option<u32>,
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
    stack_ptr_global: u32,
    panicking_global: u32,
    panic_value_ptr_global: u32,
    panic_value_len_global: u32,
    map_iter_counter_global: u32,
    oom_func_idx: u32,
    wasm_imports: HashMap<String, u32>,

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
    last_iface_call_returns_iface: bool,
    pending_closures: Vec<(u32, Function)>,
    constants: HashMap<String, ConstValue>,
    constant_types: HashMap<String, String>,
    global_vars: HashMap<String, (u32, ValType)>,
    global_array_elem_types: HashMap<String, (ValType, i32, u32)>,
    global_slice_elem_struct_types: HashMap<String, String>,
    global_var_struct_types: HashMap<String, String>,
    current_iota: Option<i128>,
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
    next_anon_iface_id: u32,

    // init() function support
    init_func_indices: Vec<u32>,
    start_func_idx: Option<u32>,

    // Deferred global variable initializers (non-constant or string expressions)
    global_var_inits: Vec<(String, ast::Expression, ValType, Option<String>)>,

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

    // Goto dispatch state
    goto_target_local: Option<u32>,
    goto_label_segments: HashMap<String, u32>,
    goto_segment_depth: u32,

    // call_indirect / range-over-func support
    needs_func_table: bool,
    iter_func_info: HashMap<String, IterFuncInfo>,

    // Current function's stack frame info for escape-analysis-driven stack allocation
    current_stack_frame: Option<StackFrameInfo>,
    // When set, the next allocation should use the stack frame slot for this variable
    stack_alloc_target: Option<String>,

    // Stdlib package compilation
    current_package: Option<String>,
    compiled_packages: HashSet<String>,

    // Functions forward-declared before their bodies are compiled
    forward_declared: HashSet<String>,

    // Buffered code section entries: (func_idx, Function body)
    // Sorted by func_idx before writing to code_section in build_module
    code_buffer: Vec<(u32, Function)>,

    // WasmGC support: maps Go struct names to their WasmGC type indices
    gc_struct_types: HashMap<String, u32>,
    // WasmGC support: maps element ValType to array type index
    gc_array_types: HashMap<ValType, u32>,
    // WasmGC support: per-element-type slice struct type indices
    gc_slice_types: HashMap<ValType, u32>,
    // WasmGC support: builtin GC type indices
    gc_builtin_types: GcBuiltinTypes,
    // WasmGC support: closure env GC type indices
    gc_closure_env_types: Vec<u32>,
}


mod analysis;
mod builtins;
mod calls;
mod declarations;
mod emit;
mod expressions;
mod functions;
mod interfaces;
mod maps;
mod memory;
mod statements;
mod strings;
mod type_system;

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
            stack_ptr_global: 0,
            panicking_global: 0,
            panic_value_ptr_global: 0,
            panic_value_len_global: 0,
            map_iter_counter_global: 0,
            oom_func_idx: 0,
            wasm_imports: HashMap::new(),

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
            last_iface_call_returns_iface: false,
            pending_closures: Vec::new(),
            constants: HashMap::new(),
            constant_types: HashMap::new(),
            global_vars: HashMap::new(),
            global_array_elem_types: HashMap::new(),
            global_slice_elem_struct_types: HashMap::new(),
            global_var_struct_types: HashMap::new(),
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
            next_anon_iface_id: 0,

            init_func_indices: Vec::new(),
            start_func_idx: None,

            global_var_inits: Vec::new(),
            global_func_vars: HashMap::new(),

            generic_funcs: HashMap::new(),
            generic_types: HashMap::new(),
            monomorphized: HashMap::new(),
            struct_field_map_types: HashMap::new(),
            goto_target_local: None,
            goto_label_segments: HashMap::new(),
            goto_segment_depth: 0,
            needs_func_table: false,
            iter_func_info: HashMap::new(),
            current_stack_frame: None,
            stack_alloc_target: None,

            current_package: None,
            compiled_packages: HashSet::new(),
            forward_declared: HashSet::new(),
            code_buffer: Vec::new(),

            gc_struct_types: HashMap::new(),
            gc_array_types: HashMap::new(),
            gc_slice_types: HashMap::new(),
            gc_builtin_types: GcBuiltinTypes::default(),
            gc_closure_env_types: Vec::new(),
        }
    }

    pub(crate) fn pkg(&self) -> &str {
        self.current_package.as_deref().unwrap_or("")
    }

    pub(crate) fn define_var(&mut self, name: &str, dt: DefineType) {
        self.symbols.define(
            self.current_package.as_deref().unwrap_or(""),
            name,
            DefineType::Qualified(Qualifier::Var, Box::new(dt)),
            false,
        );
    }

    pub(crate) fn resolve_var_type(&self, name: &str) -> Option<DefineType> {
        let pkg = self.current_package.as_deref().unwrap_or("");
        self.symbols.resolve_type(pkg, name).map(|dt| dt.unwrap_to_base_type())
    }

    pub(crate) fn is_string_var(&self, name: &str) -> bool {
        matches!(self.resolve_var_type(name), Some(DefineType::String))
    }

    pub(crate) fn is_sym_interface_var(&self, name: &str) -> bool {
        matches!(self.resolve_var_type(name), Some(DefineType::Interface { .. }))
    }

    pub(crate) fn is_unsigned_var(&self, name: &str) -> bool {
        matches!(
            self.resolve_var_type(name),
            Some(DefineType::Uint | DefineType::Uint8 | DefineType::Uint16 |
                 DefineType::Uint32 | DefineType::Uint64 | DefineType::Uintptr |
                 DefineType::Byte)
        )
    }

    pub(crate) fn is_slice_var(&self, name: &str) -> bool {
        matches!(self.resolve_var_type(name), Some(DefineType::Slice(_)))
    }

    pub(crate) fn is_map_var(&self, name: &str) -> bool {
        matches!(self.resolve_var_type(name), Some(DefineType::Map(_, _)))
    }

    pub(crate) fn is_array_var(&self, name: &str) -> bool {
        matches!(self.resolve_var_type(name), Some(DefineType::Array { .. }))
    }

    pub(crate) fn is_context_var(&self, name: &str) -> bool {
        matches!(
            self.resolve_var_type(name),
            Some(DefineType::Struct { ref name, .. }) if name == "Context"
        )
    }

    pub(crate) fn is_complex64_var(&self, name: &str) -> bool {
        matches!(self.resolve_var_type(name), Some(DefineType::Complex64))
    }

    pub(crate) fn is_complex_var(&self, name: &str) -> bool {
        matches!(
            self.resolve_var_type(name),
            Some(DefineType::Complex64 | DefineType::Complex128)
        )
    }

    pub(crate) fn get_struct_var_type(&self, name: &str) -> Option<String> {
        match self.resolve_var_type(name) {
            Some(DefineType::Struct { name: sname, .. }) => Some(sname),
            Some(DefineType::Spec { name: sname, .. }) => Some(sname),
            _ => None,
        }
    }

    pub(crate) fn get_iface_var_name(&self, name: &str) -> Option<String> {
        match self.resolve_var_type(name) {
            Some(DefineType::Interface { name: iname, .. }) => Some(iname),
            _ => None,
        }
    }

    pub(crate) fn infer_define_type_from_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> Option<DefineType> {
        match expr {
            ast::Expression::BasicLit(lit) => match lit.kind {
                LitKind::String => Some(DefineType::String),
                LitKind::Integer => Some(DefineType::Int),
                LitKind::Float => Some(DefineType::Float64),
                LitKind::Imag => Some(DefineType::Complex128),
                _ => None,
            },
            ast::Expression::Ident(id) => {
                self.resolve_var_type(&id.name)
            }
            ast::Expression::CompositeLit(comp) => self.expr_to_define_type(&comp.typ),
            ast::Expression::Operation(op) if op.op == Operator::And && op.y.is_none() => {
                let inner = self.infer_define_type_from_expr(&op.x, locals)?;
                Some(DefineType::Ref(Box::new(inner)))
            }
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(fn_id) = call.func.as_ref() {
                    match fn_id.name.as_str() {
                        "make" => {
                            if let Some(type_arg) = call.args.first() {
                                self.expr_to_define_type(type_arg)
                            } else {
                                None
                            }
                        }
                        "append" => Some(DefineType::Slice(Box::new(DefineType::Null))),
                        "new" => {
                            if let Some(type_arg) = call.args.first() {
                                let inner = self.expr_to_define_type(type_arg).unwrap_or(DefineType::Null);
                                Some(DefineType::Ref(Box::new(inner)))
                            } else {
                                None
                            }
                        }
                        "complex" => Some(DefineType::Complex128),
                        _ => {
                            let go_type = self.go_type_name_to_define_type(&fn_id.name);
                            if !matches!(go_type, DefineType::Struct { .. }) {
                                Some(go_type)
                            } else {
                                None
                            }
                        }
                    }
                } else if let ast::Expression::TypeSlice(_) = call.func.as_ref() {
                    self.expr_to_define_type(call.func.as_ref())
                } else if let ast::Expression::TypeArray(_) = call.func.as_ref() {
                    self.expr_to_define_type(call.func.as_ref())
                } else {
                    None
                }
            }
            ast::Expression::Slice(sl) => {
                if let ast::Expression::Ident(src) = &*sl.left {
                    if self.is_string_var(&src.name) {
                        return Some(DefineType::String);
                    }
                }
                Some(DefineType::Slice(Box::new(DefineType::Null)))
            }
            ast::Expression::TypeAssert(ta) => {
                if let Some(ref target) = ta.right {
                    self.expr_to_define_type(target)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub(crate) fn expr_to_define_type(&self, expr: &ast::Expression) -> Option<DefineType> {
        match expr {
            ast::Expression::Ident(id) => Some(self.go_type_name_to_define_type(&id.name)),
            ast::Expression::TypeSlice(sl) => {
                let inner = self.expr_to_define_type(&sl.typ).unwrap_or(DefineType::Null);
                Some(DefineType::Slice(Box::new(inner)))
            }
            ast::Expression::TypeArray(arr) => {
                let inner = self.expr_to_define_type(&arr.typ).unwrap_or(DefineType::Null);
                let len = if let ast::Expression::BasicLit(lit) = arr.len.as_ref() {
                    lit.value.parse().unwrap_or(0)
                } else {
                    0
                };
                Some(DefineType::Array { inner_type: Box::new(inner), len })
            }
            ast::Expression::TypePointer(ptr) => {
                let inner = self.expr_to_define_type(&ptr.typ).unwrap_or(DefineType::Null);
                Some(DefineType::Ref(Box::new(inner)))
            }
            ast::Expression::TypeMap(m) => {
                let k = self.expr_to_define_type(&m.key).unwrap_or(DefineType::Null);
                let v = self.expr_to_define_type(&m.val).unwrap_or(DefineType::Null);
                Some(DefineType::Map(Box::new(k), Box::new(v)))
            }
            ast::Expression::Selector(sel) => {
                if let ast::Expression::Ident(pkg) = sel.x.as_ref() {
                    if pkg.name == "context" && sel.sel.name == "Context" {
                        return Some(DefineType::Struct {
                            name: "Context".to_string(),
                            fields: vec![],
                            methods: vec![],
                        });
                    }
                    Some(self.go_type_name_to_define_type(&sel.sel.name))
                } else {
                    None
                }
            }
            ast::Expression::TypeInterface(_) => {
                Some(DefineType::Interface { name: String::new(), methods: vec![] })
            }
            ast::Expression::TypeFunction(_) => None,
            _ => None,
        }
    }

    pub(crate) fn go_type_name_to_define_type(&self, name: &str) -> DefineType {
        match name {
            "int" => DefineType::Int,
            "int8" => DefineType::Int8,
            "int16" => DefineType::Int16,
            "int32" | "rune" => DefineType::Int32,
            "int64" => DefineType::Int64,
            "uint" => DefineType::Uint,
            "uint8" | "byte" => DefineType::Uint8,
            "uint16" => DefineType::Uint16,
            "uint32" | "uintptr" => DefineType::Uint32,
            "uint64" => DefineType::Uint64,
            "float32" => DefineType::Float32,
            "float64" => DefineType::Float64,
            "bool" => DefineType::Bool,
            "string" => DefineType::String,
            "complex64" => DefineType::Complex64,
            "complex128" => DefineType::Complex128,
            "error" => DefineType::Interface { name: "error".to_string(), methods: vec![] },
            "any" => DefineType::Interface { name: "any".to_string(), methods: vec![] },
            _ if self.iface_defs.contains_key(name) => {
                DefineType::Interface { name: name.to_string(), methods: vec![] }
            }
            _ if self.struct_defs.contains_key(name) => {
                DefineType::Struct { name: name.to_string(), fields: vec![], methods: vec![] }
            }
            _ => DefineType::Struct { name: name.to_string(), fields: vec![], methods: vec![] },
        }
    }

    pub(crate) fn get_or_create_type_id(&mut self, type_name: &str) -> u32 {
        if let Some(&id) = self.type_registry.get(type_name) {
            return id;
        }
        let id = self.next_type_id;
        self.next_type_id += 1;
        self.type_registry.insert(type_name.to_string(), id);
        id
    }

    pub(crate) fn register_builtin_types(&mut self) {
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
        let error_result = if let Some(gc_idx) = self.gc_builtin_types.go_string {
            vec![WasmType::Ref(gc_idx)]
        } else {
            vec![WasmType::I32, WasmType::I32]
        };
        error_sigs.insert("Error".to_string(), (vec![], error_result));
        self.iface_method_sigs.insert("error".to_string(), error_sigs);
    }

    pub(crate) fn type_id_for_val_type(vt: ValType) -> &'static str {
        match vt {
            ValType::I32 => "int32",
            ValType::I64 => "int",
            ValType::F32 => "float32",
            ValType::F64 => "float64",
            _ => "int",
        }
    }

    pub(crate) fn val_type_for_type_name(name: &str) -> ValType {
        match name {
            "int" | "int64" | "uint" | "uint64" => ValType::I64,
            "int32" | "uint32" | "int16" | "uint16" | "int8" | "uint8"
            | "byte" | "rune" | "bool" | "uintptr" => ValType::I32,
            "float32" => ValType::F32,
            "float64" => ValType::F64,
            _ => ValType::I32, // structs are pointers (overridden to GC ref when gc_struct_types is populated)
        }
    }

    pub(crate) fn gc_ref_val_type(gc_type_idx: u32) -> ValType {
        ValType::Ref(RefType {
            nullable: true,
            heap_type: HeapType::Concrete(gc_type_idx),
        })
    }

    pub(crate) fn val_type_to_wasm_type(vt: ValType) -> WasmType {
        match vt {
            ValType::I32 => WasmType::I32,
            ValType::I64 => WasmType::I64,
            ValType::F32 => WasmType::F32,
            ValType::F64 => WasmType::F64,
            ValType::Ref(rt) => match rt.heap_type {
                HeapType::Concrete(idx) => WasmType::Ref(idx),
                _ => WasmType::I32,
            },
            _ => WasmType::I32,
        }
    }

    pub(crate) fn gc_val_type_for_struct(&self, name: &str) -> Option<ValType> {
        self.gc_struct_types.get(name).map(|&idx| Self::gc_ref_val_type(idx))
    }

    pub(crate) fn gc_val_type_for_type_name(&self, name: &str) -> ValType {
        if let Some(vt) = self.gc_val_type_for_struct(name) {
            return vt;
        }
        if let Some(&idx) = match name {
            "string" => self.gc_builtin_types.go_string.as_ref(),
            "complex64" => self.gc_builtin_types.complex64.as_ref(),
            "complex128" => self.gc_builtin_types.complex128.as_ref(),
            _ => None,
        } {
            return Self::gc_ref_val_type(idx);
        }
        Self::val_type_for_type_name(name)
    }

    pub(crate) fn get_or_create_gc_array_type(&mut self, elem_vt: ValType) -> u32 {
        if let Some(&idx) = self.gc_array_types.get(&elem_vt) {
            return idx;
        }
        let idx = self.next_type_idx;
        self.next_type_idx += 1;
        let storage = match elem_vt {
            ValType::I32 => StorageType::Val(ValType::I32),
            ValType::I64 => StorageType::Val(ValType::I64),
            ValType::F32 => StorageType::Val(ValType::F32),
            ValType::F64 => StorageType::Val(ValType::F64),
            ValType::Ref(rt) => StorageType::Val(ValType::Ref(rt)),
            _ => StorageType::Val(ValType::I32),
        };
        self.type_section.ty().array(&storage, true);
        self.gc_array_types.insert(elem_vt, idx);
        idx
    }

    pub(crate) fn get_or_create_gc_slice_type(&mut self, elem_vt: ValType) -> (u32, u32) {
        let array_type_idx = self.get_or_create_gc_array_type(elem_vt);
        if let Some(&slice_idx) = self.gc_slice_types.get(&elem_vt) {
            return (slice_idx, array_type_idx);
        }
        let slice_idx = self.next_type_idx;
        self.next_type_idx += 1;
        let fields = vec![
            FieldType { element_type: StorageType::Val(ValType::Ref(RefType { nullable: true, heap_type: HeapType::Concrete(array_type_idx) })), mutable: true },
            FieldType { element_type: StorageType::Val(ValType::I32), mutable: true }, // offset (for sub-slicing)
            FieldType { element_type: StorageType::Val(ValType::I32), mutable: true }, // len
            FieldType { element_type: StorageType::Val(ValType::I32), mutable: true }, // cap
        ];
        self.type_section.ty().struct_(fields);
        self.gc_slice_types.insert(elem_vt, slice_idx);
        (slice_idx, array_type_idx)
    }

    pub(crate) fn register_gc_builtin_types(&mut self) {
        // $ByteArray = (array (mut i8))
        let byte_array_idx = self.next_type_idx;
        self.next_type_idx += 1;
        self.type_section.ty().array(&StorageType::I8, true);
        self.gc_builtin_types.byte_array = Some(byte_array_idx);

        // $GoString = (struct (field (ref null $ByteArray)) (field i32))
        let go_string_idx = self.next_type_idx;
        self.next_type_idx += 1;
        let string_fields = vec![
            FieldType { element_type: StorageType::Val(ValType::Ref(RefType { nullable: true, heap_type: HeapType::Concrete(byte_array_idx) })), mutable: true },
            FieldType { element_type: StorageType::Val(ValType::I32), mutable: true },
        ];
        self.type_section.ty().struct_(string_fields);
        self.gc_builtin_types.go_string = Some(go_string_idx);

        // $Complex64 = (struct (field f32) (field f32))
        let complex64_idx = self.next_type_idx;
        self.next_type_idx += 1;
        let c64_fields = vec![
            FieldType { element_type: StorageType::Val(ValType::F32), mutable: true },
            FieldType { element_type: StorageType::Val(ValType::F32), mutable: true },
        ];
        self.type_section.ty().struct_(c64_fields);
        self.gc_builtin_types.complex64 = Some(complex64_idx);

        // $Complex128 = (struct (field f64) (field f64))
        let complex128_idx = self.next_type_idx;
        self.next_type_idx += 1;
        let c128_fields = vec![
            FieldType { element_type: StorageType::Val(ValType::F64), mutable: true },
            FieldType { element_type: StorageType::Val(ValType::F64), mutable: true },
        ];
        self.type_section.ty().struct_(c128_fields);
        self.gc_builtin_types.complex128 = Some(complex128_idx);

        // $GoIface = (struct (field i32) (field (ref null any)))
        let go_iface_idx = self.next_type_idx;
        self.next_type_idx += 1;
        let iface_fields = vec![
            FieldType { element_type: StorageType::Val(ValType::I32), mutable: true },
            FieldType { element_type: StorageType::Val(ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Abstract { shared: false, ty: wasm_encoder::AbstractHeapType::Any },
            })), mutable: true },
        ];
        self.type_section.ty().struct_(iface_fields);
        self.gc_builtin_types.go_iface = Some(go_iface_idx);
    }

    pub(crate) fn emit_gc_struct_rec_group(&mut self) {
        let struct_names: Vec<String> = self.struct_defs.keys().cloned().collect();
        if struct_names.is_empty() {
            return;
        }

        let base_idx = self.next_type_idx;
        let mut name_to_gc_idx: HashMap<String, u32> = HashMap::new();
        for (i, name) in struct_names.iter().enumerate() {
            name_to_gc_idx.insert(name.clone(), base_idx + i as u32);
        }

        fn wasm_type_to_storage(wt: WasmType) -> StorageType {
            match wt {
                WasmType::I32 => StorageType::Val(ValType::I32),
                WasmType::I64 => StorageType::Val(ValType::I64),
                WasmType::F32 => StorageType::Val(ValType::F32),
                WasmType::F64 => StorageType::Val(ValType::F64),
                WasmType::Ref(idx) => StorageType::Val(WasmCompiler::gc_ref_val_type(idx)),
            }
        }

        let subtypes: Vec<SubType> = struct_names.iter().map(|name| {
            let sd = self.struct_defs.get(name).unwrap();
            let fields: Vec<FieldType> = sd.fields.iter().map(|f| {
                let storage = if let Some(ref tag) = f.go_type_tag {
                    if let Some(&gc_idx) = name_to_gc_idx.get(tag.as_str()) {
                        StorageType::Val(Self::gc_ref_val_type(gc_idx))
                    } else {
                        wasm_type_to_storage(f.wasm_type)
                    }
                } else {
                    wasm_type_to_storage(f.wasm_type)
                };
                FieldType { element_type: storage, mutable: true }
            }).collect();

            SubType {
                is_final: true,
                supertype_idx: None,
                composite_type: CompositeType {
                    inner: CompositeInnerType::Struct(StructType {
                        fields: fields.into(),
                    }),
                    shared: false,
                    descriptor: None,
                    describes: None,
                },
            }
        }).collect();

        let count = subtypes.len() as u32;
        self.type_section.ty().rec(subtypes);
        self.next_type_idx += count;

        for name in &struct_names {
            let gc_idx = name_to_gc_idx[name];
            self.gc_struct_types.insert(name.clone(), gc_idx);
            if let Some(sd) = self.struct_defs.get_mut(name) {
                sd.gc_type_idx = Some(gc_idx);
                for f in &mut sd.fields {
                    if let Some(ref tag) = f.go_type_tag {
                        if let Some(&ref_gc_idx) = name_to_gc_idx.get(tag.as_str()) {
                            f.wasm_type = WasmType::Ref(ref_gc_idx);
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn register_gc_struct_type_late(&mut self, name: &str) {
        if self.gc_struct_types.contains_key(name) {
            return;
        }
        let sd = match self.struct_defs.get(name).cloned() {
            Some(sd) => sd,
            None => return,
        };

        let gc_idx = self.next_type_idx;
        self.next_type_idx += 1;

        let gc_fields: Vec<FieldType> = sd.fields.iter().map(|f| {
            let storage = if let Some(ref tag) = f.go_type_tag {
                if let Some(&ref_gc_idx) = self.gc_struct_types.get(tag.as_str()) {
                    StorageType::Val(Self::gc_ref_val_type(ref_gc_idx))
                } else {
                    match f.wasm_type {
                        WasmType::I32 => StorageType::Val(ValType::I32),
                        WasmType::I64 => StorageType::Val(ValType::I64),
                        WasmType::F32 => StorageType::Val(ValType::F32),
                        WasmType::F64 => StorageType::Val(ValType::F64),
                        WasmType::Ref(idx) => StorageType::Val(Self::gc_ref_val_type(idx)),
                    }
                }
            } else {
                match f.wasm_type {
                    WasmType::I32 => StorageType::Val(ValType::I32),
                    WasmType::I64 => StorageType::Val(ValType::I64),
                    WasmType::F32 => StorageType::Val(ValType::F32),
                    WasmType::F64 => StorageType::Val(ValType::F64),
                    WasmType::Ref(idx) => StorageType::Val(Self::gc_ref_val_type(idx)),
                }
            };
            FieldType { element_type: storage, mutable: true }
        }).collect();

        self.type_section.ty().struct_(gc_fields);
        self.gc_struct_types.insert(name.to_string(), gc_idx);

        if let Some(sd) = self.struct_defs.get_mut(name) {
            sd.gc_type_idx = Some(gc_idx);
            for f in &mut sd.fields {
                if let Some(ref tag) = f.go_type_tag {
                    if let Some(&ref_gc_idx) = self.gc_struct_types.get(tag.as_str()) {
                        f.wasm_type = WasmType::Ref(ref_gc_idx);
                    }
                }
            }
        }
    }

    pub(crate) fn parse_go_int(s: &str) -> Result<i128, String> {
        let s = s.replace('_', "");
        if let Some(digits) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
            i128::from_str_radix(digits, 2)
                .or_else(|_| u128::from_str_radix(digits, 2).map(|v| v as i128))
                .map_err(|e| format!("invalid binary literal '{}': {}", s, e))
        } else if let Some(digits) = s.strip_prefix("0o").or_else(|| s.strip_prefix("0O")) {
            i128::from_str_radix(digits, 8)
                .or_else(|_| u128::from_str_radix(digits, 8).map(|v| v as i128))
                .map_err(|e| format!("invalid octal literal '{}': {}", s, e))
        } else if let Some(digits) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            i128::from_str_radix(digits, 16)
                .or_else(|_| u128::from_str_radix(digits, 16).map(|v| v as i128))
                .map_err(|e| format!("invalid hex literal '{}': {}", s, e))
        } else if s.starts_with('0') && s.len() > 1 && s[1..].chars().all(|c| c.is_ascii_digit()) {
            i128::from_str_radix(&s[1..], 8)
                .or_else(|_| u128::from_str_radix(&s[1..], 8).map(|v| v as i128))
                .map_err(|e| format!("invalid octal literal '{}': {}", s, e))
        } else {
            s.parse::<i128>()
                .or_else(|_| s.parse::<u128>().map(|v| v as i128))
                .map_err(|e| format!("invalid integer literal '{}': {}", s, e))
        }
    }

    pub(crate) fn is_integer_representable_float(s: &str) -> Option<i64> {
        let cleaned = s.replace('_', "");
        if cleaned.starts_with("0x") || cleaned.starts_with("0X") {
            return None;
        }
        let val: f64 = cleaned.parse().ok()?;
        if val.fract() != 0.0 || val.is_nan() || val.is_infinite() {
            return None;
        }
        if val < (i64::MIN as f64) || val > (i64::MAX as f64) {
            return None;
        }
        let ival = val as i64;
        if (ival as f64) == val {
            Some(ival)
        } else {
            None
        }
    }

    pub(crate) fn parse_go_float(s: &str) -> Result<f64, String> {
        let s = s.replace('_', "");
        if s.starts_with("0x") || s.starts_with("0X") {
            Self::parse_hex_float(&s)
        } else {
            s.parse::<f64>()
                .map_err(|e| format!("invalid float literal '{}': {}", s, e))
        }
    }

    pub(crate) fn parse_hex_float(s: &str) -> Result<f64, String> {
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

    pub(crate) fn qualify_pkg_name(&self, name: &str) -> String {
        if let Some(ref pkg) = self.current_package {
            format!("{}.{}", pkg, name)
        } else {
            name.to_string()
        }
    }

    pub(crate) fn find_func_in_pkg(&self, name: &str) -> Option<&FuncInfo> {
        if let Some(fi) = self.functions.iter().find(|f| f.name == name) {
            return Some(fi);
        }
        if let Some(ref pkg) = self.current_package {
            let qualified = format!("{}.{}", pkg, name);
            return self.functions.iter().find(|f| f.name == qualified);
        }
        None
    }

    pub(crate) fn resolve_struct_in_pkg(&self, name: &str) -> String {
        if self.struct_defs.contains_key(name) {
            return name.to_string();
        }
        if let Some(ref pkg) = self.current_package {
            let qualified = format!("{}.{}", pkg, name);
            if self.struct_defs.contains_key(&qualified) {
                return qualified;
            }
        }
        for pkg in &self.compiled_packages {
            if pkg.starts_with("__compiled_") {
                continue;
            }
            let qualified = format!("{}.{}", pkg, name);
            if self.struct_defs.contains_key(&qualified) {
                return qualified;
            }
        }
        name.to_string()
    }

    pub(crate) fn resolve_global_var(&self, name: &str) -> Option<&(u32, ValType)> {
        if let Some(entry) = self.global_vars.get(name) {
            return Some(entry);
        }
        if let Some(ref pkg) = self.current_package {
            let qualified = format!("{}.{}", pkg, name);
            return self.global_vars.get(&qualified);
        }
        None
    }

    pub(crate) fn resolve_global_var_name(&self, name: &str) -> String {
        if self.global_vars.contains_key(name) {
            return name.to_string();
        }
        if let Some(ref pkg) = self.current_package {
            let qualified = format!("{}.{}", pkg, name);
            if self.global_vars.contains_key(&qualified) {
                return qualified;
            }
        }
        name.to_string()
    }

    pub(crate) fn pkg_short_name(pkg: &str) -> &str {
        pkg.rsplit('/').next().unwrap_or(pkg)
    }

    pub fn compile_source(&mut self, source: &str) -> Result<CompileResult, Error> {
        let file = crate::parser::parse_source(source)
            .map_err(|e| Error::SyntaxError(e.to_string()))?;
        self.compile_file(&file)
    }

    pub fn compile_file(&mut self, file: &ast::File) -> Result<CompileResult, Error> {
        self.register_gc_builtin_types();
        self.emit_memory();
        self.emit_heap_globals();
        self.emit_host_imports();
        self.emit_native_imports(file)?;
        self.emit_alloc_function();
        self.emit_reset_function();
        self.emit_gc_string_bridge_function();
        self.register_builtin_types();

        // Phase 1: Prescan all types (stdlib + user) and forward-declare all functions.
        for imp in &file.imports {
            let path = imp.path.value.trim_matches('"');
            if let Ok(crate::wasm::stdlib::ImportKind::Stdlib(pkg)) =
                crate::wasm::stdlib::resolve_import(path)
            {
                if let Some(sources) = crate::wasm::stdlib::get_stdlib_sources(&pkg) {
                    self.prescan_stdlib_package(&pkg, sources)?;
                }
            }
        }
        self.prescan_type_declarations(file);
        self.emit_gc_struct_rec_group();
        let sorted_decls = Self::sort_declarations_by_deps(&file.decl);
        self.forward_declare_functions(&sorted_decls);

        // Phase 2: Compile stdlib function bodies.
        for imp in &file.imports {
            let path = imp.path.value.trim_matches('"');
            if let Ok(crate::wasm::stdlib::ImportKind::Stdlib(pkg)) =
                crate::wasm::stdlib::resolve_import(path)
            {
                if let Some(sources) = crate::wasm::stdlib::get_stdlib_sources(&pkg) {
                    self.compile_stdlib_bodies(&pkg, sources)?;
                }
            }
        }

        // Phase 3: Compile user declarations.
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

    pub(crate) fn build_manifest(&mut self, file: &ast::File) -> Result<(), Error> {
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

    pub(crate) fn emit_aggregate_init(&mut self, name: &str, struct_size: u32) -> Result<(), Error> {
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

        self.code_buffer.push((func_idx, func));
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

    pub(crate) fn extract_input_fields(
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

    pub(crate) fn extract_output(
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

    pub(crate) fn expr_type_name(&self, expr: &ast::Expression) -> String {
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

    pub(crate) fn build_module(&mut self) -> Vec<u8> {
        // Sort buffered code entries by func_idx to match function_section order
        self.code_buffer.sort_by_key(|(idx, _)| *idx);
        for (_, func) in &self.code_buffer {
            self.code_section.function(func);
        }

        if self.needs_func_table && self.next_func_idx > 0 {
            self.table_section.table(TableType {
                element_type: RefType::FUNCREF,
                minimum: self.next_func_idx as u64,
                maximum: Some(self.next_func_idx as u64),
                table64: false,
                shared: false,
            });
            let func_indices: Vec<u32> = (0..self.next_func_idx).collect();
            self.element_section.active(
                Some(0),
                &ConstExpr::i32_const(0),
                Elements::Functions(func_indices.into()),
            );
        }

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
