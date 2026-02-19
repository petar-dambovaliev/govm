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
struct FuncInfo {
    wasm_func_idx: u32,
    type_idx: u32,
    name: String,
    params: Vec<(String, WasmType)>,
    results: Vec<WasmType>,
    is_exported: bool,
    recv_type: Option<String>,
}

struct DeferredCall {
    func_idx: u32,
    arg_locals: Vec<(u32, ValType)>,
}

#[derive(Debug, Clone)]
struct StructDef {
    fields: Vec<StructFieldDef>,
    total_size: u32,
}

#[derive(Debug, Clone)]
struct StructFieldDef {
    name: String,
    wasm_type: WasmType,
    offset: u32,
}

impl StructDef {
    fn find_field(&self, name: &str) -> Option<&StructFieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }
}

struct CapturedVar {
    name: String,
    val_type: ValType,
    outer_local_idx: u32,
    env_offset: u32,
}

struct ClosureCaptureState {
    outer_locals: Vec<(String, ValType)>,
    captures: Vec<CapturedVar>,
}

struct LocalAlloc {
    params: Vec<(String, ValType)>,
    locals: Vec<(String, ValType)>,
    var_types: HashMap<String, String>,
    closure_info: HashMap<String, (u32, u32)>,
    slice_elem_types: HashMap<String, ValType>,
    string_locals: HashMap<String, (u32, u32)>,
}

impl LocalAlloc {
    fn new(params: Vec<(String, ValType)>) -> Self {
        Self {
            params,
            locals: Vec::new(),
            var_types: HashMap::new(),
            closure_info: HashMap::new(),
            slice_elem_types: HashMap::new(),
            string_locals: HashMap::new(),
        }
    }

    fn param_count(&self) -> u32 {
        self.params.len() as u32
    }

    fn add_local(&mut self, name: &str, vt: ValType) -> u32 {
        let idx = self.param_count() + self.locals.len() as u32;
        self.locals.push((name.to_string(), vt));
        idx
    }

