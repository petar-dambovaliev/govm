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
}

impl LocalAlloc {
    fn new(params: Vec<(String, ValType)>) -> Self {
        Self {
            params,
            locals: Vec::new(),
            var_types: HashMap::new(),
            closure_info: HashMap::new(),
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

    fn compile_global_var(&mut self, _spec: &ast::VarSpec) -> Result<(), Error> {
        Ok(())
    }

    fn compile_global_const(&mut self, _spec: &ast::ConstSpec) -> Result<(), Error> {
        Ok(())
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

        // Track struct types for parameters
        for field in &decl.typ.params.list {
            if let ast::Expression::Ident(type_ident) = &field.typ {
                if self.struct_defs.contains_key(&type_ident.name) {
                    for name_ident in &field.name {
                        locals.set_var_struct_type(&name_ident.name, &type_ident.name);
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

                        // Track closure assignments
                        let is_func_lit =
                            matches!(&assign.right[i], ast::Expression::FuncLit(_));

                        self.compile_expression(&assign.right[i], out, locals)?;
                        out.push(Instruction::LocalSet(local_idx));

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
                        if let Some(idx) = locals.find(&ident.name) {
                            let vt = locals
                                .find_type(&ident.name)
                                .unwrap_or(ValType::I64);
                            match assign.op {
                                Operator::Assign => {
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::AddAssign => {
                                    let val = out.pop();
                                    out.push(Instruction::LocalGet(idx));
                                    if let Some(v) = val {
                                        out.push(v);
                                    }
                                    out.push(Self::typed_add(vt));
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::SubAssign => {
                                    let val = out.pop();
                                    out.push(Instruction::LocalGet(idx));
                                    if let Some(v) = val {
                                        out.push(v);
                                    }
                                    out.push(Self::typed_sub(vt));
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::MulAssign => {
                                    let val = out.pop();
                                    out.push(Instruction::LocalGet(idx));
                                    if let Some(v) = val {
                                        out.push(v);
                                    }
                                    out.push(Self::typed_mul(vt));
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::QuoAssign => {
                                    let val = out.pop();
                                    out.push(Instruction::LocalGet(idx));
                                    if let Some(v) = val {
                                        out.push(v);
                                    }
                                    out.push(Self::typed_div(vt));
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::RemAssign => {
                                    let val = out.pop();
                                    out.push(Instruction::LocalGet(idx));
                                    if let Some(v) = val {
                                        out.push(v);
                                    }
                                    match vt {
                                        ValType::I32 => {
                                            out.push(Instruction::I32RemS)
                                        }
                                        _ => out.push(Instruction::I64RemS),
                                    }
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::AndAssign => {
                                    let val = out.pop();
                                    out.push(Instruction::LocalGet(idx));
                                    if let Some(v) = val {
                                        out.push(v);
                                    }
                                    match vt {
                                        ValType::I32 => {
                                            out.push(Instruction::I32And)
                                        }
                                        _ => out.push(Instruction::I64And),
                                    }
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::OrAssign => {
                                    let val = out.pop();
                                    out.push(Instruction::LocalGet(idx));
                                    if let Some(v) = val {
                                        out.push(v);
                                    }
                                    match vt {
                                        ValType::I32 => {
                                            out.push(Instruction::I32Or)
                                        }
                                        _ => out.push(Instruction::I64Or),
                                    }
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::XorAssign => {
                                    let val = out.pop();
                                    out.push(Instruction::LocalGet(idx));
                                    if let Some(v) = val {
                                        out.push(v);
                                    }
                                    match vt {
                                        ValType::I32 => {
                                            out.push(Instruction::I32Xor)
                                        }
                                        _ => out.push(Instruction::I64Xor),
                                    }
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::ShlAssign => {
                                    let val = out.pop();
                                    out.push(Instruction::LocalGet(idx));
                                    if let Some(v) = val {
                                        out.push(v);
                                    }
                                    match vt {
                                        ValType::I32 => {
                                            out.push(Instruction::I32Shl)
                                        }
                                        _ => out.push(Instruction::I64Shl),
                                    }
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::ShrAssign => {
                                    let val = out.pop();
                                    out.push(Instruction::LocalGet(idx));
                                    if let Some(v) = val {
                                        out.push(v);
                                    }
                                    match vt {
                                        ValType::I32 => {
                                            out.push(Instruction::I32ShrS)
                                        }
                                        _ => out.push(Instruction::I64ShrS),
                                    }
                                    out.push(Instruction::LocalSet(idx));
                                }
                                _ => {
                                    out.push(Instruction::LocalSet(idx));
                                }
                            }
                        }
                    }
                    _ => {}
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

        self.compile_expression(&range.expr, out, locals)?;

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
                    let value_local = if range
                        .op
                        .as_ref()
                        .map_or(false, |(_, op)| *op == Operator::Define)
                    {
                        locals.add_local(&ident.name, ValType::I64)
                    } else {
                        locals.find(&ident.name).unwrap_or_else(|| {
                            locals.add_local(&ident.name, ValType::I64)
                        })
                    };
                    // Load element: base_ptr + idx * 8
                    out.push(Instruction::LocalGet(base_ptr_local));
                    out.push(Instruction::LocalGet(idx_local));
                    out.push(Instruction::I32Const(8));
                    out.push(Instruction::I32Mul);
                    out.push(Instruction::I32Add);
                    out.push(Instruction::I64Load(MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    }));
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

        let tag_local = if let Some(tag) = &switch.tag {
            let local = locals.add_local("__switch_tag", ValType::I64);
            self.compile_expression(tag, out, locals)?;
            out.push(Instruction::LocalSet(local));
            Some(local)
        } else {
            None
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

        let num_cases = non_default_cases.len();

        for (i, case) in non_default_cases.iter().enumerate() {
            let mut first = true;
            for expr in &case.list {
                if !first {
                    out.push(Instruction::I32Or);
                }
                if let Some(tag_l) = tag_local {
                    out.push(Instruction::LocalGet(tag_l));
                    self.compile_expression(expr, out, locals)?;
                    out.push(Instruction::I64Eq);
                } else {
                    self.compile_expression(expr, out, locals)?;
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
            _ => {}
        }
        Ok(())
    }

    fn compile_incdec(
        &mut self,
        incdec: &ast::IncDecStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        if let ast::Expression::Ident(ident) = &incdec.expr {
            if let Some(idx) = locals.find(&ident.name) {
                out.push(Instruction::LocalGet(idx));
                let vt = self.infer_val_type(&incdec.expr, locals);
                match (incdec.op, vt) {
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
                    _ => {
                        out.push(Instruction::I64Const(1));
                        out.push(Instruction::I64Add);
                    }
                }
                out.push(Instruction::LocalSet(idx));
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

                        // Track struct types
                        if let Some(ref typ) = spec.typ {
                            if let ast::Expression::Ident(type_ident) = typ {
                                if self.struct_defs.contains_key(&type_ident.name) {
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
                        }

                        if i < spec.values.len() {
                            self.compile_expression(&spec.values[i], out, locals)?;
                            out.push(Instruction::LocalSet(local_idx));
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
            ast::Expression::BasicLit(lit) => self.compile_basic_lit(lit, out),
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
            ast::Expression::TypeAssert(ta) => {
                self.compile_expression(&ta.left, out, locals)?;
                Ok(())
            }
            ast::Expression::Slice(_) => Ok(()),
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
            _ => Ok(()),
        }
    }

    fn compile_basic_lit(
        &self,
        lit: &ast::BasicLit,
        out: &mut Vec<Instruction<'static>>,
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
                let bytes = s.as_bytes();
                let len = bytes.len() as i32;

                out.push(Instruction::I32Const(len));
                out.push(Instruction::Call(self.alloc_func_idx()));
                out.push(Instruction::I32Const(len));
            }
            LitKind::Char => {
                let s = lit.value.trim_matches('\'');
                let ch = s.chars().next().unwrap_or('\0') as i32;
                out.push(Instruction::I32Const(ch));
            }
            _ => {
                out.push(Instruction::I64Const(0));
            }
        }
        Ok(())
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

        out.push(Instruction::I64Const(0));
        Ok(())
    }

    fn is_string_expr(&self, expr: &ast::Expression, _locals: &LocalAlloc) -> bool {
        match expr {
            ast::Expression::BasicLit(lit) => lit.kind == LitKind::String,
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    ident.name == "string"
                } else {
                    false
                }
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
            // String concatenation
            if op.op == Operator::Add
                && self.is_string_expr(&op.x, locals)
                && self.is_string_expr(y, locals)
            {
                return self.emit_string_concat(&op.x, y, out, locals);
            }

            self.compile_expression(&op.x, out, locals)?;
            self.compile_expression(y, out, locals)?;

            let lhs_type = self.infer_val_type(&op.x, locals);
            let rhs_type = self.infer_val_type(y, locals);

            if lhs_type == ValType::F64 || rhs_type == ValType::F64 {
                if lhs_type != ValType::F64 {
                    let val = out.pop();
                    let lhs = out.pop();
                    if let Some(l) = lhs {
                        out.push(l);
                    }
                    out.push(Instruction::F64ConvertI64S);
                    if let Some(v) = val {
                        out.push(v);
                    }
                }
                if rhs_type != ValType::F64 {
                    out.push(Instruction::F64ConvertI64S);
                }
                return self.emit_f64_op(op.op, out);
            }

            if lhs_type == ValType::I32 && rhs_type == ValType::I32 {
                return self.emit_i32_op(op.op, out);
            }

            if lhs_type == ValType::I32 && rhs_type == ValType::I64 {
                let rhs = out.pop();
                let lhs = out.pop();
                if let Some(l) = lhs {
                    out.push(l);
                }
                out.push(Instruction::I64ExtendI32S);
                if let Some(r) = rhs {
                    out.push(r);
                }
            } else if lhs_type == ValType::I64 && rhs_type == ValType::I32 {
                out.push(Instruction::I64ExtendI32S);
            }

            return self.emit_i64_op(op.op, out);
        }

        // Unary operations
        self.compile_expression(&op.x, out, locals)?;
        match op.op {
            Operator::Sub => {
                let vt = self.infer_val_type(&op.x, locals);
                match vt {
                    ValType::I64 => {
                        let val = out.pop();
                        out.push(Instruction::I64Const(0));
                        if let Some(v) = val {
                            out.push(v);
                        }
                        out.push(Instruction::I64Sub);
                    }
                    ValType::F64 => {
                        out.push(Instruction::F64Neg);
                    }
                    ValType::I32 => {
                        let val = out.pop();
                        out.push(Instruction::I32Const(0));
                        if let Some(v) = val {
                            out.push(v);
                        }
                        out.push(Instruction::I32Sub);
                    }
                    _ => {}
                }
            }
            Operator::Not => {
                out.push(Instruction::I32Eqz);
            }
            Operator::And => {}
            _ => {}
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
            Operator::AndAnd => out.push(Instruction::I64And),
            Operator::OrOr => out.push(Instruction::I64Or),
            _ => {}
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
            Operator::AndAnd => out.push(Instruction::I32And),
            Operator::OrOr => out.push(Instruction::I32Or),
            _ => {}
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
            _ => {}
        }
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
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                        }
                        return Ok(());
                    }
                    "make" => {
                        if let Some(len_arg) = call.args.get(1) {
                            self.compile_expression(len_arg, out, locals)?;
                        } else {
                            out.push(Instruction::I64Const(0));
                        }
                        return Ok(());
                    }
                    "append" => {
                        for arg in &call.args {
                            self.compile_expression(arg, out, locals)?;
                        }
                        return Ok(());
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
                                _ => {}
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
                                _ => {}
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
                                _ => {}
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
                                _ => {}
                            }
                        }
                        return Ok(());
                    }
                    "byte" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            if vt == ValType::I64 {
                                out.push(Instruction::I32WrapI64);
                            }
                        }
                        return Ok(());
                    }
                    "string" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
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
                }
            }
            ast::Expression::Selector(sel) => {
                if let ast::Expression::Ident(pkg_ident) = sel.x.as_ref() {
                    match (pkg_ident.name.as_str(), sel.sel.name.as_str()) {
                        ("fmt", "Errorf" | "Sprintf") => {
                            out.push(Instruction::I32Const(1));
                            out.push(Instruction::I32Const(5));
                            return Ok(());
                        }
                        ("math", "Sqrt") => {
                            if let Some(arg) = call.args.first() {
                                self.compile_expression(arg, out, locals)?;
                            }
                            out.push(Instruction::F64Sqrt);
                            return Ok(());
                        }
                        ("math", "Abs") => {
                            if let Some(arg) = call.args.first() {
                                self.compile_expression(arg, out, locals)?;
                            }
                            out.push(Instruction::F64Abs);
                            return Ok(());
                        }
                        ("math", "Floor") => {
                            if let Some(arg) = call.args.first() {
                                self.compile_expression(arg, out, locals)?;
                            }
                            out.push(Instruction::F64Floor);
                            return Ok(());
                        }
                        ("math", "Ceil") => {
                            if let Some(arg) = call.args.first() {
                                self.compile_expression(arg, out, locals)?;
                            }
                            out.push(Instruction::F64Ceil);
                            return Ok(());
                        }
                        ("math", "Min") => {
                            if call.args.len() >= 2 {
                                self.compile_expression(&call.args[0], out, locals)?;
                                self.compile_expression(&call.args[1], out, locals)?;
                            }
                            out.push(Instruction::F64Min);
                            return Ok(());
                        }
                        ("math", "Max") => {
                            if call.args.len() >= 2 {
                                self.compile_expression(&call.args[0], out, locals)?;
                                self.compile_expression(&call.args[1], out, locals)?;
                            }
                            out.push(Instruction::F64Max);
                            return Ok(());
                        }
                        _ => {}
                    }

                    // Method call on a receiver
                    self.compile_expression(sel.x.as_ref(), out, locals)?;
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }

                    // Try qualified name first (Type.Method), then simple name
                    let qualified_name =
                        format!("{}.{}", pkg_ident.name, sel.sel.name);
                    if let Some(func_info) =
                        self.functions.iter().find(|f| f.name == qualified_name)
                    {
                        out.push(Instruction::Call(func_info.wasm_func_idx));
                    } else if let Some(func_info) = self
                        .functions
                        .iter()
                        .find(|f| f.name == sel.sel.name)
                    {
                        out.push(Instruction::Call(func_info.wasm_func_idx));
                    }
                }
            }
            _ => {
                for arg in &call.args {
                    self.compile_expression(arg, out, locals)?;
                }
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

        // Fallback
        out.push(Instruction::I32Load(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        }));
        Ok(())
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
        self.code_section.function(&func);

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

    fn compile_index(
        &mut self,
        idx: &ast::Index,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        self.compile_expression(&idx.left, out, locals)?;
        self.compile_expression(&idx.index, out, locals)?;

        let idx_vt = self.infer_val_type(&idx.index, locals);
        if idx_vt == ValType::I64 {
            out.push(Instruction::I32WrapI64);
        }

        // Default to 8-byte i64 elements; a full type inference pass would
        // determine the real element type here.
        out.push(Instruction::I32Const(8));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::I64Load(MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        }));
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
                        "len" => ValType::I32,
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
            ast::Expression::Selector(_) => ValType::I32,
            ast::Expression::CompositeLit(_) => ValType::I32,
            ast::Expression::Index(_) => ValType::I64,
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

    fn expression_result_count(&self, expr: &ast::Expression) -> usize {
        match expr {
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    match ident.name.as_str() {
                        "panic" => 0,
                        "len" | "int" | "int64" | "float64" | "float32" | "int32"
                        | "byte" | "string" | "bool" => 1,
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