    fn find(&self, name: &str) -> Option<u32> {
        for (i, (n, _)) in self.params.iter().enumerate() {
            if n == name {
                return Some(i as u32);
            }
        }
        for (i, (n, _)) in self.locals.iter().enumerate() {
            if n == name {
                return Some(self.param_count() + i as u32);
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
        for (n, vt) in &self.locals {
            if n == name {
                return Some(*vt);
            }
        }
        None
    }

    fn local_types(&self) -> Vec<(u32, ValType)> {
        self.locals.iter().map(|(_, vt)| (1, *vt)).collect()
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
            .chain(self.locals.iter())
            .cloned()
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

    functions: Vec<FuncInfo>,
    deferred_calls: Vec<Vec<DeferredCall>>,
    loop_depth: Vec<u32>,
    manifest: Manifest,
    struct_defs: HashMap<String, StructDef>,
    closure_captures: Option<ClosureCaptureState>,
    last_closure_func_idx: Option<u32>,
    last_closure_env: Option<u32>,
    pending_closures: Vec<Function>,
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

            functions: Vec::new(),
            deferred_calls: Vec::new(),
            loop_depth: Vec::new(),
            manifest: Manifest::new(),
            struct_defs: HashMap::new(),
            closure_captures: None,
            last_closure_func_idx: None,
            last_closure_env: None,
            pending_closures: Vec::new(),
        }
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

        for decl in &file.decl {
            self.compile_declaration(decl)?;
        }

        self.build_manifest(file);

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
        ];

        for (name, params, results) in pairs {
            let type_idx = self.next_type_idx;
            self.type_section.ty().function(
                params.iter().copied().collect::<Vec<_>>(),
                results.iter().copied().collect::<Vec<_>>(),
            );
            self.next_type_idx += 1;

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
            is_exported: true,
            recv_type: None,
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
            is_exported: true,
            recv_type: None,
        });
    }

    fn alloc_func_idx(&self) -> u32 {
        self.functions
            .iter()
            .find(|f| f.name == "alloc")
            .unwrap()
            .wasm_func_idx
    }

    fn compile_declaration(&mut self, decl: &ast::Declaration) -> Result<(), Error> {
        match decl {
            ast::Declaration::Function(func_decl) => self.compile_func_decl(func_decl),
            ast::Declaration::Variable(var_decl) => {
                for spec in &var_decl.specs {
                    self.compile_global_var(spec)?;
                }
                Ok(())
            }
            ast::Declaration::Const(const_decl) => {
                for spec in &const_decl.specs {
                    self.compile_global_const(spec)?;
                }
                Ok(())
            }
            ast::Declaration::Type(type_decl) => {
                for spec in &type_decl.specs {
                    if let ast::Expression::TypeStruct(struct_type) = &spec.typ {
                        let struct_def = self.compute_struct_def(&struct_type.fields);
                        self.struct_defs
                            .insert(spec.name.name.clone(), struct_def);
                    }
                }
                Ok(())
            }
        }
    }

    fn compute_struct_def(&self, fields: &[ast::Field]) -> StructDef {
        let mut result_fields = Vec::new();
        let mut offset: u32 = 0;

        for field in fields {
            let wasm_types = self.field_to_wasm_types(field);
            let names: Vec<String> = if field.name.is_empty() {
                vec!["".to_string()]
            } else {
                field.name.iter().map(|n| n.name.clone()).collect()
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
                        });
                        offset += size;
                    }
                }
            }
        }

        let align = 8u32;
        let total_size = ((offset + align - 1) & !(align - 1)).max(8);

        StructDef {
            fields: result_fields,
            total_size,
        }
    }

    fn compile_global_var(&mut self, spec: &ast::VarSpec) -> Result<(), Error> {
        let names: Vec<&str> = spec.name.iter().map(|n| n.name.as_str()).collect();
        Err(Error::InternalError(format!(
            "global variables are not supported in WASM UDFs: {}",
            names.join(", ")
        )))
    }

    fn compile_global_const(&mut self, spec: &ast::ConstSpec) -> Result<(), Error> {
        let names: Vec<&str> = spec.name.iter().map(|n| n.name.as_str()).collect();
        Err(Error::InternalError(format!(
            "global constants are not supported in WASM UDFs: {}",
            names.join(", ")
        )))
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
            _ => None,
        })
    }

    fn compile_func_decl(&mut self, decl: &ast::FuncDecl) -> Result<(), Error> {
        let name = &decl.name.name;
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
                param_types.push(ValType::I32);
                let recv_name = field.name.first().map_or("self", |id| &id.name);
                param_names.push(recv_name.to_string());
            }
        }

        for field in &decl.typ.params.list {
            let field_wasm_types = self.field_to_wasm_types(field);
            if field.name.is_empty() {
                for wt in &field_wasm_types {
                    param_types.push(wt.to_val_type());
                    param_names.push(format!("_param{}", param_names.len()));
                }
            } else {
                for (i, ident) in field.name.iter().enumerate() {
                    if i < field_wasm_types.len() {
                        param_types.push(field_wasm_types[i].to_val_type());
                    } else if !field_wasm_types.is_empty() {
                        param_types.push(field_wasm_types[0].to_val_type());
                    } else {
                        param_types.push(ValType::I32);
                    }
                    param_names.push(ident.name.clone());
                }
            }
        }

        let mut result_types: Vec<ValType> = Vec::new();
        for field in &decl.typ.result.list {
            let field_wasm_types = self.field_to_wasm_types(field);
            for wt in &field_wasm_types {
                result_types.push(wt.to_val_type());
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

        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: internal_name,
            params: wasm_params,
            results: wasm_results,
            is_exported,
            recv_type: recv_type_name.clone(),
        });

        let param_entries: Vec<(String, ValType)> = param_names
            .iter()
            .zip(param_types.iter())
            .map(|(n, vt)| (n.clone(), *vt))
            .collect();
        let mut locals = LocalAlloc::new(param_entries);

        // Track struct type for receiver
        if let Some(recv) = &decl.recv {
            for field in &recv.list {
                let recv_name = field.name.first().map_or("self", |id| &id.name);
                if let Some(ref rtn) = recv_type_name {
                    locals.set_var_struct_type(recv_name, rtn);
                }
            }
        }

        // Track struct types for parameters
        for field in &decl.typ.params.list {
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
                    }
                }
            }
            if let ast::Expression::TypePointer(ptr) = &field.typ {
                if let ast::Expression::Ident(type_ident) = ptr.typ.as_ref() {
                    if self.struct_defs.contains_key(&type_ident.name) {
                        for name_ident in &field.name {
                            locals.set_var_struct_type(&name_ident.name, &type_ident.name);
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

        self.deferred_calls.push(Vec::new());

        let mut func_body: Vec<Instruction<'static>> = Vec::new();

        if let Some(body) = &decl.body {
            self.compile_block(&body, &mut func_body, &mut locals, &result_types)?;
        }

        self.emit_deferred_calls(&mut func_body);
        self.deferred_calls.pop();

        if result_types.is_empty()
            || func_body
                .last()
                .map_or(true, |i| !matches!(i, Instruction::Return))
        {
            for vt in &result_types {
                match vt {
                    ValType::I32 => func_body.push(Instruction::I32Const(0)),
                    ValType::I64 => func_body.push(Instruction::I64Const(0)),
                    ValType::F32 => func_body.push(Instruction::F32Const(0.0)),
                    ValType::F64 => func_body.push(Instruction::F64Const(0.0)),
                    _ => func_body.push(Instruction::I32Const(0)),
                }
            }
        }

        func_body.push(Instruction::End);

        let mut func = Function::new(locals.local_types());
        for instr in &func_body {
            func.instruction(instr);
        }

        self.code_section.function(&func);

        for closure_func in self.pending_closures.drain(..) {
            self.code_section.function(&closure_func);
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
        for stmt in &block.list {
            self.compile_statement(stmt, out, locals, result_types)?;
        }
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
                self.compile_expression(&expr_stmt.expr, out, locals)?;
                let wasm_types = self.expression_result_count(&expr_stmt.expr);
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

                let func_idx =
                    if let ast::Expression::Ident(ident) = defer.call.func.as_ref() {
                        self.functions
                            .iter()
                            .find(|f| f.name == ident.name)
                            .map(|f| f.wasm_func_idx)
                    } else if let ast::Expression::Selector(sel) =
                        defer.call.func.as_ref()
                    {
                        if let ast::Expression::Ident(pkg) = sel.x.as_ref() {
                            let qname = format!("{}.{}", pkg.name, sel.sel.name);
                            self.functions
                                .iter()
                                .find(|f| f.name == qname)
                                .map(|f| f.wasm_func_idx)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                if let Some(idx) = func_idx {
                    if let Some(deferred) = self.deferred_calls.last_mut() {
                        deferred.push(DeferredCall {
                            func_idx: idx,
                            arg_locals,
                        });
                    }
                }
                Ok(())
            }
            ast::Statement::Range(range) => {
                self.compile_range(range, out, locals, result_types)
            }
            ast::Statement::Empty(_) => Ok(()),
            ast::Statement::Label(labeled) => {
                self.compile_statement(&labeled.stmt, out, locals, result_types)
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
            ast::Statement::TypeSwitch(_) => Err(Error::InternalError(
                "type switch not yet supported".to_string(),
            )),
        }
    }

    fn compile_return(
        &mut self,
        ret: &ast::ReturnStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        _result_types: &[ValType],
    ) -> Result<(), Error> {
        for expr in &ret.ret {
            self.compile_expression(expr, out, locals)?;
        }
        self.emit_deferred_calls(out);
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

                    let local_idx = locals.add_local(&ident.name, vt);

                    if i < assign.right.len() {
                        // Track struct type from composite literals
                        if let ast::Expression::CompositeLit(comp) = &assign.right[i] {
                            if let ast::Expression::Ident(type_ident) = comp.typ.as_ref()
                            {
                                locals.set_var_struct_type(
                                    &ident.name,
                                    &type_ident.name,
                                );
                            }
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
                                } else if fn_ident.name == "append" {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        "__slice",
                                    );
                                }
                            }
                        }

                        // Track closure assignments
                        let is_func_lit =
                            matches!(&assign.right[i], ast::Expression::FuncLit(_));

                        self.compile_expression(&assign.right[i], out, locals)?;
                        if is_string {
                            let (ptr_local, len_local) = locals.string_locals[&ident.name];
                            out.push(Instruction::LocalSet(len_local));
                            out.push(Instruction::LocalSet(ptr_local));
                        } else {
                            out.push(Instruction::LocalSet(local_idx));
                        }

                        if is_func_lit {
                            if let Some(func_idx) = self.last_closure_func_idx.take() {
                                let env_local =
                                    self.last_closure_env.take().unwrap_or(u32::MAX);
                                locals
                                    .closure_info
                                    .insert(ident.name.clone(), (func_idx, env_local));
                            }
                        }
                    }
                }
            }
        } else {
            for (i, left) in assign.left.iter().enumerate() {
                if i < assign.right.len() {
                    self.compile_expression(&assign.right[i], out, locals)?;
                }

                match left {
                    ast::Expression::Ident(ident) => {
                        if ident.name == "_" {
                            out.push(Instruction::Drop);
                            continue;
                        }
                        if let Some(&(ptr_local, len_local)) = locals.string_locals.get(&ident.name) {
                            if assign.op == Operator::Assign {
                                out.push(Instruction::LocalSet(len_local));
                                out.push(Instruction::LocalSet(ptr_local));
                            } else {
                                return Err(Error::InternalError(format!(
                                    "compound assignment {:?} not supported on string variables",
                                    assign.op
                                )));
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
                                    let tmp = locals.add_local(
                                        &format!("__ca_tmp_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(idx));
                                    out.push(Instruction::LocalGet(tmp));
                                    out.push(Self::typed_div(vt));
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::RemAssign
                                | Operator::AndAssign
                                | Operator::OrAssign
                                | Operator::XorAssign
                                | Operator::ShlAssign
                                | Operator::ShrAssign => {
                                    let tmp = locals.add_local(
                                        &format!("__ca_tmp_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(idx));
                                    out.push(Instruction::LocalGet(tmp));
                                    self.emit_compound_op(&assign.op, vt, out)?;
                                    out.push(Instruction::LocalSet(idx));
                                }
                                _ => {
                                    out.push(Instruction::LocalSet(idx));
                                }
                            }
                        }
                    }
                    ast::Expression::Index(idx_expr) => {
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
                                Self::emit_typed_coerce(rhs_vt, elem_vt, out);
                                Self::emit_typed_store(elem_vt, 0, align, out);
                            }
                            _ => {
                                out.push(Instruction::LocalGet(addr_tmp));
                                Self::emit_typed_load(elem_vt, 0, align, out);

                                out.push(Instruction::LocalGet(rhs_tmp));
                                Self::emit_typed_coerce(rhs_vt, elem_vt, out);

                                self.emit_compound_op(&assign.op, elem_vt, out)?;

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
                                Self::emit_typed_coerce(rhs_vt, field_vt, out);
                                Self::emit_typed_store(field_vt, offset, align, out);
                            }
                            _ => {
                                out.push(Instruction::LocalGet(addr_tmp));
                                Self::emit_typed_load(field_vt, offset, align, out);

                                out.push(Instruction::LocalGet(rhs_tmp));
                                Self::emit_typed_coerce(rhs_vt, field_vt, out);

                                self.emit_compound_op(&assign.op, field_vt, out)?;

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

    fn typed_div(vt: ValType) -> Instruction<'static> {
        match vt {
            ValType::I32 => Instruction::I32DivS,
            ValType::F32 => Instruction::F32Div,
            ValType::F64 => Instruction::F64Div,
            _ => Instruction::I64DivS,
        }
    }

    fn compile_if(
        &mut self,
        if_stmt: &ast::IfStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        if let Some(init) = &if_stmt.init {
            self.compile_statement(init, out, locals, result_types)?;
        }

        self.compile_expression(&if_stmt.cond, out, locals)?;

        out.push(Instruction::If(BlockType::Empty));
        if let Some(depth) = self.loop_depth.last_mut() {
            *depth += 1;
        }
        self.compile_block(&if_stmt.body, out, locals, result_types)?;

        if let Some(else_) = &if_stmt.else_ {
            out.push(Instruction::Else);
            self.compile_statement(else_, out, locals, result_types)?;
        }

        if let Some(depth) = self.loop_depth.last_mut() {
            *depth -= 1;
        }
        out.push(Instruction::End);
        Ok(())
    }

    fn compile_for(
        &mut self,
        for_stmt: &ast::ForStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        if let Some(init) = &for_stmt.init {
            self.compile_statement(init, out, locals, result_types)?;
        }

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));

        self.loop_depth.push(0);

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

        self.compile_block(&for_stmt.body, out, locals, result_types)?;

        if let Some(post) = &for_stmt.post {
            self.compile_statement(post, out, locals, result_types)?;
        }

        out.push(Instruction::Br(0));
        out.push(Instruction::End);
        out.push(Instruction::End);

        self.loop_depth.pop();

        Ok(())
    }

    fn compile_range(
        &mut self,
        range: &ast::RangeStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        let idx_local = locals.add_local("__range_idx", ValType::I32);
        let len_local = locals.add_local("__range_len", ValType::I32);
        let base_ptr_local = locals.add_local("__range_base", ValType::I32);

        // Check if the range expression is a slice header variable
        let is_slice_header = if let ast::Expression::Ident(ident) = &range.expr {
            locals.get_var_struct_type(&ident.name) == Some("__slice")
        } else {
            false
        };

        self.compile_expression(&range.expr, out, locals)?;

        if is_slice_header {
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
        } else {
            // Check how many values the expression pushed
            let expr_count = self.expression_result_count(&range.expr);
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
                out.push(Instruction::LocalSet(len_local));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(base_ptr_local));
            }
        }

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(idx_local));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));

        self.loop_depth.push(0);

        out.push(Instruction::LocalGet(idx_local));
        out.push(Instruction::LocalGet(len_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        if let Some(key) = &range.key {
            if let ast::Expression::Ident(ident) = key {
                if ident.name != "_" {
                    let key_local = if range
                        .op
                        .as_ref()
                        .map_or(false, |(_, op)| *op == Operator::Define)
                    {
                        locals.add_local(&ident.name, ValType::I32)
                    } else {
                        locals
                            .find(&ident.name)
                            .unwrap_or_else(|| locals.add_local(&ident.name, ValType::I32))
                    };
                    out.push(Instruction::LocalGet(idx_local));
                    out.push(Instruction::LocalSet(key_local));
                }
            }
        }

        if let Some(value) = &range.value {
            if let ast::Expression::Ident(ident) = value {
                if ident.name != "_" {
                    let elem_vt = if let ast::Expression::Ident(slice_ident) = &range.expr {
                        locals
                            .slice_elem_types
                            .get(&slice_ident.name)
                            .copied()
                            .unwrap_or(ValType::I64)
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

        self.compile_block(&range.body, out, locals, result_types)?;

        out.push(Instruction::LocalGet(idx_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(idx_local));

        out.push(Instruction::Br(0));
        out.push(Instruction::End);
        out.push(Instruction::End);

        self.loop_depth.pop();

        Ok(())
    }

    fn compile_switch(
        &mut self,
        switch: &ast::SwitchStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        if let Some(init) = &switch.init {
            self.compile_statement(init, out, locals, result_types)?;
        }

        let (tag_local, tag_vt) = if let Some(tag) = &switch.tag {
            let vt = self.infer_val_type(tag, locals);
            let local = locals.add_local("__switch_tag", vt);
            self.compile_expression(tag, out, locals)?;
            out.push(Instruction::LocalSet(local));
            (Some(local), vt)
        } else {
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

        if non_default_cases.is_empty() {
            if let Some(def) = default_case {
                for stmt in def.body.iter() {
                    self.compile_statement(stmt, out, locals, result_types)?;
                }
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

        for (i, case) in non_default_cases.iter().enumerate() {
            let mut first = true;
            for expr in &case.list {
                if let Some(tag_l) = tag_local {
                    out.push(Instruction::LocalGet(tag_l));
                    self.compile_expression(expr, out, locals)?;
                    let case_vt = self.infer_val_type(expr, locals);
                    if case_vt != tag_vt {
                        match (case_vt, tag_vt) {
                            (ValType::I64, ValType::I32) => {
                                out.push(Instruction::I32WrapI64);
                            }
                            (ValType::I32, ValType::I64) => {
                                out.push(Instruction::I64ExtendI32S);
                            }
                            _ => {}
                        }
                    }
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
            if let Some(depth) = self.loop_depth.last_mut() {
                *depth += 1;
            }

            for stmt in case.body.iter() {
                self.compile_statement(stmt, out, locals, result_types)?;
            }

            let is_last = i == num_cases - 1;
            if !is_last || default_case.is_some() {
                out.push(Instruction::Else);
            } else {
                if let Some(depth) = self.loop_depth.last_mut() {
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
            if let Some(depth) = self.loop_depth.last_mut() {
                *depth -= 1;
            }
            out.push(Instruction::End);
        }

        Ok(())
    }

    fn compile_branch(
        &mut self,
        branch: &ast::BranchStmt,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        let extra = self.loop_depth.last().copied().unwrap_or(0);
        match branch.key {
            Keyword::Break => {
                out.push(Instruction::Br(1 + extra));
            }
            Keyword::Continue => {
                out.push(Instruction::Br(0 + extra));
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
                    for (i, ident) in spec.name.iter().enumerate() {
                        let vt = if let Some(ref typ) = spec.typ {
                            self.expr_to_val_type(typ)
                        } else if i < spec.values.len() {
                            self.infer_val_type(&spec.values[i], locals)
                        } else {
                            ValType::I64
                        };

                        let local_idx = locals.add_local(&ident.name, vt);

                        // Track struct and string types
                        let mut is_string = false;
                        if let Some(ref typ) = spec.typ {
                            if let ast::Expression::Ident(type_ident) = typ {
                                if type_ident.name == "string" {
                                    is_string = true;
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        "__string",
                                    );
                                } else if self.struct_defs.contains_key(&type_ident.name) {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        &type_ident.name,
                                    );
                                }
                            }
                        }
                        if i < spec.values.len() {
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
                            if is_string {
                                let (ptr_local, len_local) = locals.string_locals[&ident.name];
                                out.push(Instruction::LocalSet(len_local));
                                out.push(Instruction::LocalSet(ptr_local));
                            } else {
                                out.push(Instruction::LocalSet(local_idx));
                            }
                        }
                    }
                }
                Ok(())
            }
            ast::DeclStmt::Const(const_decl) => {
                for spec in &const_decl.specs {
                    for (i, ident) in spec.name.iter().enumerate() {
                        let vt = if let Some(ref typ) = spec.typ {
                            self.expr_to_val_type(typ)
                        } else if i < spec.values.len() {
                            self.infer_val_type(&spec.values[i], locals)
                        } else {
                            ValType::I64
                        };

                        let local_idx = locals.add_local(&ident.name, vt);

                        if i < spec.values.len() {
                            self.compile_expression(&spec.values[i], out, locals)?;
                            out.push(Instruction::LocalSet(local_idx));
                        }
                    }
                }
                Ok(())
            }
            ast::DeclStmt::Type(type_decl) => {
                for spec in &type_decl.specs {
                    if let ast::Expression::TypeStruct(struct_type) = &spec.typ {
                        let struct_def = self.compute_struct_def(&struct_type.fields);
                        self.struct_defs
                            .insert(spec.name.name.clone(), struct_def);
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
                out.push(Instruction::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                Ok(())
            }
            ast::Expression::TypeAssert(_) => Err(Error::InternalError(
                "type assertions are not supported in WASM UDFs".to_string(),
            )),
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
            ast::Expression::Range(_) => Ok(()),
            ast::Expression::TypeMap(_)
            | ast::Expression::TypeArray(_)
            | ast::Expression::TypeSlice(_)
            | ast::Expression::TypeFunction(_)
            | ast::Expression::TypeStruct(_)
            | ast::Expression::TypeChannel(_)
            | ast::Expression::TypePointer(_)
            | ast::Expression::TypeInterface(_) => Ok(()),
            _ => Err(Error::InternalError(format!(
                "unsupported expression in WASM compilation: {:?}",
                expr
            ))),
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
                let val: i64 = lit.value.parse().map_err(|_| {
                    Error::SyntaxError(format!("invalid integer literal: {}", lit.value))
                })?;
                out.push(Instruction::I64Const(val));
            }
            LitKind::Float => {
                let val: f64 = lit.value.parse().map_err(|_| {
                    Error::SyntaxError(format!("invalid float literal: {}", lit.value))
                })?;
                out.push(Instruction::F64Const(val));
            }
            LitKind::String => {
                let s = lit.value.trim_matches('"');
                let bytes = Self::unescape_go_string(s);
                let len = bytes.len() as i32;

                out.push(Instruction::I32Const(len));
                out.push(Instruction::Call(self.alloc_func_idx()));

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
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported literal kind: {:?}",
                    lit.kind
                )));
            }
        }
        Ok(())
    }

    fn unescape_go_string(s: &str) -> Vec<u8> {
        let mut result = Vec::new();
        let mut chars = s.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('n') => result.push(b'\n'),
                    Some('t') => result.push(b'\t'),
                    Some('r') => result.push(b'\r'),
                    Some('\\') => result.push(b'\\'),
                    Some('"') => result.push(b'"'),
                    Some('\'') => result.push(b'\''),
                    Some('0') => result.push(0),
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
                let env_offset = (cc.captures.len() * 8) as u32;
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

        let is_type_or_package = self.struct_defs.contains_key(&ident.name)
            || self.is_known_package(&ident.name);
        if is_type_or_package {
            out.push(Instruction::I64Const(0));
            return Ok(());
        }

        if self.functions.iter().any(|f| f.name == ident.name) {
            return Err(Error::InternalError(format!(
                "function '{}' used as value; first-class function values are not supported",
                ident.name
            )));
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

    fn is_string_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> bool {
        match expr {
            ast::Expression::BasicLit(lit) => lit.kind == LitKind::String,
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    ident.name == "string"
                } else {
                    false
                }
            }
            ast::Expression::Ident(ident) => {
                locals.get_var_struct_type(&ident.name) == Some("__string")
            }
            ast::Expression::Paren(p) => self.is_string_expr(&p.expr, locals),
            ast::Expression::Operation(op) if op.op == Operator::Add && op.y.is_some() => {
                self.is_string_expr(&op.x, locals)
                    && self.is_string_expr(op.y.as_ref().unwrap(), locals)
            }
            _ => false,
        }
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

            if lhs_type == ValType::I32 && rhs_type == ValType::I32 {
                self.compile_expression(&op.x, out, locals)?;
                self.compile_expression(y, out, locals)?;
                return self.emit_i32_op(op.op, out);
            }

            if lhs_type == ValType::I32 && rhs_type == ValType::I64 {
                let mut lhs_buf = Vec::new();
                self.compile_expression(&op.x, &mut lhs_buf, locals)?;
                let mut rhs_buf = Vec::new();
                self.compile_expression(y, &mut rhs_buf, locals)?;

                out.extend(lhs_buf);
                out.push(Instruction::I64ExtendI32S);
                out.extend(rhs_buf);
            } else if lhs_type == ValType::I64 && rhs_type == ValType::I32 {
                self.compile_expression(&op.x, out, locals)?;
                let mut rhs_buf = Vec::new();
                self.compile_expression(y, &mut rhs_buf, locals)?;
                out.extend(rhs_buf);
                out.push(Instruction::I64ExtendI32S);
            } else {
                self.compile_expression(&op.x, out, locals)?;
                self.compile_expression(y, out, locals)?;
            }

            return self.emit_i64_op(op.op, out);
        }

        // Unary operations
        match op.op {
            Operator::Sub => {
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
            Operator::Not => {
                self.compile_expression(&op.x, out, locals)?;
                out.push(Instruction::I32Eqz);
            }
            Operator::And => {
                self.compile_expression(&op.x, out, locals)?;
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

        // Total length
        let total_len = locals.add_local("__scat_total", ValType::I32);
        out.push(Instruction::LocalGet(len1));
        out.push(Instruction::LocalGet(len2));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalTee(total_len));

        // Allocate buffer
        out.push(Instruction::Call(self.alloc_func_idx()));
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

    fn emit_i64_op(
        &self,
        op: Operator,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        match op {
            Operator::Add => out.push(Instruction::I64Add),
            Operator::Sub => out.push(Instruction::I64Sub),
            Operator::Star => out.push(Instruction::I64Mul),
            Operator::Quo => out.push(Instruction::I64DivS),
            Operator::Rem => out.push(Instruction::I64RemS),
            Operator::And => out.push(Instruction::I64And),
            Operator::Or => out.push(Instruction::I64Or),
            Operator::Xor => out.push(Instruction::I64Xor),
            Operator::Shl => out.push(Instruction::I64Shl),
            Operator::Shr => out.push(Instruction::I64ShrS),
            Operator::Equal => out.push(Instruction::I64Eq),
            Operator::NotEqual => out.push(Instruction::I64Ne),
            Operator::Less => out.push(Instruction::I64LtS),
            Operator::LessEqual => out.push(Instruction::I64LeS),
            Operator::Greater => out.push(Instruction::I64GtS),
            Operator::GreaterEqual => out.push(Instruction::I64GeS),
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported operator {:?} for i64 type",
                    op
                )))
            }
        }
        Ok(())
    }

    fn emit_i32_op(
        &self,
        op: Operator,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        match op {
            Operator::Add => out.push(Instruction::I32Add),
            Operator::Sub => out.push(Instruction::I32Sub),
            Operator::Star => out.push(Instruction::I32Mul),
            Operator::Quo => out.push(Instruction::I32DivS),
            Operator::Rem => out.push(Instruction::I32RemS),
            Operator::And => out.push(Instruction::I32And),
            Operator::Or => out.push(Instruction::I32Or),
            Operator::Xor => out.push(Instruction::I32Xor),
            Operator::Shl => out.push(Instruction::I32Shl),
            Operator::Shr => out.push(Instruction::I32ShrS),
            Operator::Equal => out.push(Instruction::I32Eq),
            Operator::NotEqual => out.push(Instruction::I32Ne),
            Operator::Less => out.push(Instruction::I32LtS),
            Operator::LessEqual => out.push(Instruction::I32LeS),
            Operator::Greater => out.push(Instruction::I32GtS),
            Operator::GreaterEqual => out.push(Instruction::I32GeS),
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
                match slice_type.typ.as_ref() {
                    ast::Expression::Ident(id) => match id.name.as_str() {
                        "int32" | "uint32" | "byte" | "bool" => ValType::I32,
                        "float32" => ValType::F32,
                        "float64" => ValType::F64,
                        _ => ValType::I64,
                    },
                    _ => ValType::I64,
                }
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

        if let ast::Expression::Ident(ident) = arg {
            if let Some(&(_, len_local)) = locals.string_locals.get(&ident.name) {
                out.push(Instruction::LocalGet(len_local));
                return Ok(());
            }
            if locals.get_var_struct_type(&ident.name) == Some("__slice") {
                self.compile_expression(arg, out, locals)?;
                out.push(Instruction::I32Load(MemArg {
                    offset: 4,
                    align: 2,
                    memory_index: 0,
                }));
                return Ok(());
            }
        }

        self.compile_expression(arg, out, locals)?;
        let result_count = self.expression_result_count(arg);
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
        Ok(())
    }

    fn compile_builtin_make(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
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

        // Allocate header (12 bytes)
        out.push(Instruction::I32Const(HEADER_SIZE));
        out.push(Instruction::Call(self.alloc_func_idx()));
        let hdr_local = locals.add_local(
            &format!("__make_hdr_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(hdr_local));

        // Allocate data region (len * elem_size bytes)
        out.push(Instruction::LocalGet(len_local));
        out.push(Instruction::I32Const(elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::Call(self.alloc_func_idx()));
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

        // Store cap at header[8] (cap = len initially)
        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::LocalGet(len_local));
        out.push(Instruction::I32Store(MemArg {
            offset: 8,
            align: 2,
            memory_index: 0,
        }));

        // Push header pointer as the slice value
        out.push(Instruction::LocalGet(hdr_local));
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

        // Compile slice argument (header pointer)
        self.compile_expression(&call.args[0], out, locals)?;
        let hdr_local = locals.add_local(
            &format!("__app_hdr_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(hdr_local));

        // Compile the element to append, coerce to slice element type
        self.compile_expression(&call.args[1], out, locals)?;
        let expr_vt = self.infer_val_type(&call.args[1], locals);
        if expr_vt != elem_vt_from_slice {
            match (expr_vt, elem_vt_from_slice) {
                (ValType::I64, ValType::I32) => out.push(Instruction::I32WrapI64),
                (ValType::I32, ValType::I64) => out.push(Instruction::I64ExtendI32S),
                (ValType::F64, ValType::F32) => out.push(Instruction::F32DemoteF64),
                (ValType::F32, ValType::F64) => out.push(Instruction::F64PromoteF32),
                _ => {}
            }
        }
        let elem_vt = elem_vt_from_slice;
        let elem_local = locals.add_local(
            &format!("__app_elem_{}", locals.locals.len()),
            elem_vt,
        );
        out.push(Instruction::LocalSet(elem_local));

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

        // If old_len >= cap, grow
        out.push(Instruction::LocalGet(old_len));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::If(BlockType::Empty));
        {
            // new_cap = (cap + 1) * 2
            let new_cap = locals.add_local(
                &format!("__app_ncap_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalGet(cap_local));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Const(2));
            out.push(Instruction::I32Mul);
            out.push(Instruction::LocalSet(new_cap));

            // Allocate new data: new_cap * elem_size
            let new_data = locals.add_local(
                &format!("__app_ndata_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::Call(self.alloc_func_idx()));
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

        // Store element at data_ptr + old_len * elem_size
        out.push(Instruction::LocalGet(data_ptr));
        out.push(Instruction::LocalGet(old_len));
        out.push(Instruction::I32Const(elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalGet(elem_local));
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

        // Update header: len = old_len + 1
        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::LocalGet(old_len));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::I32Store(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        }));

        // Push header pointer as result
        out.push(Instruction::LocalGet(hdr_local));
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
                    "panic" => {
                        out.push(Instruction::Unreachable);
                        return Ok(());
                    }
                    "int" | "int64" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::F64 => out.push(Instruction::I64TruncF64S),
                                ValType::F32 => out.push(Instruction::I64TruncF32S),
                                ValType::I32 => out.push(Instruction::I64ExtendI32S),
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
                            match vt {
                                ValType::I64 => out.push(Instruction::F64ConvertI64S),
                                ValType::I32 => out.push(Instruction::F64ConvertI32S),
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
                            match vt {
                                ValType::I64 => out.push(Instruction::F32ConvertI64S),
                                ValType::I32 => out.push(Instruction::F32ConvertI32S),
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
                    "int32" => {
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
                    "byte" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::I32 => {}
                                ValType::F64 => out.push(Instruction::I32TruncF64S),
                                ValType::F32 => out.push(Instruction::I32TruncF32S),
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for byte conversion",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(());
                    }
                    "string" => {
                        if let Some(arg) = call.args.first() {
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I32 => {
                                    self.compile_expression(arg, out, locals)?;
                                    return Ok(());
                                }
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "string() conversion from {:?} is not supported; only []byte is supported",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(());
                    }
                    _ => {}
                }

                // Check if it's a closure variable
                if let Some(&(func_idx, env_local)) =
                    locals.closure_info.get(&ident.name)
                {
                    if env_local != u32::MAX {
                        out.push(Instruction::LocalGet(env_local));
                    } else {
                        out.push(Instruction::I32Const(0));
                    }
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }
                    out.push(Instruction::Call(func_idx));
                    return Ok(());
                }

                // Look up as a user function
                for arg in &call.args {
                    self.compile_expression(arg, out, locals)?;
                }

                if let Some(func_info) =
                    self.functions.iter().find(|f| f.name == ident.name)
                {
                    out.push(Instruction::Call(func_info.wasm_func_idx));
                } else {
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
                            _ => None,
                        };
                        if let Some(host_idx) = ctx_host_idx {
                            if sel.sel.name == "Log" {
                                if let Some(arg) = call.args.first() {
                                    self.compile_expression(arg, out, locals)?;
                                }
                                out.push(Instruction::Call(host_idx));
                                return Ok(());
                            } else {
                                let buf_size = 256i32;
                                out.push(Instruction::I32Const(buf_size));
                                out.push(Instruction::Call(self.alloc_func_idx()));
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
                        ("math", func_name) => {
                            return Err(Error::InternalError(format!(
                                "unsupported math function: math.{}",
                                func_name
                            )));
                        }
                        _ => {}
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

                    // Fallback: try variable-name-qualified, then bare method name
                    let func_idx = found
                        .or_else(|| {
                            let qualified = format!("{}.{}", pkg_ident.name, sel.sel.name);
                            self.functions.iter().find(|f| f.name == qualified).map(|f| f.wasm_func_idx)
                        })
                        .or_else(|| {
                            self.functions.iter().find(|f| f.recv_type.is_some() && f.name.ends_with(&format!(".{}", sel.sel.name))).map(|f| f.wasm_func_idx)
                        });

                    if let Some(idx) = func_idx {
                        out.push(Instruction::Call(idx));
                    } else {
                        return Err(Error::InternalError(format!(
                            "undefined method: {}.{}",
                            pkg_ident.name, sel.sel.name
                        )));
                    }
                }
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

        let struct_type_name = if let ast::Expression::Ident(ident) = sel.x.as_ref() {
            locals
                .get_var_struct_type(&ident.name)
                .map(|s| s.to_string())
        } else {
            None
        };

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
        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: closure_name,
            params: vec![],
            results: vec![],
            is_exported: false,
            recv_type: None,
        });

        // Build inner locals: env_ptr + go params
        let mut inner_params: Vec<(String, ValType)> =
            vec![("__env_ptr".to_string(), ValType::I32)];
        for (name, vt) in go_param_names.iter().zip(go_param_types.iter()) {
            inner_params.push((name.clone(), *vt));
        }
        let mut inner_locals = LocalAlloc::new(inner_params);

        // Set up capture state
        let outer_snapshot = outer_locals.all_entries();
        self.closure_captures = Some(ClosureCaptureState {
            outer_locals: outer_snapshot,
            captures: Vec::new(),
        });

        let mut body: Vec<Instruction<'static>> = Vec::new();
        self.deferred_calls.push(Vec::new());
        self.compile_block(&func_lit.body, &mut body, &mut inner_locals, &result_types)?;
        self.emit_deferred_calls(&mut body);
        self.deferred_calls.pop();

        // Extract captures
        let captures = if let Some(cc) = self.closure_captures.take() {
            cc.captures
        } else {
            Vec::new()
        };

        // In outer function: allocate env and store captures
        if !captures.is_empty() {
            let env_size = (captures.len() * 8) as i32;
            out.push(Instruction::I32Const(env_size));
            out.push(Instruction::Call(self.alloc_func_idx()));
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

        // Default return values if body doesn't return
        for vt in &result_types {
            match vt {
                ValType::I32 => body.push(Instruction::I32Const(0)),
                ValType::I64 => body.push(Instruction::I64Const(0)),
                ValType::F32 => body.push(Instruction::F32Const(0.0)),
                ValType::F64 => body.push(Instruction::F64Const(0.0)),
                _ => body.push(Instruction::I32Const(0)),
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

    fn compile_composite_lit(
        &mut self,
        comp: &ast::CompositeLit,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let type_name = if let ast::Expression::Ident(ident) = comp.typ.as_ref() {
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
        out.push(Instruction::Call(self.alloc_func_idx()));

        let ptr_local = locals.add_local("__comp_ptr", ValType::I32);
        out.push(Instruction::LocalSet(ptr_local));

        for (i, kv) in comp.val.values.iter().enumerate() {
            let elem_expr = match &kv.val {
                ast::Element::Expr(e) => e,
                ast::Element::LitValue(_) => continue,
            };

            // Determine offset and type from struct layout
            let (offset, field_wasm_type) = if let Some(ref key) = kv.key {
                if let ast::Element::Expr(ast::Expression::Ident(key_ident)) = key {
                    if let Some(ref sd) = struct_def {
                        if let Some(field) = sd.find_field(&key_ident.name) {
                            (field.offset as u64, Some(field.wasm_type))
                        } else {
                            ((i * 8) as u64, None)
                        }
                    } else {
                        ((i * 8) as u64, None)
                    }
                } else {
                    ((i * 8) as u64, None)
                }
            } else if let Some(ref sd) = struct_def {
                if i < sd.fields.len() {
                    (
                        sd.fields[i].offset as u64,
                        Some(sd.fields[i].wasm_type),
                    )
                } else {
                    ((i * 8) as u64, None)
                }
            } else {
                ((i * 8) as u64, None)
            };

            out.push(Instruction::LocalGet(ptr_local));
            self.compile_expression(elem_expr, out, locals)?;

            let vt = field_wasm_type
                .map(|wt| wt.to_val_type())
                .unwrap_or_else(|| self.infer_val_type(elem_expr, locals));
            match vt {
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
        self.compile_expression(&slice.left, out, locals)?;

        let base_local = locals.add_local(
            &format!("__slice_base_{}", locals.locals.len()),
            ValType::I32,
        );
        let len_local = locals.add_local(
            &format!("__slice_len_{}", locals.locals.len()),
            ValType::I32,
        );

        let expr_count = self.expression_result_count(&slice.left);
        if expr_count >= 2 {
            if expr_count >= 3 {
                out.push(Instruction::Drop);
            }
            out.push(Instruction::LocalSet(len_local));
            out.push(Instruction::LocalSet(base_local));
        } else {
            out.push(Instruction::LocalSet(base_local));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(len_local));
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
            out.push(Instruction::LocalGet(len_local));
        }
        out.push(Instruction::LocalSet(high_local));

        let slice_elem_vt = if let ast::Expression::Ident(ident) = &*slice.left {
            locals
                .slice_elem_types
                .get(&ident.name)
                .copied()
                .unwrap_or(ValType::I64)
        } else {
            ValType::I64
        };
        let (slice_elem_size, _) = Self::elem_size_and_align(slice_elem_vt);

        out.push(Instruction::LocalGet(base_local));
        out.push(Instruction::LocalGet(low_local));
        out.push(Instruction::I32Const(slice_elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);

        // new_len = high - low
        out.push(Instruction::LocalGet(high_local));
        out.push(Instruction::LocalGet(low_local));
        out.push(Instruction::I32Sub);

        Ok(())
    }

    fn compile_index(
        &mut self,
        idx: &ast::Index,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let is_slice_header = if let ast::Expression::Ident(ident) = idx.left.as_ref() {
            locals.get_var_struct_type(&ident.name) == Some("__slice")
        } else {
            false
        };

        let elem_vt = if let ast::Expression::Ident(ident) = idx.left.as_ref() {
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
            self.compile_expression(&idx.left, out, locals)?;
            out.push(Instruction::I32Load(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
        } else {
            self.compile_expression(&idx.left, out, locals)?;
        }

        self.compile_expression(&idx.index, out, locals)?;

        let idx_vt = self.infer_val_type(&idx.index, locals);
        if idx_vt == ValType::I64 {
            out.push(Instruction::I32WrapI64);
        }

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
        Ok(())
    }

    fn compile_index_store_addr(
        &mut self,
        idx: &ast::Index,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(ValType, u32), Error> {
        let is_slice_header = if let ast::Expression::Ident(ident) = idx.left.as_ref() {
            locals.get_var_struct_type(&ident.name) == Some("__slice")
        } else {
            false
        };

        let elem_vt = if let ast::Expression::Ident(ident) = idx.left.as_ref() {
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
            self.compile_expression(&idx.left, out, locals)?;
            out.push(Instruction::I32Load(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
        } else {
            self.compile_expression(&idx.left, out, locals)?;
        }

        self.compile_expression(&idx.index, out, locals)?;

        let idx_vt = self.infer_val_type(&idx.index, locals);
        if idx_vt == ValType::I64 {
            out.push(Instruction::I32WrapI64);
        }

        out.push(Instruction::I32Const(elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);

        Ok((elem_vt, align))
    }

    fn compile_selector_store_addr(
        &mut self,
        sel: &ast::Selector,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(u64, ValType), Error> {
        self.compile_expression(&sel.x, out, locals)?;

        let struct_type_name = if let ast::Expression::Ident(ident) = sel.x.as_ref() {
            locals
                .get_var_struct_type(&ident.name)
                .map(|s| s.to_string())
        } else {
            None
        };

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

    fn emit_typed_coerce(from: ValType, to: ValType, out: &mut Vec<Instruction<'static>>) {
        if from == to {
            return;
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
            _ => {}
        }
    }

    fn emit_compound_op(
        &self,
        op: &Operator,
        vt: ValType,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        match op {
            Operator::AddAssign => out.push(Self::typed_add(vt)),
            Operator::SubAssign => out.push(Self::typed_sub(vt)),
            Operator::MulAssign => out.push(Self::typed_mul(vt)),
            Operator::QuoAssign => out.push(Self::typed_div(vt)),
            Operator::RemAssign
            | Operator::AndAssign
            | Operator::OrAssign
            | Operator::XorAssign
            | Operator::ShlAssign
            | Operator::ShrAssign
                if matches!(vt, ValType::F32 | ValType::F64) =>
            {
                return Err(Error::InternalError(format!(
                    "operator {:?} is not valid on floating-point type {:?}",
                    op, vt
                )));
            }
            Operator::RemAssign => match vt {
                ValType::I32 => out.push(Instruction::I32RemS),
                _ => out.push(Instruction::I64RemS),
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
                ValType::I32 => out.push(Instruction::I32ShrS),
                _ => out.push(Instruction::I64ShrS),
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
        if let Some(deferred) = self.deferred_calls.last() {
            for call in deferred.iter().rev() {
                for (local_idx, _vt) in &call.arg_locals {
                    out.push(Instruction::LocalGet(*local_idx));
                }
                out.push(Instruction::Call(call.func_idx));
            }
        }
    }

    fn field_to_wasm_types(&self, field: &ast::Field) -> Vec<WasmType> {
        match &field.typ {
            ast::Expression::Ident(ident) => match ident.name.as_str() {
                "bool" | "byte" | "uint8" | "int8" | "int16" | "uint16" | "int32"
                | "uint32" | "rune" => vec![WasmType::I32],
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
                _ => ValType::I64,
            },
            ast::Expression::Ident(ident) => match ident.name.as_str() {
                "true" | "false" => ValType::I32,
                "nil" => ValType::I32,
                _ => locals.find_type(&ident.name).unwrap_or(ValType::I64),
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
                    self.infer_val_type(&op.x, locals)
                }
            }
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    match ident.name.as_str() {
                        "float64" => ValType::F64,
                        "float32" => ValType::F32,
                        "int" | "int64" => ValType::I64,
                        "int32" | "byte" | "bool" => ValType::I32,
                        "len" | "make" | "append" => ValType::I32,
                        _ => ValType::I64,
                    }
                } else if let ast::Expression::Selector(sel) = call.func.as_ref() {
                    if let ast::Expression::Ident(pkg) = sel.x.as_ref() {
                        match (pkg.name.as_str(), sel.sel.name.as_str()) {
                            ("math", _) => ValType::F64,
                            _ => ValType::I64,
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
                if let ast::Expression::Ident(ident) = sel.x.as_ref() {
                    if let Some(type_name) = locals.get_var_struct_type(&ident.name) {
                        if let Some(sd) = self.struct_defs.get(type_name) {
                            if let Some(field) = sd.find_field(&sel.sel.name) {
                                return field.wasm_type.to_val_type();
                            }
                        }
                    }
                }
                ValType::I32
            }
            ast::Expression::CompositeLit(_) => ValType::I32,
            ast::Expression::Index(idx) => {
                if let ast::Expression::Ident(ident) = idx.left.as_ref() {
                    locals
                        .slice_elem_types
                        .get(&ident.name)
                        .copied()
                        .unwrap_or(ValType::I64)
                } else {
                    ValType::I64
                }
            }
            ast::Expression::FuncLit(_) => ValType::I32,
            _ => ValType::I64,
        }
    }

    fn expr_to_val_type(&self, expr: &ast::Expression) -> ValType {
        match expr {
            ast::Expression::Ident(ident) => match ident.name.as_str() {
                "bool" | "byte" | "uint8" | "int8" | "int16" | "uint16" | "int32"
                | "uint32" | "rune" => ValType::I32,
                "int" | "int64" | "uint" | "uint64" => ValType::I64,
                "float32" => ValType::F32,
                "float64" => ValType::F64,
                "string" | "error" => ValType::I32,
                _ => ValType::I32,
            },
            ast::Expression::TypePointer(_) => ValType::I32,
            ast::Expression::TypeSlice(_) => ValType::I32,
            _ => ValType::I64,
        }
    }

    fn elem_size_and_align(vt: ValType) -> (i32, u32) {
        match vt {
            ValType::I32 | ValType::F32 => (4, 2),
            _ => (8, 3),
        }
    }

    fn expression_result_count(&self, expr: &ast::Expression) -> usize {
        match expr {
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    match ident.name.as_str() {
                        "panic" => 0,
                        "len" | "make" | "append" | "int" | "int64" | "float64"
                        | "float32" | "int32" | "byte" | "string" | "bool" => 1,
                        _ => {
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
                    if let ast::Expression::Ident(_pkg) = sel.x.as_ref() {
                        match sel.sel.name.as_str() {
                            "Log" => 0,
                            "QueryID" | "Database" | "Schema" | "User" | "Config" => 2,
                            _ => 1,
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
            _ => 1,
        }
    }

    fn build_manifest(&mut self, file: &ast::File) {
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
                let accumulate = methods
                    .iter()
                    .find(|m| m.name.name == "Accumulate")
                    .unwrap();
                let finalize = methods
                    .iter()
                    .find(|m| m.name.name == "Finalize")
                    .unwrap();

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
                self.emit_aggregate_init(&init_name, struct_size);

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
    }

    fn emit_aggregate_init(&mut self, name: &str, struct_size: u32) {
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
        func.instruction(&Instruction::Call(self.alloc_func_idx()));
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
            is_exported: true,
            recv_type: None,
        });
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
        if !self.element_section.is_empty() {
            module.section(&self.element_section);
        }
        module.section(&self.code_section);

        module.finish()
    }
}
