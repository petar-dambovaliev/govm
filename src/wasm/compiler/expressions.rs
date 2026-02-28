use super::*;

impl WasmCompiler {
    pub(crate) fn compile_expression(
        &mut self,
        expr: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<GoType, Error> {
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
                Ok(GoType::from_val_type(deref_vt))
            }
            ast::Expression::TypeAssert(ta) => {
                self.compile_type_assert(ta, out, locals)
            }
            ast::Expression::Slice(slice) => {
                self.compile_slice_expr(slice, out, locals)
            }
            ast::Expression::List(exprs) => {
                let mut last_gt = GoType::Void;
                for e in exprs {
                    last_gt = self.compile_expression(e, out, locals)?;
                }
                Ok(last_gt)
            }
            ast::Expression::Invar(inv) => {
                self.compile_expression(&inv.expr, out, locals)
            }
            ast::Expression::Range(_) => Err(Error::InternalError(
                "range expression is only valid inside a for statement".to_string(),
            )),
            ast::Expression::TypeMap(_) => Ok(GoType::Void),
            ast::Expression::TypeInterface(_) => Ok(GoType::Void),
            ast::Expression::TypeChannel(_) => Err(Error::InternalError(
                "channels are not supported in WASM UDFs".to_string(),
            )),
            ast::Expression::TypeArray(_)
            | ast::Expression::TypeSlice(_)
            | ast::Expression::TypeFunction(_)
            | ast::Expression::TypeStruct(_)
            | ast::Expression::TypePointer(_)
            | ast::Expression::IndexList(_)
            | ast::Expression::Ellipsis(_) => Ok(GoType::Void),
        }
    }

    pub(crate) fn compile_basic_lit(
        &self,
        lit: &ast::BasicLit,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<GoType, Error> {
        match lit.kind {
            LitKind::Integer => {
                let val128 = Self::parse_go_int(&lit.value)
                    .map_err(|e| Error::SyntaxError(e))?;
                out.push(Instruction::I64Const(val128 as i64));
                return Ok(GoType::UntypedInt);
            }
            LitKind::Float => {
                let val: f64 = Self::parse_go_float(&lit.value).map_err(|_| {
                    Error::SyntaxError(format!("invalid float literal: {}", lit.value))
                })?;
                out.push(Instruction::F64Const(val.into()));
                return Ok(GoType::UntypedFloat);
            }
            LitKind::String => {
                let bytes = Self::extract_string_bytes(&lit.value);
                let len = bytes.len() as i32;

                if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                    let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
                    for &byte in bytes.iter() {
                        out.push(Instruction::I32Const(byte as i32));
                    }
                    out.push(Instruction::ArrayNewFixed {
                        array_type_index: byte_array_idx,
                        array_size: bytes.len() as u32,
                    });
                    out.push(Instruction::I32Const(len));
                    out.push(Instruction::StructNew(go_string_idx));
                } else {
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
                return Ok(GoType::String);
            }
            LitKind::Char => {
                let s = lit.value.strip_prefix('\'').unwrap_or(&lit.value);
                let s = s.strip_suffix('\'').unwrap_or(s);
                let ch = Self::unescape_go_char(s)? as i32;
                out.push(Instruction::I32Const(ch));
                return Ok(GoType::Int32);
            }
            LitKind::Imag => {
                let num_str = lit.value.trim_end_matches('i');
                let imag_val: f64 = num_str.parse().map_err(|_| {
                    Error::SyntaxError(format!("invalid imaginary literal: {}", lit.value))
                })?;

                if let Some(gc_idx) = self.gc_builtin_types.complex128 {
                    out.push(Instruction::F64Const(0.0_f64.into()));
                    out.push(Instruction::F64Const(imag_val.into()));
                    out.push(Instruction::StructNew(gc_idx));
                } else {
                    let total_size: i32 = 16;
                    let float_align: u32 = 3;

                    let real_local = locals.add_local(
                        &format!("__imag_r_{}", locals.locals.len()),
                        ValType::F64,
                    );
                    let imag_local = locals.add_local(
                        &format!("__imag_i_{}", locals.locals.len()),
                        ValType::F64,
                    );
                    out.push(Instruction::F64Const(0.0_f64.into()));
                    out.push(Instruction::LocalSet(real_local));
                    out.push(Instruction::F64Const(imag_val.into()));
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
                return Ok(GoType::Complex128);
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported literal kind: {:?}",
                    lit.kind
                )));
            }
        }
    }

    pub(crate) fn extract_string_bytes(lit_value: &str) -> Vec<u8> {
        if lit_value.starts_with('`') {
            let s = lit_value.trim_matches('`');
            // Go spec: carriage return characters inside raw string literals are discarded
            s.bytes().filter(|&b| b != b'\r').collect()
        } else {
            let s = lit_value.trim_matches('"');
            Self::unescape_go_string(s)
        }
    }

    pub(crate) fn extract_string_content(lit_value: &str) -> Option<String> {
        if lit_value.starts_with('`') {
            let s: String = lit_value.trim_matches('`').chars().filter(|&c| c != '\r').collect();
            Some(s)
        } else {
            let s = lit_value.trim_matches('"');
            let bytes = Self::unescape_go_string(s);
            match String::from_utf8(bytes) {
                Ok(s) => Some(s),
                Err(e) => {
                    let bytes = e.into_bytes();
                    Some(bytes.into_iter().map(|b| b as char).collect())
                }
            }
        }
    }

    pub(crate) fn unescape_go_string(s: &str) -> Vec<u8> {
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

    pub(crate) fn unescape_go_char(s: &str) -> Result<char, Error> {
        let mut chars = s.chars();
        match chars.next() {
            Some('\\') => match chars.next() {
                Some('n') => Ok('\n'),
                Some('t') => Ok('\t'),
                Some('r') => Ok('\r'),
                Some('\\') => Ok('\\'),
                Some('\'') => Ok('\''),
                Some('"') => Ok('"'),
                Some(d @ '0'..='7') => {
                    let mut octal = String::new();
                    octal.push(d);
                    for _ in 0..2 {
                        match chars.next() {
                            Some(c @ '0'..='7') => octal.push(c),
                            _ => break,
                        }
                    }
                    u32::from_str_radix(&octal, 8)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| Error::SyntaxError(format!("invalid octal escape: \\{}", octal)))
                }
                Some('a') => Ok('\x07'),
                Some('b') => Ok('\x08'),
                Some('f') => Ok('\x0C'),
                Some('v') => Ok('\x0B'),
                Some('x') => {
                    let hex: String = chars.take(2).collect();
                    u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| Error::SyntaxError(format!("invalid hex escape: \\x{}", hex)))
                }
                Some('u') => {
                    let hex: String = chars.take(4).collect();
                    u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| Error::SyntaxError(format!("invalid unicode escape: \\u{}", hex)))
                }
                Some('U') => {
                    let hex: String = chars.take(8).collect();
                    u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| Error::SyntaxError(format!("invalid unicode escape: \\U{}", hex)))
                }
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

    pub(crate) fn compile_ident(
        &mut self,
        ident: &ast::Ident,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<GoType, Error> {
        match ident.name.as_str() {
            "true" => {
                out.push(Instruction::I32Const(1));
                return Ok(GoType::Bool);
            }
            "false" => {
                out.push(Instruction::I32Const(0));
                return Ok(GoType::Bool);
            }
            "nil" => {
                out.push(Instruction::I32Const(0));
                return Ok(GoType::Int32);
            }
            _ => {}
        }

        if let Some(&gc_ref_idx) = locals.gc_string_locals.get(&ident.name) {
            out.push(Instruction::LocalGet(gc_ref_idx));
            return Ok(GoType::String);
        }

        if let Some(&(ptr_local, len_local)) = locals.string_locals.get(&ident.name) {
            out.push(Instruction::LocalGet(ptr_local));
            out.push(Instruction::LocalGet(len_local));
            return Ok(GoType::String);
        }

        if let Some(&(offset, vt)) = locals.memory_backed_vars.get(&ident.name) {
            if let Some(sf) = &self.current_stack_frame {
                if let Some(fb) = sf.frame_base_local {
                    out.push(Instruction::LocalGet(fb));
                    if offset > 0 {
                        out.push(Instruction::I32Const(offset as i32));
                        out.push(Instruction::I32Add);
                    }
                    let (_, align) = Self::elem_size_and_align(vt);
                    Self::emit_typed_load(vt, 0, align, out);
                    return Ok(GoType::from_val_type(vt));
                }
            }
        }

        if let Some(idx) = locals.find(&ident.name) {
            let vt = locals.find_type(&ident.name).unwrap_or(ValType::I64);
            out.push(Instruction::LocalGet(idx));
            return Ok(GoType::from_val_type(vt));
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
                return Ok(GoType::from_val_type(vt));
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
                return Ok(GoType::from_val_type(vt));
            }
        }

        if let Some(cv) = self.constants.get(&ident.name) {
            let cv_vt = match cv {
                ConstValue::I64(_) => GoType::UntypedInt,
                ConstValue::F64(_) => GoType::UntypedFloat,
                ConstValue::Bool(_) => GoType::Bool,
                ConstValue::Str(_) => GoType::String,
                ConstValue::Complex128(_, _) => GoType::Complex128,
            };
            match cv {
                ConstValue::I64(v) => {
                    let v = *v;
                    if v > u64::MAX as i128 || v < i64::MIN as i128 {
                        return Err(Error::SyntaxError(format!(
                            "constant {} overflows integer", ident.name
                        )));
                    }
                    out.push(Instruction::I64Const(v as i64));
                }
                ConstValue::F64(v) => out.push(Instruction::F64Const((*v).into())),
                ConstValue::Bool(v) => out.push(Instruction::I32Const(*v as i32)),
                ConstValue::Str(s) => {
                    let bytes = s.as_bytes();
                    let len = bytes.len() as i32;

                    if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                        let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
                        for &byte in bytes.iter() {
                            out.push(Instruction::I32Const(byte as i32));
                        }
                        out.push(Instruction::ArrayNewFixed {
                            array_type_index: byte_array_idx,
                            array_size: bytes.len() as u32,
                        });
                        out.push(Instruction::I32Const(len));
                        out.push(Instruction::StructNew(go_string_idx));
                    } else {
                        let ptr_local = locals.add_local(
                            &format!("__const_str_ptr_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::I32Const(len));
                        out.push(Instruction::Call(self.alloc_func_idx()?));
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
                }
                ConstValue::Complex128(real, imag) => {
                    if let Some(gc_idx) = self.gc_builtin_types.complex128 {
                        out.push(Instruction::F64Const((*real).into()));
                        out.push(Instruction::F64Const((*imag).into()));
                        out.push(Instruction::StructNew(gc_idx));
                    } else {
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
                        out.push(Instruction::F64Const((*real).into()));
                        out.push(Instruction::F64Store(MemArg { offset: 0, align: float_align, memory_index: 0 }));

                        out.push(Instruction::LocalGet(ptr_local));
                        out.push(Instruction::F64Const((*imag).into()));
                        out.push(Instruction::F64Store(MemArg { offset: 8, align: float_align, memory_index: 0 }));

                        out.push(Instruction::LocalGet(ptr_local));
                    }
                }
            }
            return Ok(cv_vt);
        }

        if let Some(&(global_idx, gvt)) = self.resolve_global_var(&ident.name) {
            if matches!(gvt, ValType::Ref(_)) {
                out.push(Instruction::GlobalGet(global_idx));
                return Ok(GoType::String);
            }
            let resolved_name = self.resolve_global_var_name(&ident.name);
            let len_key = format!("{}_1", resolved_name);
            if let Some(&(len_global_idx, _)) = self.global_vars.get(&len_key) {
                out.push(Instruction::GlobalGet(global_idx));
                out.push(Instruction::GlobalGet(len_global_idx));
                return Ok(GoType::String);
            } else {
                out.push(Instruction::GlobalGet(global_idx));
                return Ok(GoType::from_val_type(gvt));
            }
        }

        let resolved_struct = self.resolve_struct_in_pkg(&ident.name);
        let is_type_or_package = self.struct_defs.contains_key(&resolved_struct)
            || self.is_known_package(&ident.name);
        if is_type_or_package {
            out.push(Instruction::I64Const(0));
            return Ok(GoType::Void);
        }

        if let Some(fi) = self.find_func_in_pkg(&ident.name) {
            let idx = fi.wasm_func_idx;
            out.push(Instruction::I64Const(idx as i64));
            self.last_func_value_idx = Some(idx);
            return Ok(GoType::Func);
        }

        Err(Error::InternalError(format!(
            "undefined identifier: {}",
            ident.name
        )))
    }

    pub(crate) fn is_known_package(&self, name: &str) -> bool {
        self.compiled_packages.contains(name)
            || matches!(
                name,
                "fmt" | "math" | "strings" | "strconv" | "sort" | "unicode"
                    | "utf8" | "bytes" | "encoding" | "time"
            )
    }

    pub(crate) fn is_complex64_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> bool {
        if let ast::Expression::Ident(ident) = expr {
            matches!(locals.get_var_type(&ident.name), Some(DefineType::Complex64))
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

    pub(crate) fn is_complex_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> bool {
        if let ast::Expression::Ident(ident) = expr {
            matches!(locals.get_var_type(&ident.name), Some(DefineType::Complex64 | DefineType::Complex128))
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

    pub(crate) fn emit_complex_binop(
        &mut self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        op: Operator,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let is_c64 = self.is_complex64_expr(lhs, locals);
        let float_vt = if is_c64 { ValType::F32 } else { ValType::F64 };
        let gc_idx = if is_c64 { self.gc_builtin_types.complex64 } else { self.gc_builtin_types.complex128 };

        let ar = locals.add_local(&format!("__cx_ar_{}", locals.locals.len()), float_vt);
        let ai = locals.add_local(&format!("__cx_ai_{}", locals.locals.len()), float_vt);
        let br = locals.add_local(&format!("__cx_br_{}", locals.locals.len()), float_vt);
        let bi = locals.add_local(&format!("__cx_bi_{}", locals.locals.len()), float_vt);

        if let Some(gc_idx) = gc_idx {
            self.compile_expression(lhs, out, locals)?;
            let lref = locals.add_local(&format!("__cx_lr_{}", locals.locals.len()), Self::gc_ref_val_type(gc_idx));
            out.push(Instruction::LocalSet(lref));
            out.push(Instruction::LocalGet(lref));
            out.push(Instruction::StructGet { struct_type_index: gc_idx, field_index: 0 });
            out.push(Instruction::LocalSet(ar));
            out.push(Instruction::LocalGet(lref));
            out.push(Instruction::StructGet { struct_type_index: gc_idx, field_index: 1 });
            out.push(Instruction::LocalSet(ai));

            self.compile_expression(rhs, out, locals)?;
            let rref = locals.add_local(&format!("__cx_rr_ref_{}", locals.locals.len()), Self::gc_ref_val_type(gc_idx));
            out.push(Instruction::LocalSet(rref));
            out.push(Instruction::LocalGet(rref));
            out.push(Instruction::StructGet { struct_type_index: gc_idx, field_index: 0 });
            out.push(Instruction::LocalSet(br));
            out.push(Instruction::LocalGet(rref));
            out.push(Instruction::StructGet { struct_type_index: gc_idx, field_index: 1 });
            out.push(Instruction::LocalSet(bi));
        } else {
            let imag_offset = if is_c64 { 4u64 } else { 8u64 };
            let float_align = if is_c64 { 2u32 } else { 3u32 };

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
        }

        match op {
            Operator::Equal => {
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

        let res_r = locals.add_local(&format!("__cx_rr_{}", locals.locals.len()), float_vt);
        let res_i = locals.add_local(&format!("__cx_ri_{}", locals.locals.len()), float_vt);

        match op {
            Operator::Add => {
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
                let denom = locals.add_local(&format!("__cx_d_{}", locals.locals.len()), float_vt);
                out.push(Instruction::LocalGet(br));
                out.push(Instruction::LocalGet(br));
                if is_c64 { out.push(Instruction::F32Mul); } else { out.push(Instruction::F64Mul); }
                out.push(Instruction::LocalGet(bi));
                out.push(Instruction::LocalGet(bi));
                if is_c64 { out.push(Instruction::F32Mul); } else { out.push(Instruction::F64Mul); }
                if is_c64 { out.push(Instruction::F32Add); } else { out.push(Instruction::F64Add); }
                out.push(Instruction::LocalSet(denom));

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

        if let Some(gc_idx) = gc_idx {
            out.push(Instruction::LocalGet(res_r));
            out.push(Instruction::LocalGet(res_i));
            out.push(Instruction::StructNew(gc_idx));
        } else {
            let imag_offset = if is_c64 { 4u64 } else { 8u64 };
            let float_align = if is_c64 { 2u32 } else { 3u32 };
            let total_size = if is_c64 { 8i32 } else { 16i32 };

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
        }
        Ok(())
    }

    pub(crate) fn is_selector_slice_field(&self, sel: &ast::Selector, locals: &LocalAlloc) -> bool {
        if let Some(type_name) = self.infer_struct_type_from_expr(sel.x.as_ref(), locals) {
            if let Some(struct_def) = self.struct_defs.get(&type_name) {
                if let Some(field) = struct_def.find_field(&sel.sel.name) {
                    return field.is_slice_field();
                }
            }
        }
        false
    }

    pub(crate) fn is_selector_map_field(&self, sel: &ast::Selector, locals: &LocalAlloc) -> bool {
        if let Some(type_name) = self.infer_struct_type_from_expr(sel.x.as_ref(), locals) {
            if let Some(struct_def) = self.struct_defs.get(&type_name) {
                if let Some(field) = struct_def.find_field(&sel.sel.name) {
                    return field.is_map_field();
                }
            }
        }
        false
    }

    pub(crate) fn is_string_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> bool {
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
                    let resolved = self.resolve_type_name(&ident.name);
                    if resolved == "string" {
                        return true;
                    }
                    if let Some(fi) = self.functions.iter().find(|f| f.name == ident.name && f.recv_type.is_none()) {
                        if fi.result_define_types.first().map_or(false, |dt| dt.is_string_type()) {
                            return true;
                        }
                    }
                    if let Some(fi) = self.find_func_in_pkg(&ident.name) {
                        if fi.result_define_types.first().map_or(false, |dt| dt.is_string_type()) {
                            return true;
                        }
                    }
                    false
                } else if let ast::Expression::Selector(sel) = call.func.as_ref() {
                    if let ast::Expression::Ident(recv) = sel.x.as_ref() {
                        if matches!(locals.get_var_type(&recv.name), Some(DefineType::Struct { name, .. }) if name == "Context") {
                            return matches!(sel.sel.name.as_str(), "User" | "Schema" | "Database" | "QueryID" | "Config");
                        }
                        if self.is_interface_var(&recv.name, locals) {
                            let method_name = &sel.sel.name;
                            for fi in &self.functions {
                                if fi.recv_type.is_some() && fi.name.ends_with(&format!(".{}", method_name)) {
                                    if fi.result_define_types.first().map_or(false, |dt| dt.is_string_type()) {
                                        return true;
                                    }
                                    return fi.results.len() == 2
                                        && fi.results[0] == WasmType::I32
                                        && fi.results[1] == WasmType::I32;
                                }
                            }
                        }
                    }
                    if let Some(qualified) = self.resolve_selector_method_name(sel, locals) {
                        if let Some(fi) = self.functions.iter().find(|f| f.name == qualified) {
                            if fi.result_define_types.first().map_or(false, |dt| dt.is_string_type()) {
                                return true;
                            }
                            if fi.results.len() == 2
                                && fi.results[0] == WasmType::I32
                                && fi.results[1] == WasmType::I32
                                && fi.result_define_types.first().map_or(false, |dt| dt.is_string_type())
                            {
                                return true;
                            }
                        }
                    }
                    if self.is_interface_field_selector(sel.x.as_ref(), locals) {
                        let method_suffix = format!(".{}", sel.sel.name);
                        for fi in &self.functions {
                            if fi.recv_type.is_some() && fi.name.ends_with(&method_suffix) {
                                if fi.result_define_types.first().map_or(false, |dt| dt.is_string_type()) {
                                    return true;
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
                locals.is_var_type_string(&ident.name)
                    || locals.gc_string_locals.contains_key(&ident.name)
                    || self.global_vars.contains_key(&format!("{}_1", ident.name))
                    || self.global_vars.contains_key(&format!("{}_1", self.resolve_global_var_name(&ident.name)))
                    || self.resolve_global_var(&ident.name).map_or(false, |&(_, vt)| matches!(vt, ValType::Ref(_)))
                    || matches!(self.constants.get(&ident.name), Some(ConstValue::Str(_)))
                    || {
                        let qualified = self.qualify_pkg_name(&ident.name);
                        matches!(self.constants.get(&qualified), Some(ConstValue::Str(_)))
                    }
            }
            ast::Expression::Paren(p) => self.is_string_expr(&p.expr, locals),
            ast::Expression::Operation(op) if op.op == Operator::Add && op.y.is_some() => {
                self.is_string_expr(&op.x, locals)
                    && self.is_string_expr(op.y.as_ref().unwrap(), locals)
            }
            ast::Expression::Slice(slice) => {
                self.is_string_expr(&slice.left, locals)
            }
            ast::Expression::Index(idx) => {
                if let Some(left) = idx.left.as_deref() {
                    if let ast::Expression::Ident(ident) = left {
                        if let Some(&(arr_elem_vt, ..)) = locals.array_info.get(&ident.name) {
                            if let Some(gc_idx) = self.gc_builtin_types.go_string {
                                return arr_elem_vt == Self::gc_ref_val_type(gc_idx);
                            }
                        }
                    }
                }
                false
            }
            ast::Expression::Selector(sel) => {
                if let Some(type_name) = self.infer_struct_type_from_expr(sel.x.as_ref(), locals) {
                    if let Some(sdef) = self.struct_defs.get(&type_name) {
                        if let Some(field) = sdef.find_field(&sel.sel.name) {
                            return field.is_string_field();
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }

    pub(crate) fn is_unsigned_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> bool {
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
            ast::Expression::Operation(op) => {
                self.is_unsigned_expr(&op.x, locals)
                    || op.y.as_ref().map_or(false, |y| self.is_unsigned_expr(y, locals))
            }
            _ => false,
        }
    }

    pub(crate) fn is_unsigned_type_name(name: &str) -> bool {
        matches!(name, "uint" | "uint64" | "uint32" | "uint8" | "uint16" | "byte" | "uintptr")
    }

    pub(crate) fn find_captured_closure(&self, name: &str) -> Option<(u32, u32, Vec<(String, u32, ValType)>)> {
        let cc = self.closure_captures.as_ref()?;
        let (func_idx, env_local) = cc.outer_closure_info.get(name)?;
        let env_captures = cc.outer_closure_env_captures.get(name).cloned().unwrap_or_default();
        Some((*func_idx, *env_local, env_captures))
    }

    pub(crate) fn find_or_add_capture(&mut self, name: &str) -> Option<(u32, ValType)> {
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

    pub(crate) fn get_struct_type_of_expr<'b>(&self, expr: &ast::Expression, locals: &'b LocalAlloc) -> Option<&'b str> {
        match expr {
            ast::Expression::Ident(ident) => {
                let st = locals.get_var_struct_name(&ident.name)?;
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

    pub(crate) fn get_comparable_struct_type(
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
            if field.is_slice_or_map_field() {
                return None;
            }
        }
        Some(lhs_type.to_string())
    }

    pub(crate) fn emit_struct_compare(
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

        let gc_mode = sdef.gc_type_idx.is_some();
        let gc_type_idx = sdef.gc_type_idx.unwrap_or(0);

        if gc_mode {
            let ref_vt = Self::gc_ref_val_type(gc_type_idx);
            self.compile_expression(lhs, out, locals)?;
            let lhs_ref = locals.add_local(
                &format!("__scmp_l_{}", locals.locals.len()), ref_vt,
            );
            out.push(Instruction::LocalSet(lhs_ref));

            self.compile_expression(rhs, out, locals)?;
            let rhs_ref = locals.add_local(
                &format!("__scmp_r_{}", locals.locals.len()), ref_vt,
            );
            out.push(Instruction::LocalSet(rhs_ref));

            let result = locals.add_local(
                &format!("__scmp_res_{}", locals.locals.len()), ValType::I32,
            );
            out.push(Instruction::I32Const(1));
            out.push(Instruction::LocalSet(result));

            out.push(Instruction::Block(BlockType::Empty));
            for field in &sdef.fields {
                if field.is_string_field() && self.gc_builtin_types.go_string.is_some() {
                    let go_string_idx = self.gc_builtin_types.go_string.unwrap();
                    let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
                    let gs_vt = Self::gc_ref_val_type(go_string_idx);
                    let ba_vt = Self::gc_ref_val_type(byte_array_idx);

                    let l_gs = locals.add_local(&format!("__scf_lgs_{}", locals.locals.len()), gs_vt);
                    let r_gs = locals.add_local(&format!("__scf_rgs_{}", locals.locals.len()), gs_vt);

                    out.push(Instruction::LocalGet(lhs_ref));
                    out.push(Instruction::StructGet { struct_type_index: gc_type_idx, field_index: field.field_index });
                    out.push(Instruction::LocalSet(l_gs));
                    out.push(Instruction::LocalGet(rhs_ref));
                    out.push(Instruction::StructGet { struct_type_index: gc_type_idx, field_index: field.field_index });
                    out.push(Instruction::LocalSet(r_gs));

                    let l_len = locals.add_local(&format!("__scf_ll_{}", locals.locals.len()), ValType::I32);
                    let r_len = locals.add_local(&format!("__scf_rl_{}", locals.locals.len()), ValType::I32);
                    let l_arr = locals.add_local(&format!("__scf_la_{}", locals.locals.len()), ba_vt);
                    let r_arr = locals.add_local(&format!("__scf_ra_{}", locals.locals.len()), ba_vt);

                    out.push(Instruction::LocalGet(l_gs));
                    out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
                    out.push(Instruction::LocalSet(l_len));
                    out.push(Instruction::LocalGet(r_gs));
                    out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
                    out.push(Instruction::LocalSet(r_len));

                    out.push(Instruction::LocalGet(l_len));
                    out.push(Instruction::LocalGet(r_len));
                    out.push(Instruction::I32Ne);
                    out.push(Instruction::If(BlockType::Empty));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::LocalSet(result));
                    out.push(Instruction::Br(1));
                    out.push(Instruction::End);

                    out.push(Instruction::LocalGet(l_gs));
                    out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
                    out.push(Instruction::LocalSet(l_arr));
                    out.push(Instruction::LocalGet(r_gs));
                    out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
                    out.push(Instruction::LocalSet(r_arr));

                    let si = locals.add_local(&format!("__scf_si_{}", locals.locals.len()), ValType::I32);
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::LocalSet(si));
                    out.push(Instruction::Block(BlockType::Empty));
                    out.push(Instruction::Loop(BlockType::Empty));
                    out.push(Instruction::LocalGet(si));
                    out.push(Instruction::LocalGet(l_len));
                    out.push(Instruction::I32GeU);
                    out.push(Instruction::BrIf(1));
                    out.push(Instruction::LocalGet(l_arr));
                    out.push(Instruction::LocalGet(si));
                    out.push(Instruction::ArrayGet(byte_array_idx));
                    out.push(Instruction::LocalGet(r_arr));
                    out.push(Instruction::LocalGet(si));
                    out.push(Instruction::ArrayGet(byte_array_idx));
                    out.push(Instruction::I32Ne);
                    out.push(Instruction::If(BlockType::Empty));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::LocalSet(result));
                    out.push(Instruction::Br(3));
                    out.push(Instruction::End);
                    out.push(Instruction::LocalGet(si));
                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::I32Add);
                    out.push(Instruction::LocalSet(si));
                    out.push(Instruction::Br(0));
                    out.push(Instruction::End); // loop
                    out.push(Instruction::End); // block
                } else {
                    out.push(Instruction::LocalGet(lhs_ref));
                    out.push(Instruction::StructGet { struct_type_index: gc_type_idx, field_index: field.field_index });
                    out.push(Instruction::LocalGet(rhs_ref));
                    out.push(Instruction::StructGet { struct_type_index: gc_type_idx, field_index: field.field_index });
                    let ne_instr = match field.wasm_type {
                        WasmType::I64 => Instruction::I64Ne,
                        WasmType::F64 => Instruction::F64Ne,
                        WasmType::F32 => Instruction::F32Ne,
                        _ => Instruction::I32Ne,
                    };
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
        } else {
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
                if field.is_string_field() {
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

                    out.push(Instruction::LocalGet(l_slen));
                    out.push(Instruction::LocalGet(r_slen));
                    out.push(Instruction::I32Ne);
                    out.push(Instruction::If(BlockType::Empty));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::LocalSet(result));
                    out.push(Instruction::Br(1));
                    out.push(Instruction::End);

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
                    out.push(Instruction::Br(3));
                    out.push(Instruction::End);
                    out.push(Instruction::LocalGet(si));
                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::I32Add);
                    out.push(Instruction::LocalSet(si));
                    out.push(Instruction::Br(0));
                    out.push(Instruction::End); // loop
                    out.push(Instruction::End); // block
                } else {
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
                        WasmType::I32 | WasmType::Ref(_) => (
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
        }
        Ok(())
    }

    pub(crate) fn get_array_type_of_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> Option<(ValType, u32, i32, u32)> {
        match expr {
            ast::Expression::Ident(ident) => locals.array_info.get(&ident.name).copied(),
            ast::Expression::Paren(p) => self.get_array_type_of_expr(&p.expr, locals),
            _ => None,
        }
    }

    pub(crate) fn get_comparable_array_type(
        &self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        locals: &LocalAlloc,
    ) -> Option<(ValType, u32)> {
        let (lhs_vt, lhs_len, ..) = self.get_array_type_of_expr(lhs, locals)?;
        let (rhs_vt, rhs_len, ..) = self.get_array_type_of_expr(rhs, locals)?;
        if lhs_vt != rhs_vt || lhs_len != rhs_len {
            return None;
        }
        Some((lhs_vt, lhs_len))
    }

    pub(crate) fn emit_array_compare(
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

    pub(crate) fn compile_operation(
        &mut self,
        op: &ast::Operation,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<GoType, Error> {
        if let Some(ref y) = op.y {
            if op.op == Operator::Add
                && self.is_string_expr(&op.x, locals)
                && self.is_string_expr(y, locals)
            {
                self.emit_string_concat(&op.x, y, out, locals)?;
                return Ok(GoType::String);
            }

            if matches!(op.op, Operator::Equal | Operator::NotEqual | Operator::Less | Operator::Greater | Operator::LessEqual | Operator::GreaterEqual)
                && self.is_string_expr(&op.x, locals)
                && self.is_string_expr(y, locals)
            {
                self.emit_string_compare(&op.x, y, op.op, out, locals)?;
                return Ok(GoType::Bool);
            }

            // Complex number arithmetic
            if self.is_complex_expr(&op.x, locals) && self.is_complex_expr(y, locals) {
                self.emit_complex_binop(&op.x, y, op.op, out, locals)?;
                return Ok(GoType::Complex128);
            }

            // Interface nil comparison: err == nil or err != nil
            if op.op == Operator::Equal || op.op == Operator::NotEqual {
                let (iface_name, is_nil_cmp) = self.check_interface_nil_cmp(&op.x, y, locals);
                if is_nil_cmp {
                    if let Some(tid_local) = self.get_iface_type_id_local(&iface_name, locals) {
                        out.push(Instruction::LocalGet(tid_local));
                        if op.op == Operator::Equal {
                            out.push(Instruction::I32Eqz);
                        } else {
                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::I32Ne);
                        }
                        return Ok(GoType::Bool);
                    }
                    let resolved = self.resolve_global_var_name(&iface_name);
                    let tid_key = format!("{}_tid", resolved);
                    if let Some(&(tid_global, _)) = self.global_vars.get(&tid_key) {
                        out.push(Instruction::GlobalGet(tid_global));
                        if op.op == Operator::Equal {
                            out.push(Instruction::I32Eqz);
                        } else {
                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::I32Ne);
                        }
                        return Ok(GoType::Bool);
                    }
                }

                // Interface-to-interface equality: err == target
                if let (ast::Expression::Ident(lhs_id), ast::Expression::Ident(rhs_id)) = (op.x.as_ref(), y.as_ref()) {
                    let lhs_is_iface = self.is_interface_var(&lhs_id.name, locals);
                    let rhs_is_iface = self.is_interface_var(&rhs_id.name, locals);
                    if lhs_is_iface && rhs_is_iface {
                        if let (Some(lhs_tid), Some(rhs_tid)) = (
                            self.get_iface_type_id_local(&lhs_id.name, locals),
                            self.get_iface_type_id_local(&rhs_id.name, locals),
                        ) {
                            let lhs_data = locals.find(&lhs_id.name).ok_or_else(|| {
                                Error::InternalError(format!("variable '{}' not found", lhs_id.name))
                            })?;
                            let rhs_data = locals.find(&rhs_id.name).ok_or_else(|| {
                                Error::InternalError(format!("variable '{}' not found", rhs_id.name))
                            })?;
                            // tid_a == tid_b AND data_a == data_b (pointer identity)
                            // Note: __rt_eq is not used here because interface boxing
                            // stores a pointer in a heap cell, and comparison functions
                            // expect direct data pointers. Until boxing is reworked,
                            // pointer identity is the correct semantic for interfaces.
                            out.push(Instruction::LocalGet(lhs_tid));
                            out.push(Instruction::LocalGet(rhs_tid));
                            out.push(Instruction::I32Eq);
                            out.push(Instruction::If(BlockType::Result(ValType::I32)));
                            out.push(Instruction::LocalGet(lhs_data));
                            out.push(Instruction::LocalGet(rhs_data));
                            out.push(Instruction::I32Eq);
                            out.push(Instruction::Else);
                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::End);
                            if op.op == Operator::NotEqual {
                                out.push(Instruction::I32Eqz);
                            }
                            return Ok(GoType::Bool);
                        }
                    }
                }

                let lhs_is_any_iface = self.is_interface_var_expr(&op.x, locals)
                    || self.is_interface_field_selector(&op.x, locals);
                let rhs_is_any_iface = self.is_interface_var_expr(y, locals)
                    || self.is_interface_field_selector(y, locals);
                if lhs_is_any_iface || rhs_is_any_iface {
                    let (lhs_data, lhs_tid) = self.compile_iface_expr_to_locals(&op.x, out, locals)?;
                    let (rhs_data, rhs_tid) = self.compile_iface_expr_to_locals(y, out, locals)?;
                    // tid_a == tid_b AND data_a == data_b (pointer identity)
                    out.push(Instruction::LocalGet(lhs_tid));
                    out.push(Instruction::LocalGet(rhs_tid));
                    out.push(Instruction::I32Eq);
                    out.push(Instruction::If(BlockType::Result(ValType::I32)));
                    out.push(Instruction::LocalGet(lhs_data));
                    out.push(Instruction::LocalGet(rhs_data));
                    out.push(Instruction::I32Eq);
                    out.push(Instruction::Else);
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::End);
                    if op.op == Operator::NotEqual {
                        out.push(Instruction::I32Eqz);
                    }
                    return Ok(GoType::Bool);
                }
            }

            // Struct comparison: s1 == s2 or s1 != s2
            if matches!(op.op, Operator::Equal | Operator::NotEqual) {
                if let Some(struct_type) = self.get_comparable_struct_type(&op.x, y, locals) {
                    self.emit_struct_compare(&op.x, y, &struct_type, op.op, out, locals)?;
                    return Ok(GoType::Bool);
                }
                if let (Some(lhs_st), Some(_rhs_st)) = (
                    self.get_struct_type_of_expr(&op.x, locals).map(|s| s.to_string()),
                    self.get_struct_type_of_expr(y, locals).map(|s| s.to_string()),
                ) {
                    if let Some(sdef) = self.struct_defs.get(&lhs_st) {
                        let uncomparable: Vec<&str> = sdef.fields.iter()
                            .filter(|f| f.is_slice_or_map_field())
                            .map(|f| f.name.as_str())
                            .collect();
                        if !uncomparable.is_empty() {
                            return Err(Error::InternalError(format!(
                                "struct {} cannot be compared: contains uncomparable field(s): {}",
                                lhs_st,
                                uncomparable.join(", ")
                            )));
                        }
                    }
                }
                if let Some((elem_vt, arr_len)) = self.get_comparable_array_type(&op.x, y, locals) {
                    self.emit_array_compare(&op.x, y, elem_vt, arr_len, op.op, out, locals)?;
                    return Ok(GoType::Bool);
                }
            }

            if op.op == Operator::AndAnd {
                self.compile_expression(&op.x, out, locals)?;
                out.push(Instruction::If(BlockType::Result(ValType::I32)));
                self.compile_expression(y, out, locals)?;
                out.push(Instruction::Else);
                out.push(Instruction::I32Const(0));
                out.push(Instruction::End);
                return Ok(GoType::Bool);
            }

            if op.op == Operator::OrOr {
                self.compile_expression(&op.x, out, locals)?;
                out.push(Instruction::If(BlockType::Result(ValType::I32)));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::Else);
                self.compile_expression(y, out, locals)?;
                out.push(Instruction::End);
                return Ok(GoType::Bool);
            }

            let lhs_type = self.infer_val_type(&op.x, locals);
            let rhs_type = self.infer_val_type(y, locals);

            // Go untyped constant rule: a float literal like 1e9 that represents a
            // whole number should be treated as an integer when the other operand is
            // an integer type. This enables `int64 % 1e9` to use integer ops.
            let lhs_int_float = if let ast::Expression::BasicLit(lit) = op.x.as_ref() {
                if lit.kind == LitKind::Float { Self::is_integer_representable_float(&lit.value) } else { None }
            } else { None };
            let rhs_int_float = if let ast::Expression::BasicLit(lit) = y.as_ref() {
                if lit.kind == LitKind::Float { Self::is_integer_representable_float(&lit.value) } else { None }
            } else { None };

            let lhs_is_int = matches!(lhs_type, ValType::I64 | ValType::I32);
            let rhs_is_int = matches!(rhs_type, ValType::I64 | ValType::I32);

            // Only promote float literal to int when the OTHER operand is genuinely integer
            // (a variable, function call, etc.), not when both sides are float literals.
            if (lhs_type == ValType::F64 && rhs_is_int && lhs_int_float.is_some())
                || (rhs_type == ValType::F64 && lhs_is_int && rhs_int_float.is_some())
            {
                // Both should be compiled as integers
                if let Some(ival) = lhs_int_float {
                    out.push(Instruction::I64Const(ival));
                } else {
                    self.compile_expression(&op.x, out, locals)?;
                    if lhs_type == ValType::I32 {
                        let is_unsigned = self.is_unsigned_expr(&op.x, locals);
                        out.push(if is_unsigned { Instruction::I64ExtendI32U } else { Instruction::I64ExtendI32S });
                    }
                }
                if let Some(ival) = rhs_int_float {
                    out.push(Instruction::I64Const(ival));
                } else {
                    self.compile_expression(y, out, locals)?;
                    if rhs_type == ValType::I32 {
                        let is_unsigned = self.is_unsigned_expr(y, locals);
                        out.push(if is_unsigned { Instruction::I64ExtendI32U } else { Instruction::I64ExtendI32S });
                    }
                }
                let is_unsigned = self.is_unsigned_expr(&op.x, locals) || self.is_unsigned_expr(y, locals);
                self.emit_i64_op_signed(op.op, !is_unsigned, out, locals)?;
                return Ok(if matches!(op.op, Operator::Equal | Operator::NotEqual | Operator::Less | Operator::Greater | Operator::LessEqual | Operator::GreaterEqual) { GoType::Bool } else { GoType::Int64 });
            }

            if lhs_type == ValType::F32 && rhs_type == ValType::F32 {
                self.compile_expression(&op.x, out, locals)?;
                self.compile_expression(y, out, locals)?;
                self.emit_f32_op(op.op, out)?;
                return Ok(if matches!(op.op, Operator::Equal | Operator::NotEqual | Operator::Less | Operator::Greater | Operator::LessEqual | Operator::GreaterEqual) { GoType::Bool } else { GoType::Float32 });
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
                    } else if lhs_type == ValType::F64 {
                        out.push(Instruction::F32DemoteF64);
                    }
                }
                out.extend(rhs_buf);
                if rhs_type != ValType::F32 {
                    if rhs_type == ValType::I32 {
                        out.push(Instruction::F32ConvertI32S);
                    } else if rhs_type == ValType::I64 {
                        out.push(Instruction::F32ConvertI64S);
                    } else if rhs_type == ValType::F64 {
                        out.push(Instruction::F32DemoteF64);
                    }
                }
                self.emit_f32_op(op.op, out)?;
                return Ok(if matches!(op.op, Operator::Equal | Operator::NotEqual | Operator::Less | Operator::Greater | Operator::LessEqual | Operator::GreaterEqual) { GoType::Bool } else { GoType::Float32 });
            }

            if lhs_type == ValType::F64 || rhs_type == ValType::F64 {
                let lhs_const = if lhs_type != ValType::F64 { self.try_eval_const_expr(&op.x) } else { None };
                let rhs_const = if rhs_type != ValType::F64 { self.try_eval_const_expr(y) } else { None };

                if let Some(ref cv) = lhs_const {
                    let fv = match cv {
                        ConstValue::I64(v) => *v as f64,
                        ConstValue::F64(v) => *v,
                        _ => 0.0,
                    };
                    out.push(Instruction::F64Const(fv.into()));
                } else {
                    let mut lhs_buf = Vec::new();
                    self.compile_expression(&op.x, &mut lhs_buf, locals)?;
                    out.extend(lhs_buf);
                    if lhs_type != ValType::F64 {
                        if lhs_type == ValType::F32 {
                            out.push(Instruction::F64PromoteF32);
                        } else if lhs_type == ValType::I32 {
                            out.push(Instruction::F64ConvertI32S);
                        } else {
                            out.push(Instruction::F64ConvertI64S);
                        }
                    }
                }

                if let Some(ref cv) = rhs_const {
                    let fv = match cv {
                        ConstValue::I64(v) => *v as f64,
                        ConstValue::F64(v) => *v,
                        _ => 0.0,
                    };
                    out.push(Instruction::F64Const(fv.into()));
                } else {
                    let mut rhs_buf = Vec::new();
                    self.compile_expression(y, &mut rhs_buf, locals)?;
                    out.extend(rhs_buf);
                    if rhs_type != ValType::F64 {
                        if rhs_type == ValType::F32 {
                            out.push(Instruction::F64PromoteF32);
                        } else if rhs_type == ValType::I32 {
                            out.push(Instruction::F64ConvertI32S);
                        } else {
                            out.push(Instruction::F64ConvertI64S);
                        }
                    }
                }

                self.emit_f64_op(op.op, out)?;
                return Ok(if matches!(op.op, Operator::Equal | Operator::NotEqual | Operator::Less | Operator::Greater | Operator::LessEqual | Operator::GreaterEqual) { GoType::Bool } else { GoType::Float64 });
            }

            let is_unsigned = if op.op == Operator::Shr {
                self.is_unsigned_expr(&op.x, locals)
            } else {
                self.is_unsigned_expr(&op.x, locals)
                    || self.is_unsigned_expr(y, locals)
            };

            if lhs_type == ValType::I32 && rhs_type == ValType::I32 {
                self.compile_expression(&op.x, out, locals)?;
                self.compile_expression(y, out, locals)?;
                self.emit_i32_op_signed(op.op, !is_unsigned, out, locals)?;
                return Ok(if matches!(op.op, Operator::Equal | Operator::NotEqual | Operator::Less | Operator::Greater | Operator::LessEqual | Operator::GreaterEqual) { GoType::Bool } else { GoType::Int32 });
            }

            // Go untyped constant rule: when one operand is I32 (uint32, int32, etc.)
            // and the other is an integer literal, the literal adopts the I32 type.
            let rhs_is_int_lit = matches!(y.as_ref(), ast::Expression::BasicLit(lit) if lit.kind == LitKind::Integer);
            let lhs_is_int_lit = matches!(op.x.as_ref(), ast::Expression::BasicLit(lit) if lit.kind == LitKind::Integer);

            if lhs_type == ValType::I32 && rhs_type == ValType::I64 && rhs_is_int_lit {
                self.compile_expression(&op.x, out, locals)?;
                if let ast::Expression::BasicLit(lit) = y.as_ref() {
                    let val = Self::parse_go_int(&lit.value).unwrap_or(0) as i32;
                    out.push(Instruction::I32Const(val));
                }
                self.emit_i32_op_signed(op.op, !is_unsigned, out, locals)?;
                return Ok(if matches!(op.op, Operator::Equal | Operator::NotEqual | Operator::Less | Operator::Greater | Operator::LessEqual | Operator::GreaterEqual) { GoType::Bool } else { GoType::Int32 });
            }

            if lhs_type == ValType::I64 && rhs_type == ValType::I32 && lhs_is_int_lit {
                if let ast::Expression::BasicLit(lit) = op.x.as_ref() {
                    let val = Self::parse_go_int(&lit.value).unwrap_or(0) as i32;
                    out.push(Instruction::I32Const(val));
                }
                self.compile_expression(y, out, locals)?;
                self.emit_i32_op_signed(op.op, !is_unsigned, out, locals)?;
                return Ok(if matches!(op.op, Operator::Equal | Operator::NotEqual | Operator::Less | Operator::Greater | Operator::LessEqual | Operator::GreaterEqual) { GoType::Bool } else { GoType::Int32 });
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

            self.emit_i64_op_signed(op.op, !is_unsigned, out, locals)?;
            return Ok(if matches!(op.op, Operator::Equal | Operator::NotEqual | Operator::Less | Operator::Greater | Operator::LessEqual | Operator::GreaterEqual) { GoType::Bool } else { GoType::Int64 });
        }

        // Unary operations
        match op.op {
            Operator::Add => {
                self.compile_expression(&op.x, out, locals)?;
            }
            Operator::Sub => {
                if self.is_complex_expr(&op.x, locals) {
                    let is_c64 = self.is_complex64_expr(&op.x, locals);
                    let gc_idx = if is_c64 { self.gc_builtin_types.complex64 } else { self.gc_builtin_types.complex128 };

                    if let Some(gc_idx) = gc_idx {
                        self.compile_expression(&op.x, out, locals)?;
                        let src_ref = locals.add_local(
                            &format!("__cneg_src_{}", locals.locals.len()),
                            Self::gc_ref_val_type(gc_idx),
                        );
                        out.push(Instruction::LocalSet(src_ref));

                        out.push(Instruction::LocalGet(src_ref));
                        out.push(Instruction::StructGet { struct_type_index: gc_idx, field_index: 0 });
                        if is_c64 { out.push(Instruction::F32Neg); } else { out.push(Instruction::F64Neg); }
                        out.push(Instruction::LocalGet(src_ref));
                        out.push(Instruction::StructGet { struct_type_index: gc_idx, field_index: 1 });
                        if is_c64 { out.push(Instruction::F32Neg); } else { out.push(Instruction::F64Neg); }
                        out.push(Instruction::StructNew(gc_idx));
                    } else {
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
                    }
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
                let is_memory_backed = if let ast::Expression::Ident(id) = &*op.x {
                    locals.memory_backed_vars.contains_key(&id.name)
                } else {
                    false
                };

                if is_memory_backed {
                    if let ast::Expression::Ident(id) = &*op.x {
                        if let Some(&(mb_offset, _mb_vt)) = locals.memory_backed_vars.get(&id.name) {
                            if let Some(sf) = &self.current_stack_frame {
                                if let Some(fb) = sf.frame_base_local {
                                    out.push(Instruction::LocalGet(fb));
                                    if mb_offset > 0 {
                                        out.push(Instruction::I32Const(mb_offset as i32));
                                        out.push(Instruction::I32Add);
                                    }
                                }
                            }
                        }
                    }
                } else {

                let is_struct_var = if let ast::Expression::Ident(id) = &*op.x {
                    locals.get_var_type(&id.name).map_or(false, |dt| {
                        matches!(dt, DefineType::Slice(_) | DefineType::Map(_, _))
                            || dt.resolved_name().map_or(false, |n| self.struct_defs.contains_key(n))
                    })
                } else {
                    false
                };
                let is_composite = matches!(&*op.x, ast::Expression::CompositeLit(_));

                // &slice[i] or &array[i]: compute element address in-place
                let is_index_addr = if let ast::Expression::Index(idx) = &*op.x {
                    if let Some(ast::Expression::Ident(id)) = idx.left.as_deref() {
                        locals.get_var_type(&id.name).map_or(false, |dt| {
                            matches!(dt, DefineType::Slice(_) | DefineType::Array { .. })
                        }) || locals.array_info.contains_key(&id.name)
                            || locals.slice_elem_types.contains_key(&id.name)
                    } else { false }
                } else { false };

                if is_index_addr {
                    if let ast::Expression::Index(idx) = &*op.x {
                        let is_gc_struct_elem = if let Some(ast::Expression::Ident(id)) = idx.left.as_deref() {
                            locals.slice_elem_struct_types.get(&id.name)
                                .and_then(|st| self.struct_defs.get(st))
                                .map_or(false, |sd| sd.gc_type_idx.is_some())
                        } else { false };

                        if is_gc_struct_elem {
                            self.compile_index(idx, out, locals)?;
                        } else {
                            let (elem_vt, _align) = self.compile_index_store_addr(idx, out, locals)?;
                            let _ = elem_vt;
                        }
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
                } // close else for is_memory_backed
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported unary operator: {:?}",
                    op.op
                )));
            }
        }

        let unary_vt = self.infer_val_type(&ast::Expression::Operation(op.clone()), locals);
        Ok(GoType::from_val_type(unary_vt))
    }

    pub(crate) fn emit_i64_op_signed(
        &self,
        op: Operator,
        signed: bool,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
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
            Operator::Shl => {
                // Go spec: shifts >= 64 produce 0. WASM masks count by 64, so we guard.
                let cnt = locals.add_local(&format!("__shl64_{}", locals.locals.len()), ValType::I64);
                out.push(Instruction::LocalTee(cnt));
                out.push(Instruction::I64Shl);
                out.push(Instruction::I64Const(0));
                out.push(Instruction::LocalGet(cnt));
                out.push(Instruction::I64Const(64));
                out.push(Instruction::I64LtU);
                out.push(Instruction::Select);
            }
            Operator::Shr => {
                if signed {
                    // Signed right shift: count >= 64 gives lhs >> 63 (0 or -1)
                    let cnt = locals.add_local(&format!("__shr64_{}", locals.locals.len()), ValType::I64);
                    let lhs = locals.add_local(&format!("__shr64_l_{}", locals.locals.len()), ValType::I64);
                    out.push(Instruction::LocalSet(cnt));
                    out.push(Instruction::LocalTee(lhs));
                    out.push(Instruction::LocalGet(cnt));
                    out.push(Instruction::I64ShrS);
                    out.push(Instruction::LocalGet(lhs));
                    out.push(Instruction::I64Const(63));
                    out.push(Instruction::I64ShrS);
                    out.push(Instruction::LocalGet(cnt));
                    out.push(Instruction::I64Const(64));
                    out.push(Instruction::I64LtU);
                    out.push(Instruction::Select);
                } else {
                    // Unsigned right shift: count >= 64 gives 0
                    let cnt = locals.add_local(&format!("__shru64_{}", locals.locals.len()), ValType::I64);
                    out.push(Instruction::LocalTee(cnt));
                    out.push(Instruction::I64ShrU);
                    out.push(Instruction::I64Const(0));
                    out.push(Instruction::LocalGet(cnt));
                    out.push(Instruction::I64Const(64));
                    out.push(Instruction::I64LtU);
                    out.push(Instruction::Select);
                }
            }
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

    pub(crate) fn emit_i32_op_signed(
        &self,
        op: Operator,
        signed: bool,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
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
            Operator::Shl => {
                // Go spec: shifts >= 32 produce 0. WASM masks count by 32, so we guard.
                let cnt = locals.add_local(&format!("__shl32_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalTee(cnt));
                out.push(Instruction::I32Shl);
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalGet(cnt));
                out.push(Instruction::I32Const(32));
                out.push(Instruction::I32LtU);
                out.push(Instruction::Select);
            }
            Operator::Shr => {
                if signed {
                    // Signed right shift: count >= 32 gives lhs >> 31 (0 or -1)
                    let cnt = locals.add_local(&format!("__shr32_{}", locals.locals.len()), ValType::I32);
                    let lhs = locals.add_local(&format!("__shr32_l_{}", locals.locals.len()), ValType::I32);
                    out.push(Instruction::LocalSet(cnt));
                    out.push(Instruction::LocalTee(lhs));
                    out.push(Instruction::LocalGet(cnt));
                    out.push(Instruction::I32ShrS);
                    out.push(Instruction::LocalGet(lhs));
                    out.push(Instruction::I32Const(31));
                    out.push(Instruction::I32ShrS);
                    out.push(Instruction::LocalGet(cnt));
                    out.push(Instruction::I32Const(32));
                    out.push(Instruction::I32LtU);
                    out.push(Instruction::Select);
                } else {
                    // Unsigned right shift: count >= 32 gives 0
                    let cnt = locals.add_local(&format!("__shru32_{}", locals.locals.len()), ValType::I32);
                    out.push(Instruction::LocalTee(cnt));
                    out.push(Instruction::I32ShrU);
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::LocalGet(cnt));
                    out.push(Instruction::I32Const(32));
                    out.push(Instruction::I32LtU);
                    out.push(Instruction::Select);
                }
            }
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

    pub(crate) fn emit_f64_op(
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

    pub(crate) fn emit_f32_op(
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

    pub(crate) fn infer_slice_elem_type(&self, type_arg: Option<&ast::Expression>) -> ValType {
        match type_arg {
            Some(ast::Expression::TypeSlice(slice_type)) => {
                self.infer_array_elem_vt(&slice_type.typ)
            }
            _ => ValType::I64,
        }
    }

    pub(crate) fn compile_selector(
        &mut self,
        sel: &ast::Selector,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<GoType, Error> {
        // Handle package-qualified access (globals and constants)
        if let ast::Expression::Ident(pkg_ident) = sel.x.as_ref() {
            if self.compiled_packages.contains(pkg_ident.name.as_str()) {
                let qualified = format!("{}.{}", pkg_ident.name, sel.sel.name);
                if let Some(&(global_idx, _vt)) = self.global_vars.get(&qualified) {
                    let len_key = format!("{}_1", qualified);
                    if let Some(&(len_global_idx, _)) = self.global_vars.get(&len_key) {
                        out.push(Instruction::GlobalGet(global_idx));
                        out.push(Instruction::GlobalGet(len_global_idx));
                    } else {
                        out.push(Instruction::GlobalGet(global_idx));
                    }
                    return Ok(GoType::Int32);
                }
                if let Some(cv) = self.constants.get(&sel.sel.name).or_else(|| self.constants.get(&qualified)).cloned() {
                    match &cv {
                        ConstValue::I64(v) => {
                            let v = *v;
                            if v > u64::MAX as i128 || v < i64::MIN as i128 {
                                return Err(Error::SyntaxError(format!(
                                    "constant {} overflows integer", sel.sel.name
                                )));
                            }
                            out.push(Instruction::I64Const(v as i64));
                        }
                        ConstValue::F64(v) => out.push(Instruction::F64Const((*v).into())),
                        ConstValue::Bool(v) => out.push(Instruction::I32Const(*v as i32)),
                        ConstValue::Str(s) => {
                            let bytes = s.as_bytes();
                            let slen = bytes.len() as i32;

                            if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
                                for &byte in bytes.iter() {
                                    out.push(Instruction::I32Const(byte as i32));
                                }
                                out.push(Instruction::ArrayNewFixed {
                                    array_type_index: byte_array_idx,
                                    array_size: bytes.len() as u32,
                                });
                                out.push(Instruction::I32Const(slen));
                                out.push(Instruction::StructNew(go_string_idx));
                            } else {
                                let ptr_local = locals.add_local(
                                    &format!("__const_str_ptr_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::I32Const(slen));
                                out.push(Instruction::Call(self.alloc_func_idx()?));
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
                                out.push(Instruction::I32Const(slen));
                            }
                        }
                        ConstValue::Complex128(re, im) => {
                            if let Some(gc_idx) = self.gc_builtin_types.complex128 {
                                out.push(Instruction::F64Const((*re).into()));
                                out.push(Instruction::F64Const((*im).into()));
                                out.push(Instruction::StructNew(gc_idx));
                            } else {
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
                                out.push(Instruction::F64Const((*re).into()));
                                out.push(Instruction::F64Store(MemArg { offset: 0, align: float_align, memory_index: 0 }));
                                out.push(Instruction::LocalGet(ptr_local));
                                out.push(Instruction::F64Const((*im).into()));
                                out.push(Instruction::F64Store(MemArg { offset: 8, align: float_align, memory_index: 0 }));
                                out.push(Instruction::LocalGet(ptr_local));
                            }
                        }
                    }
                    let go_type = match &cv {
                        ConstValue::I64(_) => GoType::UntypedInt,
                        ConstValue::F64(_) => GoType::UntypedFloat,
                        ConstValue::Bool(_) => GoType::Bool,
                        ConstValue::Str(_) => GoType::String,
                        ConstValue::Complex128(_, _) => GoType::Complex128,
                    };
                    return Ok(go_type);
                }
            }
        }

        let struct_type_name = self.infer_struct_type_from_expr(sel.x.as_ref(), locals);

        let used_index_addr = if let ast::Expression::Index(idx) = sel.x.as_ref() {
            if let Some(ref type_name) = struct_type_name {
                if let Some(sd) = self.struct_defs.get(type_name) {
                    if sd.gc_type_idx.is_none() && sd.find_field(&sel.sel.name).is_some() {
                        let (_elem_vt, _align) = self.compile_index_store_addr(idx, out, locals)?;
                        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        true
                    } else { false }
                } else { false }
            } else { false }
        } else { false };

        if !used_index_addr {
            self.compile_expression(&sel.x, out, locals)?;
        }

        if let Some(type_name) = struct_type_name {
            let type_name = if self.struct_defs.contains_key(&type_name) {
                type_name
            } else {
                self.resolve_struct_in_pkg(&type_name)
            };
            if let Some(struct_def) = self.struct_defs.get(&type_name).cloned() {
                if let Some(gc_type_idx) = struct_def.gc_type_idx {
                    if let Some(field) = struct_def.find_field(&sel.sel.name) {
                        if field.is_string_field() {
                            if let Some(_go_string_idx) = self.gc_builtin_types.go_string {
                                out.push(Instruction::StructGet {
                                    struct_type_index: gc_type_idx,
                                    field_index: field.field_index,
                                });
                            } else {
                                let ref_local = locals.add_local(
                                    &format!("__sel_str_ref_{}", locals.locals.len()),
                                    Self::gc_ref_val_type(gc_type_idx),
                                );
                                out.push(Instruction::LocalSet(ref_local));
                                out.push(Instruction::LocalGet(ref_local));
                                out.push(Instruction::StructGet {
                                    struct_type_index: gc_type_idx,
                                    field_index: field.field_index,
                                });
                                out.push(Instruction::LocalGet(ref_local));
                                out.push(Instruction::StructGet {
                                    struct_type_index: gc_type_idx,
                                    field_index: field.field_index + 1,
                                });
                            }
                            return Ok(GoType::Int32);
                        }

                        if field.is_interface_field() {
                            let ref_local = locals.add_local(
                                &format!("__sel_iface_ref_{}", locals.locals.len()),
                                Self::gc_ref_val_type(gc_type_idx),
                            );
                            out.push(Instruction::LocalSet(ref_local));
                            out.push(Instruction::LocalGet(ref_local));
                            out.push(Instruction::StructGet {
                                struct_type_index: gc_type_idx,
                                field_index: field.field_index,
                            });
                            out.push(Instruction::LocalGet(ref_local));
                            out.push(Instruction::StructGet {
                                struct_type_index: gc_type_idx,
                                field_index: field.field_index + 1,
                            });
                            return Ok(GoType::Int32);
                        }

                        out.push(Instruction::StructGet {
                            struct_type_index: gc_type_idx,
                            field_index: field.field_index,
                        });
                        return Ok(GoType::Int32);
                    }
                } else if let Some(field) = struct_def.find_field(&sel.sel.name) {
                    if field.is_string_field() {
                        let base_offset = field.offset as u64;
                        let base_local = locals.add_local(
                            &format!("__sel_str_base_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::LocalSet(base_local));
                        out.push(Instruction::LocalGet(base_local));
                        out.push(Instruction::I32Load(MemArg {
                            offset: base_offset,
                            align: 2,
                            memory_index: 0,
                        }));
                        out.push(Instruction::LocalGet(base_local));
                        out.push(Instruction::I32Load(MemArg {
                            offset: base_offset + 4,
                            align: 2,
                            memory_index: 0,
                        }));
                        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                            self.emit_linear_to_gc_string(go_string_idx, out, locals)?;
                            return Ok(GoType::String);
                        }
                        return Ok(GoType::Int32);
                    }

                    if field.is_interface_field() {
                        let base_offset = field.offset as u64;
                        let base_local = locals.add_local(
                            &format!("__sel_iface_base_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::LocalSet(base_local));
                        out.push(Instruction::LocalGet(base_local));
                        out.push(Instruction::I32Load(MemArg {
                            offset: base_offset,
                            align: 2,
                            memory_index: 0,
                        }));
                        out.push(Instruction::LocalGet(base_local));
                        out.push(Instruction::I32Load(MemArg {
                            offset: base_offset + 4,
                            align: 2,
                            memory_index: 0,
                        }));
                        return Ok(GoType::Int32);
                    }

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
                        WasmType::I32 | WasmType::Ref(_) => out.push(Instruction::I32Load(MemArg {
                            offset,
                            align: 2,
                            memory_index: 0,
                        })),
                    }
                    return Ok(GoType::Int32);
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
                    return Ok(GoType::Func);
                }
            }
        }

        // Check if this is a method value (x.Method used as a value, not called)
        if let ast::Expression::Ident(recv_ident) = sel.x.as_ref() {
            let recv_type = locals.get_var_struct_name(&recv_ident.name)
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
                    return Ok(GoType::Func);
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

    pub(crate) fn infer_array_elem_vt(&self, elem_type: &ast::Expression) -> ValType {
        match elem_type {
            ast::Expression::Ident(id) => {
                if let Some(&idx) = match id.name.as_str() {
                    "string" => self.gc_builtin_types.go_string.as_ref(),
                    "complex64" => self.gc_builtin_types.complex64.as_ref(),
                    "complex128" => self.gc_builtin_types.complex128.as_ref(),
                    _ => None,
                } {
                    return Self::gc_ref_val_type(idx);
                }
                Self::val_type_for_type_name(&id.name)
            }
            ast::Expression::TypeSlice(_)
            | ast::Expression::TypeArray(_)
            | ast::Expression::TypeMap(_)
            | ast::Expression::TypePointer(_)
            | ast::Expression::TypeStruct(_)
            | ast::Expression::TypeInterface(_) => ValType::I32,
            _ => ValType::I64,
        }
    }

    pub(crate) fn go_type_elem_size_and_align(elem_type: &ast::Expression) -> (i32, u32) {
        match elem_type {
            ast::Expression::Ident(id) => match id.name.as_str() {
                "int" | "int64" | "uint" | "uint64" | "float64" | "string" => (8, 3),
                "int32" | "uint32" | "float32" | "rune" => (4, 2),
                "int16" | "uint16" => (2, 1),
                "int8" | "uint8" | "byte" | "bool" => (1, 0),
                _ => (4, 2),
            },
            _ => (4, 2),
        }
    }

    pub(crate) fn compile_array_literal(
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
        } else if let Some(cv) = self.try_eval_const_expr(arr_type.len.as_ref()) {
            cv.as_i64().ok_or_else(|| Error::InternalError(
                "array length constant expression is not an integer".to_string()
            ))? as u32
        } else {
            return Err(Error::InternalError(format!(
                "array length must be a constant, got {:?}", arr_type.len
            )));
        };

        let elem_vt = self.infer_array_elem_vt(&arr_type.typ);

        if matches!(elem_vt, ValType::Ref(_)) {
            let gc_array_type_idx = self.get_or_create_gc_array_type(elem_vt);
            for kv in lit_val.values.iter() {
                if let ast::Element::Expr(expr) = &kv.val {
                    self.compile_expression(expr, out, locals)?;
                }
            }
            out.push(Instruction::ArrayNewFixed {
                array_type_index: gc_array_type_idx,
                array_size: arr_len,
            });
            return Ok(());
        }

        let (elem_size, align) = Self::go_type_elem_size_and_align(&arr_type.typ);
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

        let mut current_index: u64 = 0;
        for kv in lit_val.values.iter() {
            if let Some(ref key_elem) = kv.key {
                if let ast::Element::Expr(key_expr) = key_elem {
                    if let Some(cv) = self.try_eval_const_expr(key_expr) {
                        current_index = cv.as_i64().unwrap_or(current_index as i64) as u64;
                    }
                }
            }
            if let ast::Element::Expr(expr) = &kv.val {
                out.push(Instruction::LocalGet(ptr_local));

                let is_unsigned_elem = Self::is_unsigned_array_elem(&arr_type.typ);
                if let Some(cv) = self.try_eval_const_expr(expr) {
                    if elem_vt == ValType::I64 {
                        let int_val = match cv {
                            ConstValue::I64(v) => v as i64,
                            ConstValue::F64(v) if is_unsigned_elem => v as u64 as i64,
                            ConstValue::F64(v) => v as i64,
                            _ => {
                                self.compile_expression(expr, out, locals)?;
                                let val_vt = self.infer_val_type(expr, locals);
                                Self::emit_typed_coerce(val_vt, elem_vt, out)?;
                                let offset = current_index * elem_size as u64;
                                Self::emit_go_typed_store(elem_size, align, offset, elem_vt, out);
                                current_index += 1;
                                continue;
                            }
                        };
                        out.push(Instruction::I64Const(int_val));
                    } else if elem_vt == ValType::I32 {
                        let int_val = match cv {
                            ConstValue::I64(v) => v as i32,
                            ConstValue::F64(v) if is_unsigned_elem => v as u32 as i32,
                            ConstValue::F64(v) => v as i32,
                            _ => {
                                self.compile_expression(expr, out, locals)?;
                                let val_vt = self.infer_val_type(expr, locals);
                                Self::emit_typed_coerce(val_vt, elem_vt, out)?;
                                let offset = current_index * elem_size as u64;
                                Self::emit_go_typed_store(elem_size, align, offset, elem_vt, out);
                                current_index += 1;
                                continue;
                            }
                        };
                        out.push(Instruction::I32Const(int_val));
                    } else if elem_vt == ValType::F64 {
                        let f_val = match cv {
                            ConstValue::F64(v) => v,
                            ConstValue::I64(v) => v as f64,
                            _ => {
                                self.compile_expression(expr, out, locals)?;
                                let offset = current_index * elem_size as u64;
                                Self::emit_go_typed_store(elem_size, align, offset, elem_vt, out);
                                current_index += 1;
                                continue;
                            }
                        };
                        out.push(Instruction::F64Const(f_val.into()));
                    } else if elem_vt == ValType::F32 {
                        let f_val = match cv {
                            ConstValue::F64(v) => v as f32,
                            ConstValue::I64(v) => v as f32,
                            _ => {
                                self.compile_expression(expr, out, locals)?;
                                let offset = current_index * elem_size as u64;
                                Self::emit_go_typed_store(elem_size, align, offset, elem_vt, out);
                                current_index += 1;
                                continue;
                            }
                        };
                        out.push(Instruction::F32Const(f_val.into()));
                    } else {
                        self.compile_expression(expr, out, locals)?;
                        let val_vt = self.infer_val_type(expr, locals);
                        Self::emit_typed_coerce(val_vt, elem_vt, out)?;
                    }
                } else {
                    self.compile_expression(expr, out, locals)?;
                    let val_vt = self.infer_val_type(expr, locals);
                    if val_vt != elem_vt {
                        if is_unsigned_elem && val_vt == ValType::F64 && elem_vt == ValType::I64 {
                            out.push(Instruction::I64TruncF64U);
                        } else if is_unsigned_elem && val_vt == ValType::F64 && elem_vt == ValType::I32 {
                            out.push(Instruction::I32TruncF64U);
                        } else {
                            Self::emit_typed_coerce(val_vt, elem_vt, out)?;
                        }
                    }
                }
                let offset = current_index * elem_size as u64;
                Self::emit_go_typed_store(elem_size, align, offset, elem_vt, out);
            }
            current_index += 1;
        }

        out.push(Instruction::LocalGet(ptr_local));
        Ok(())
    }

    fn is_unsigned_array_elem(elem_type: &ast::Expression) -> bool {
        matches!(elem_type,
            ast::Expression::Ident(id) if matches!(id.name.as_str(),
                "uint" | "uint8" | "uint16" | "uint32" | "uint64" | "byte"
            )
        )
    }

    pub(crate) fn compile_slice_literal(
        &mut self,
        slice_type: &ast::SliceType,
        lit_val: &ast::LiteralValue,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let elem_vt = self.infer_array_elem_vt(&slice_type.typ);
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
            Some(self.resolve_struct_in_pkg(&id.name))
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
                    out.push(Instruction::Drop);
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
                let is_unsigned_elem = Self::is_unsigned_array_elem(&slice_type.typ);
                if let Some(cv) = self.try_eval_const_expr(expr) {
                    match (elem_vt, &cv) {
                        (ValType::I64, ConstValue::F64(v)) if is_unsigned_elem => {
                            out.push(Instruction::I64Const(*v as u64 as i64));
                        }
                        (ValType::I64, ConstValue::F64(v)) => {
                            out.push(Instruction::I64Const(*v as i64));
                        }
                        (ValType::I64, ConstValue::I64(v)) => {
                            out.push(Instruction::I64Const(*v as i64));
                        }
                        (ValType::I32, ConstValue::F64(v)) if is_unsigned_elem => {
                            out.push(Instruction::I32Const(*v as u32 as i32));
                        }
                        (ValType::I32, ConstValue::F64(v)) => {
                            out.push(Instruction::I32Const(*v as i32));
                        }
                        (ValType::I32, ConstValue::I64(v)) => {
                            out.push(Instruction::I32Const(*v as i32));
                        }
                        (ValType::F64, ConstValue::F64(v)) => {
                            out.push(Instruction::F64Const((*v).into()));
                        }
                        (ValType::F64, ConstValue::I64(v)) => {
                            out.push(Instruction::F64Const((*v as f64).into()));
                        }
                        (ValType::F32, ConstValue::F64(v)) => {
                            out.push(Instruction::F32Const((*v as f32).into()));
                        }
                        (ValType::F32, ConstValue::I64(v)) => {
                            out.push(Instruction::F32Const((*v as f32).into()));
                        }
                        _ => {
                            self.compile_expression(expr, out, locals)?;
                            let val_vt = self.infer_val_type(expr, locals);
                            Self::emit_typed_coerce(val_vt, elem_vt, out)?;
                        }
                    }
                } else {
                    self.compile_expression(expr, out, locals)?;
                    let val_vt = self.infer_val_type(expr, locals);
                    if val_vt != elem_vt {
                        if is_unsigned_elem && val_vt == ValType::F64 && elem_vt == ValType::I64 {
                            out.push(Instruction::I64TruncF64U);
                        } else if is_unsigned_elem && val_vt == ValType::F64 && elem_vt == ValType::I32 {
                            out.push(Instruction::I32TruncF64U);
                        } else {
                            Self::emit_typed_coerce(val_vt, elem_vt, out)?;
                        }
                    }
                }
            } else {
                out.push(Instruction::Drop);
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

    pub(crate) fn compile_map_literal(
        &mut self,
        map_type: &ast::MapType,
        lit_val: &ast::LiteralValue,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let key_vt = self.infer_array_elem_vt(&map_type.key);
        let val_vt = self.infer_array_elem_vt(&map_type.val);
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
        locals.set_var_type(&tmp_name, DefineType::Map(Box::new(DefineType::Null), Box::new(DefineType::Null)));
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
            let mut val_vt_actual = self.infer_val_type(val_expr, locals);
            if is_string_val {
                if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                    self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                    val_vt_actual = ValType::I32;
                }
            }
            let val_tmp = locals.add_local(
                &format!("__mlit_v_{}", locals.locals.len()),
                if is_string_val { ValType::I32 } else { val_vt_actual },
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

    pub(crate) fn compile_composite_lit(
        &mut self,
        comp: &ast::CompositeLit,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<GoType, Error> {
        // Handle array literals: [N]T{v1, v2, ...}
        if let ast::Expression::TypeArray(arr_type) = comp.typ.as_ref() {
            self.compile_array_literal(arr_type, &comp.val, out, locals)?;
            return Ok(GoType::Array(Box::new(GoType::Int32), 0));
        }

        // Handle slice literals: []T{v1, v2, ...}
        if let ast::Expression::TypeSlice(slice_type) = comp.typ.as_ref() {
            self.compile_slice_literal(slice_type, &comp.val, out, locals)?;
            return Ok(GoType::Slice(Box::new(GoType::Int32)));
        }

        // Handle map literals: map[K]V{k1: v1, ...}
        if let ast::Expression::TypeMap(map_type) = comp.typ.as_ref() {
            self.compile_map_literal(map_type, &comp.val, out, locals)?;
            return Ok(GoType::Map(Box::new(GoType::Int32), Box::new(GoType::Int32)));
        }

        // Handle named composite types: MySlice{1,2,3} → resolve to underlying type
        if let ast::Expression::Ident(ident) = comp.typ.as_ref() {
            let qualified_name = self.qualify_pkg_name(&ident.name);
            if let Some(underlying) = self.named_composite_types.get(&ident.name)
                .or_else(|| self.named_composite_types.get(&qualified_name))
                .cloned()
            {
                match &underlying {
                    ast::Expression::TypeSlice(slice_type) => {
                        self.compile_slice_literal(slice_type, &comp.val, out, locals)?;
                        return Ok(GoType::Slice(Box::new(GoType::Int32)));
                    }
                    ast::Expression::TypeMap(map_type) => {
                        self.compile_map_literal(map_type, &comp.val, out, locals)?;
                        return Ok(GoType::Map(Box::new(GoType::Int32), Box::new(GoType::Int32)));
                    }
                    ast::Expression::TypeArray(arr_type) => {
                        self.compile_array_literal(arr_type, &comp.val, out, locals)?;
                        return Ok(GoType::Array(Box::new(GoType::Int32), 0));
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
            Some(self.resolve_struct_in_pkg(&ident.name))
        } else {
            None
        };

        let struct_def = type_name
            .as_ref()
            .and_then(|n| self.struct_defs.get(n))
            .cloned();

        if let Some(ref sd) = struct_def {
            if let Some(gc_type_idx) = sd.gc_type_idx {
                self.stack_alloc_target = None;
                return self.compile_gc_struct_lit(comp, gc_type_idx, sd, out, locals);
            }
        }

        let total_size = if let Some(ref sd) = struct_def {
            sd.total_size as i32
        } else {
            let field_count = comp.val.values.len();
            ((field_count * 8) as i32).max(8)
        };

        let used_stack = if let Some(ref target) = self.stack_alloc_target.take() {
            if let Some(sf) = &self.current_stack_frame {
                if let Some(sl) = sf.find(target) {
                    if let Some(fb) = sf.frame_base_local {
                        out.push(Instruction::LocalGet(fb));
                        if sl.offset > 0 {
                            out.push(Instruction::I32Const(sl.offset as i32));
                            out.push(Instruction::I32Add);
                        }
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        if !used_stack {
            out.push(Instruction::I32Const(total_size));
            out.push(Instruction::Call(self.alloc_func_idx()?));
        }

        let ptr_local = locals.add_local("__comp_ptr", ValType::I32);
        out.push(Instruction::LocalSet(ptr_local));

        let primary_indices: Vec<usize> = if let Some(ref sd) = struct_def {
            let mut indices = Vec::new();
            let mut idx = 0;
            while idx < sd.fields.len() {
                indices.push(idx);
                let has_companion = idx + 1 < sd.fields.len()
                    && sd.fields[idx + 1].name == format!("{}_{}", sd.fields[idx].name, 1)
                    && (sd.fields[idx].is_string_field() || sd.fields[idx].is_interface_field());
                if has_companion {
                    idx += 2;
                } else {
                    idx += 1;
                }
            }
            indices
        } else {
            (0..comp.val.values.len()).collect()
        };

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
                    let fi = primary_indices.get(i).copied().unwrap_or(i);
                    if fi < sd.fields.len() {
                        sd.embedded_types.iter()
                            .find(|(name, _)| name == &sd.fields[fi].name)
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
                            locals.get_var_struct_name(&key_ident.name).map(|s| s.to_string())
                        })
                    } else { None }
                } else if let Some(ref sd) = struct_def {
                    let fi = primary_indices.get(i).copied().unwrap_or(i);
                    if fi < sd.fields.len() {
                        self.struct_defs.iter().find_map(|(name, def)| {
                            if def.total_size == sd.fields[fi].wasm_type.byte_size() || sd.fields[fi].wasm_type == WasmType::I32 {
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
                    let fi = primary_indices.get(i).copied().unwrap_or(i);
                    if fi < sd.fields.len() {
                        (sd.fields[fi].offset as u64, Some(sd.fields[fi].wasm_type))
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
                    let fi = primary_indices.get(i).copied().unwrap_or(i);
                    if fi < sd.fields.len() {
                        sd.embedded_types.iter()
                            .find(|(name, _)| name == &sd.fields[fi].name)
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

            // Determine offset, type, and field_type from struct layout
            let (offset, field_wasm_type, field_type) = if let Some(ref key) = kv.key {
                if let ast::Element::Expr(ast::Expression::Ident(key_ident)) = key {
                    if let Some(ref sd) = struct_def {
                        if let Some(field) = sd.find_field(&key_ident.name) {
                            (field.offset as u64, Some(field.wasm_type), field.field_type.clone())
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
                let fi = primary_indices.get(i).copied().unwrap_or(i);
                if fi < sd.fields.len() {
                    (
                        sd.fields[fi].offset as u64,
                        Some(sd.fields[fi].wasm_type),
                        sd.fields[fi].field_type.clone(),
                    )
                } else {
                    return Err(Error::InternalError(format!(
                        "too many fields in struct literal: got {}, struct has {} (primary: {})",
                        i + 1,
                        sd.fields.len(),
                        primary_indices.len()
                    )));
                }
            } else {
                return Err(Error::InternalError(
                    "struct definition not found for composite literal".to_string(),
                ));
            };

            // String fields need special handling: store both ptr and len
            if matches!(&field_type, Some(DefineType::String)) {
                self.compile_expression(elem_expr, out, locals)?;
                if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                    self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                }
                let str_len_tmp = locals.add_local(
                    &format!("__comp_str_len_{}", locals.locals.len()),
                    ValType::I32,
                );
                let str_ptr_tmp = locals.add_local(
                    &format!("__comp_str_ptr_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalSet(str_len_tmp));
                out.push(Instruction::LocalSet(str_ptr_tmp));
                out.push(Instruction::LocalGet(ptr_local));
                out.push(Instruction::LocalGet(str_ptr_tmp));
                out.push(Instruction::I32Store(MemArg {
                    offset,
                    align: 2,
                    memory_index: 0,
                }));
                out.push(Instruction::LocalGet(ptr_local));
                out.push(Instruction::LocalGet(str_len_tmp));
                out.push(Instruction::I32Store(MemArg {
                    offset: offset + 4,
                    align: 2,
                    memory_index: 0,
                }));
                continue;
            }

            // Interface fields need special handling: store both data_ptr and type_id
            if matches!(&field_type, Some(DefineType::Interface { .. })) {
                if let ast::Expression::Ident(iface_ident) = elem_expr {
                    if iface_ident.name == "nil" {
                        out.push(Instruction::LocalGet(ptr_local));
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::I32Store(MemArg { offset, align: 2, memory_index: 0 }));
                        out.push(Instruction::LocalGet(ptr_local));
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::I32Store(MemArg { offset: offset + 4, align: 2, memory_index: 0 }));
                    } else if let Some(tid_local) = self.get_iface_type_id_local(&iface_ident.name, locals) {
                        self.compile_expression(elem_expr, out, locals)?;
                        let data_tmp = locals.add_local(
                            &format!("__comp_iface_data_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::LocalSet(data_tmp));
                        out.push(Instruction::LocalGet(ptr_local));
                        out.push(Instruction::LocalGet(data_tmp));
                        out.push(Instruction::I32Store(MemArg { offset, align: 2, memory_index: 0 }));
                        out.push(Instruction::LocalGet(ptr_local));
                        out.push(Instruction::LocalGet(tid_local));
                        out.push(Instruction::I32Store(MemArg { offset: offset + 4, align: 2, memory_index: 0 }));
                    } else {
                        // Concrete type being assigned to interface field: box it
                        self.compile_expression(elem_expr, out, locals)?;
                        let rhs_vt = self.infer_val_type(elem_expr, locals);
                        let concrete_type = locals.get_var_struct_name(&iface_ident.name)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| self.infer_concrete_type_name(elem_expr, locals));
                        let type_id = self.get_or_create_type_id(&concrete_type);
                        let (elem_size, _) = Self::elem_size_and_align(rhs_vt);
                        let val_tmp = locals.add_local(
                            &format!("__comp_ibox_v_{}", locals.locals.len()),
                            rhs_vt,
                        );
                        out.push(Instruction::LocalSet(val_tmp));
                        let alloc_size = (elem_size as i32).max(8);
                        out.push(Instruction::I32Const(alloc_size));
                        out.push(Instruction::Call(self.alloc_func_idx()?));
                        let box_ptr = locals.add_local(
                            &format!("__comp_ibox_p_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::LocalSet(box_ptr));
                        out.push(Instruction::LocalGet(box_ptr));
                        out.push(Instruction::LocalGet(val_tmp));
                        let (_, align_val) = Self::elem_size_and_align(rhs_vt);
                        Self::emit_typed_store(rhs_vt, 0, align_val, out);
                        out.push(Instruction::LocalGet(ptr_local));
                        out.push(Instruction::LocalGet(box_ptr));
                        out.push(Instruction::I32Store(MemArg { offset, align: 2, memory_index: 0 }));
                        out.push(Instruction::LocalGet(ptr_local));
                        out.push(Instruction::I32Const(type_id as i32));
                        out.push(Instruction::I32Store(MemArg { offset: offset + 4, align: 2, memory_index: 0 }));
                    }
                } else {
                    // Non-ident expression: compile and store data_ptr, type_id = 0
                    self.compile_expression(elem_expr, out, locals)?;
                    let data_tmp = locals.add_local(
                        &format!("__comp_iface_data_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(data_tmp));
                    out.push(Instruction::LocalGet(ptr_local));
                    out.push(Instruction::LocalGet(data_tmp));
                    out.push(Instruction::I32Store(MemArg { offset, align: 2, memory_index: 0 }));
                    out.push(Instruction::LocalGet(ptr_local));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::I32Store(MemArg { offset: offset + 4, align: 2, memory_index: 0 }));
                }
                continue;
            }

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

        Ok(GoType::Struct("".to_string()))
    }

    fn compile_gc_struct_lit(
        &mut self,
        comp: &ast::CompositeLit,
        gc_type_idx: u32,
        struct_def: &StructDef,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<GoType, Error> {
        let ref_vt = Self::gc_ref_val_type(gc_type_idx);
        let ref_local = locals.add_local("__gc_struct", ref_vt);
        out.push(Instruction::StructNewDefault(gc_type_idx));
        out.push(Instruction::LocalSet(ref_local));

        for (i, kv) in comp.val.values.iter().enumerate() {
            let elem_expr = match &kv.val {
                ast::Element::Expr(e) => e,
                ast::Element::LitValue(lit) => {
                    let field_info = if let Some(ref key) = kv.key {
                        if let ast::Element::Expr(ast::Expression::Ident(key_ident)) = key {
                            struct_def.find_field(&key_ident.name)
                        } else { None }
                    } else if i < struct_def.fields.len() {
                        Some(&struct_def.fields[i])
                    } else { None };

                    if let Some(field) = field_info {
                        if let Some(tag) = field.struct_type_name() {
                            if let Some(inner_sd) = self.struct_defs.get(&tag).cloned() {
                                if let Some(inner_gc_idx) = inner_sd.gc_type_idx {
                                    let inner_comp = ast::CompositeLit {
                                        typ: Box::new(ast::Expression::Ident(ast::Ident {
                                            pos: 0,
                                            name: tag.clone(),
                                        })),
                                        val: lit.clone(),
                                    };
                                    out.push(Instruction::LocalGet(ref_local));
                                    self.compile_gc_struct_lit(&inner_comp, inner_gc_idx, &inner_sd, out, locals)?;
                                    out.push(Instruction::StructSet {
                                        struct_type_index: gc_type_idx,
                                        field_index: field.field_index,
                                    });
                                    continue;
                                }
                            }
                        }
                    }
                    continue;
                }
            };

            let field = if let Some(ref key) = kv.key {
                if let ast::Element::Expr(ast::Expression::Ident(key_ident)) = key {
                    struct_def.find_field(&key_ident.name).ok_or_else(|| {
                        Error::InternalError(format!(
                            "unknown field '{}' in GC struct literal", key_ident.name
                        ))
                    })?
                } else {
                    return Err(Error::InternalError(
                        "unsupported key expression in GC struct literal".to_string(),
                    ));
                }
            } else if i < struct_def.fields.len() {
                &struct_def.fields[i]
            } else {
                return Err(Error::InternalError(format!(
                    "too many fields in GC struct literal: got {}, struct has {}",
                    i + 1, struct_def.fields.len()
                )));
            };

            let field_index = field.field_index;
            let field_wt = field.wasm_type;

            if field.is_string_field() {
                if self.gc_builtin_types.go_string.is_some() {
                    out.push(Instruction::LocalGet(ref_local));
                    self.compile_expression(elem_expr, out, locals)?;
                    out.push(Instruction::StructSet {
                        struct_type_index: gc_type_idx,
                        field_index,
                    });
                } else {
                    self.compile_expression(elem_expr, out, locals)?;
                    let str_len_tmp = locals.add_local(
                        &format!("__gc_str_len_{}", locals.locals.len()), ValType::I32,
                    );
                    let str_ptr_tmp = locals.add_local(
                        &format!("__gc_str_ptr_{}", locals.locals.len()), ValType::I32,
                    );
                    out.push(Instruction::LocalSet(str_len_tmp));
                    out.push(Instruction::LocalSet(str_ptr_tmp));
                    out.push(Instruction::LocalGet(ref_local));
                    out.push(Instruction::LocalGet(str_ptr_tmp));
                    out.push(Instruction::StructSet {
                        struct_type_index: gc_type_idx,
                        field_index,
                    });
                    if let Some(len_field) = struct_def.fields.iter().find(|f| {
                        f.name == format!("{}_1", field.name) || f.field_index == field_index + 1
                    }) {
                        out.push(Instruction::LocalGet(ref_local));
                        out.push(Instruction::LocalGet(str_len_tmp));
                        out.push(Instruction::StructSet {
                            struct_type_index: gc_type_idx,
                            field_index: len_field.field_index,
                        });
                    }
                }
                continue;
            }

            if field.is_interface_field() {
                if let ast::Expression::Ident(iface_ident) = elem_expr {
                    if iface_ident.name == "nil" {
                        out.push(Instruction::LocalGet(ref_local));
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::StructSet {
                            struct_type_index: gc_type_idx,
                            field_index,
                        });
                        if field_index + 1 < struct_def.fields.len() as u32 {
                            out.push(Instruction::LocalGet(ref_local));
                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::StructSet {
                                struct_type_index: gc_type_idx,
                                field_index: field_index + 1,
                            });
                        }
                        continue;
                    }
                }
                self.compile_expression(elem_expr, out, locals)?;
                let data_tmp = locals.add_local(
                    &format!("__gc_iface_d_{}", locals.locals.len()), ValType::I32,
                );
                out.push(Instruction::LocalSet(data_tmp));
                out.push(Instruction::LocalGet(ref_local));
                out.push(Instruction::LocalGet(data_tmp));
                out.push(Instruction::StructSet {
                    struct_type_index: gc_type_idx,
                    field_index,
                });
                if field_index + 1 < struct_def.fields.len() as u32 {
                    out.push(Instruction::LocalGet(ref_local));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::StructSet {
                        struct_type_index: gc_type_idx,
                        field_index: field_index + 1,
                    });
                }
                continue;
            }

            out.push(Instruction::LocalGet(ref_local));
            self.compile_expression(elem_expr, out, locals)?;

            let target_vt = field_wt.to_val_type();
            let expr_vt = self.infer_val_type(elem_expr, locals);

            if expr_vt != target_vt && !matches!(target_vt, ValType::Ref(_)) {
                Self::emit_typed_coerce(expr_vt, target_vt, out)?;
            }

            out.push(Instruction::StructSet {
                struct_type_index: gc_type_idx,
                field_index,
            });
        }

        out.push(Instruction::LocalGet(ref_local));
        Ok(GoType::Struct("".to_string()))
    }

    pub(crate) fn compile_slice_expr(
        &mut self,
        slice: &ast::Slice,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<GoType, Error> {
        let is_string = self.is_string_expr(&slice.left, locals);

        // Detect __slice header variables so we can load data_ptr/len/cap from header
        let is_slice_header = if let ast::Expression::Ident(ident) = &*slice.left {
            locals.is_var_type_slice(&ident.name)
                || self.global_var_struct_types.get(&self.resolve_global_var_name(&ident.name)).map_or(false, |dt| matches!(dt, DefineType::Slice(_)))
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

        if is_string && self.gc_builtin_types.go_string.is_some() {
            let go_string_idx = self.gc_builtin_types.go_string.unwrap();
            let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
            let go_str_vt = Self::gc_ref_val_type(go_string_idx);
            let arr_vt = Self::gc_ref_val_type(byte_array_idx);

            let src_ref = locals.add_local(&format!("__sslice_ref_{}", locals.locals.len()), go_str_vt);
            let src_arr = locals.add_local(&format!("__sslice_arr_{}", locals.locals.len()), arr_vt);
            out.push(Instruction::LocalSet(src_ref));
            out.push(Instruction::LocalGet(src_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
            out.push(Instruction::LocalSet(src_arr));
            out.push(Instruction::LocalGet(src_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
            out.push(Instruction::LocalSet(orig_len_local));
            out.push(Instruction::LocalGet(orig_len_local));
            out.push(Instruction::LocalSet(orig_cap_local));

            let low_local = locals.add_local(&format!("__sslice_lo_{}", locals.locals.len()), ValType::I32);
            if let Some(ref lo) = slice.index[0] {
                self.compile_expression(lo, out, locals)?;
                let vt = self.infer_val_type(lo, locals);
                if vt == ValType::I64 { out.push(Instruction::I32WrapI64); }
            } else {
                out.push(Instruction::I32Const(0));
            }
            out.push(Instruction::LocalSet(low_local));

            let high_local = locals.add_local(&format!("__sslice_hi_{}", locals.locals.len()), ValType::I32);
            if let Some(ref hi) = slice.index[1] {
                self.compile_expression(hi, out, locals)?;
                let vt = self.infer_val_type(hi, locals);
                if vt == ValType::I64 { out.push(Instruction::I32WrapI64); }
            } else {
                out.push(Instruction::LocalGet(orig_len_local));
            }
            out.push(Instruction::LocalSet(high_local));

            out.push(Instruction::LocalGet(low_local));
            out.push(Instruction::LocalGet(high_local));
            out.push(Instruction::I32GtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);

            out.push(Instruction::LocalGet(high_local));
            out.push(Instruction::LocalGet(orig_len_local));
            out.push(Instruction::I32GtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);

            let new_len = locals.add_local(&format!("__sslice_nl_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalGet(high_local));
            out.push(Instruction::LocalGet(low_local));
            out.push(Instruction::I32Sub);
            out.push(Instruction::LocalSet(new_len));

            let new_arr = locals.add_local(&format!("__sslice_na_{}", locals.locals.len()), arr_vt);
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::ArrayNew(byte_array_idx));
            out.push(Instruction::LocalSet(new_arr));

            out.push(Instruction::LocalGet(new_arr));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalGet(src_arr));
            out.push(Instruction::LocalGet(low_local));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::ArrayCopy {
                array_type_index_dst: byte_array_idx,
                array_type_index_src: byte_array_idx,
            });

            out.push(Instruction::LocalGet(new_arr));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::StructNew(go_string_idx));

            return Ok(GoType::String);
        } else if is_string {
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

        Ok(GoType::Slice(Box::new(GoType::Int32)))
    }

    pub(crate) fn compile_index(
        &mut self,
        idx: &ast::Index,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<GoType, Error> {
        let left = idx.left.as_deref().ok_or_else(|| {
            Error::InternalError("index expression missing left operand".to_string())
        })?;

        // Map indexing
        if let ast::Expression::Ident(ident) = left {
            if locals.is_var_type_map(&ident.name) {
                self.compile_map_get(&ident.name.clone(), &idx.index, out, locals)?;
                return Ok(GoType::Int32);
            }
        }

        // Chained map indexing: m[k1][k2] where m is map[K]map[K2]V2
        if let ast::Expression::Index(outer_idx) = left {
            if let Some(ast::Expression::Ident(outer_ident)) = outer_idx.left.as_deref() {
                if locals.is_var_type_map(&outer_ident.name) {
                    if let Some(inner_mti) = locals.map_types.get(&outer_ident.name)
                        .and_then(|mti| mti.nested_map_val_type.clone())
                    {
                        self.compile_map_get(&outer_ident.name.clone(), &outer_idx.index, out, locals)?;
                        let tmp_name = format!("__chained_map_{}", locals.locals.len());
                        let tmp_local = locals.add_local(&tmp_name, ValType::I32);
                        out.push(Instruction::LocalSet(tmp_local));
                        locals.set_var_type(&tmp_name, DefineType::Map(Box::new(DefineType::Null), Box::new(DefineType::Null)));
                        locals.map_types.insert(tmp_name.clone(), *inner_mti);
                        self.compile_map_get(&tmp_name, &idx.index, out, locals)?;
                        return Ok(GoType::Int32);
                    }
                }
            }
        }

        // String indexing: s[i] returns a byte (i32)
        if self.is_string_expr(left, locals) {
            self.compile_expression(left, out, locals)?;

            if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
                let go_str_vt = Self::gc_ref_val_type(go_string_idx);
                let arr_vt = Self::gc_ref_val_type(byte_array_idx);

                let str_ref = locals.add_local(&format!("__sidx_ref_{}", locals.locals.len()), go_str_vt);
                out.push(Instruction::LocalSet(str_ref));

                let str_arr = locals.add_local(&format!("__sidx_arr_{}", locals.locals.len()), arr_vt);
                let str_len = locals.add_local(&format!("__sidx_len_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalGet(str_ref));
                out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
                out.push(Instruction::LocalSet(str_arr));
                out.push(Instruction::LocalGet(str_ref));
                out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
                out.push(Instruction::LocalSet(str_len));

                self.compile_expression(&idx.index, out, locals)?;
                let idx_vt = self.infer_val_type(&idx.index, locals);
                if idx_vt == ValType::I64 {
                    out.push(Instruction::I32WrapI64);
                }
                let idx_local = locals.add_local(&format!("__sidx_i_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalTee(idx_local));

                out.push(Instruction::LocalGet(str_len));
                out.push(Instruction::I32GeU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::Unreachable);
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(str_arr));
                out.push(Instruction::LocalGet(idx_local));
                out.push(Instruction::ArrayGetU(byte_array_idx));
            } else {
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
            }
            return Ok(GoType::Uint8);
        }

        // Multi-dimensional array indexing: a[i][j] where a is [M][N]T
        if let ast::Expression::Index(outer_idx) = left {
            if let Some(ast::Expression::Ident(outer_ident)) = outer_idx.left.as_deref() {
                if let Some(&(inner_elem_vt, inner_len)) = locals.nested_array_inner_info.get(&outer_ident.name) {
                    if let Some(&(_, outer_len, ..)) = locals.array_info.get(&outer_ident.name) {
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
                        return Ok(GoType::Int32);
                    }
                }
            }
        }

        // Array indexing: a[i] with bounds check
        if let ast::Expression::Ident(ident) = left {
            if let Some(&(arr_elem_vt, arr_len, go_es, go_ea)) = locals.array_info.get(&ident.name) {
                if matches!(arr_elem_vt, ValType::Ref(_)) {
                    let gc_array_type_idx = self.get_or_create_gc_array_type(arr_elem_vt);
                    self.compile_expression(left, out, locals)?;

                    self.compile_expression(&idx.index, out, locals)?;
                    let idx_vt = self.infer_val_type(&idx.index, locals);
                    if idx_vt == ValType::I64 {
                        out.push(Instruction::I32WrapI64);
                    }

                    out.push(Instruction::ArrayGet(gc_array_type_idx));
                    return Ok(GoType::String);
                }

                let (arr_elem_size, arr_align) = (go_es, go_ea);
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
                return Ok(GoType::Int32);
            }
        }

        // Handle nested slice indexing: matrix[i][j] where matrix is [][]T
        if let ast::Expression::Index(outer_idx) = left {
            if let Some(outer_left) = outer_idx.left.as_deref() {
                if let ast::Expression::Ident(outer_ident) = outer_left {
                    if locals.is_var_type_slice(&outer_ident.name) {
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
                            return Ok(GoType::Int32);
                        }
                    }
                }
            }
        }

        let is_slice_header = if let ast::Expression::Ident(ident) = left {
            locals.is_var_type_slice(&ident.name)
                || self.global_var_struct_types.get(&self.resolve_global_var_name(&ident.name)).map_or(false, |dt| matches!(dt, DefineType::Slice(_)))
        } else {
            false
        };

        let (elem_vt, elem_size, align) = if let ast::Expression::Ident(ident) = left {
            if let Some(&vt) = locals.slice_elem_types.get(&ident.name) {
                let (es, al) = Self::elem_size_and_align(vt);
                (vt, es, al)
            } else {
                let resolved = self.resolve_global_var_name(&ident.name);
                if let Some(&(vt, es, al)) = self.global_array_elem_types.get(&resolved) {
                    (vt, es, al)
                } else {
                    (ValType::I64, 8, 3)
                }
            }
        } else {
            (ValType::I64, 8, 3)
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

        match (elem_vt, elem_size) {
            (ValType::I32, 1) => out.push(Instruction::I32Load8U(MemArg {
                offset: 0,
                align: 0,
                memory_index: 0,
            })),
            (ValType::I32, 2) => out.push(Instruction::I32Load16U(MemArg {
                offset: 0,
                align: 1,
                memory_index: 0,
            })),
            (ValType::I32, _) => out.push(Instruction::I32Load(MemArg {
                offset: 0,
                align,
                memory_index: 0,
            })),
            (ValType::F32, _) => out.push(Instruction::F32Load(MemArg {
                offset: 0,
                align,
                memory_index: 0,
            })),
            (ValType::F64, _) => out.push(Instruction::F64Load(MemArg {
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
        Ok(GoType::Int32)
    }

    pub(crate) fn compile_index_store_addr(
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
                    if let Some(&(_, outer_len, ..)) = locals.array_info.get(&outer_ident.name) {
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
            if let Some(&(arr_elem_vt, arr_len, go_es, go_ea)) = locals.array_info.get(&ident.name) {
                let (arr_elem_size, arr_align) = (go_es, go_ea);
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
            locals.is_var_type_slice(&ident.name)
                || self.global_var_struct_types.get(&self.resolve_global_var_name(&ident.name)).map_or(false, |dt| matches!(dt, DefineType::Slice(_)))
        } else {
            false
        };

        let (elem_vt, elem_size, align) = if let ast::Expression::Ident(ident) = left {
            if let Some(&vt) = locals.slice_elem_types.get(&ident.name) {
                let (es, al) = Self::elem_size_and_align(vt);
                (vt, es, al)
            } else {
                let resolved = self.resolve_global_var_name(&ident.name);
                if let Some(&(vt, es, al)) = self.global_array_elem_types.get(&resolved) {
                    (vt, es, al)
                } else {
                    (ValType::I64, 8, 3)
                }
            }
        } else {
            (ValType::I64, 8, 3)
        };

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

    pub(crate) fn compile_selector_store_addr(
        &mut self,
        sel: &ast::Selector,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(u64, ValType), Error> {
        self.compile_expression(&sel.x, out, locals)?;

        let struct_type_name = self.infer_struct_type_from_expr(sel.x.as_ref(), locals);

        if let Some(type_name) = struct_type_name {
            let type_name = if self.struct_defs.contains_key(&type_name) {
                type_name
            } else {
                self.resolve_struct_in_pkg(&type_name)
            };
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

    pub(crate) fn compile_embedded_lit_inline(
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
                                .and_then(|f| f.struct_type_name())
                                .and_then(|tag| self.struct_defs.get(&tag).cloned())
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

    pub(crate) fn infer_selector_struct_type(&self, expr: &ast::Expression, locals: &LocalAlloc) -> Option<String> {
        match expr {
            ast::Expression::Ident(ident) => {
                locals.get_var_struct_name(&ident.name).map(|s| s.to_string())
            }
            ast::Expression::Selector(sel) => {
                let parent_type = self.infer_selector_struct_type(sel.x.as_ref(), locals)?;
                let sdef = self.struct_defs.get(&parent_type)?;
                let field = sdef.fields.iter().find(|f| f.name == sel.sel.name)?;
                field.struct_type_name()
            }
            _ => None,
        }
    }

    pub(crate) fn infer_deref_type(&self, expr: &ast::Expression, locals: &LocalAlloc) -> ValType {
        if let ast::Expression::Ident(ident) = expr {
            if let Some(dt) = locals.get_var_type(&ident.name) {
                if matches!(dt, DefineType::Slice(_) | DefineType::String)
                    || dt.resolved_name().map_or(false, |n| self.struct_defs.contains_key(n) || n == "Context")
                {
                    return ValType::I32;
                }
                if let DefineType::Ref(inner) = dt {
                    return match inner.as_ref() {
                        DefineType::Int64 | DefineType::Uint64 | DefineType::Int | DefineType::Uint => ValType::I64,
                        DefineType::Float32 => ValType::F32,
                        DefineType::Float64 => ValType::F64,
                        _ => ValType::I32,
                    };
                }
                return ValType::I32;
            }
        }
        ValType::I32
    }

    pub(crate) fn elem_size_and_align(vt: ValType) -> (i32, u32) {
        match vt {
            ValType::I32 | ValType::F32 => (4, 2),
            _ => (8, 3),
        }
    }

    pub(crate) fn expression_result_count(&self, expr: &ast::Expression, locals: Option<&LocalAlloc>) -> usize {
        match expr {
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    match ident.name.as_str() {
                        "panic" | "println" | "print" | "delete" | "clear" => 0,
                        "recover" => {
                            if self.gc_builtin_types.go_string.is_some() { 1 } else { 2 }
                        }
                        "len" | "cap" | "copy" | "make" | "append"
                        | "int" | "int64" | "uint" | "uint64"
                        | "float64" | "float32"
                        | "int8" | "int16" | "int32" | "rune"
                        | "byte" | "uint8" | "uint16" | "uint32"
                        | "bool"
                        | "new" | "min" | "max"
                        | "complex" | "real" | "imag" => 1,
                        "string" => {
                            if self.gc_builtin_types.go_string.is_some() { 1 } else { 2 }
                        }
                        _ => {
                            if self.iface_defs.contains_key(&ident.name)
                                || ident.name == "error"
                                || ident.name == "any"
                            {
                                return 2;
                            }
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
                            } else if let Some(fi) = self.find_func_in_pkg(&ident.name) {
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
                LitKind::String => {
                    if self.gc_builtin_types.go_string.is_some() { 1 } else { 2 }
                }
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

    pub(crate) fn call_return_val_types(&self, expr: &ast::Expression, locals: &LocalAlloc) -> Vec<ValType> {
        if let ast::Expression::Call(call) = expr {
            if let ast::Expression::Ident(ident) = call.func.as_ref() {
                match ident.name.as_str() {
                    "Float64frombits" => return vec![ValType::F64],
                    "Float64bits" => return vec![ValType::I64],
                    "Float32frombits" => return vec![ValType::F32],
                    "Float32bits" => return vec![ValType::I32],
                    _ => {}
                }
                if let Some(fi) = self.find_func_in_pkg(&ident.name) {
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

    pub(crate) fn infer_type_alias_from_expr(&self, expr: &ast::Expression) -> Option<String> {
        match expr {
            ast::Expression::Selector(sel) => {
                if let ast::Expression::Ident(pkg) = sel.x.as_ref() {
                    let qualified = format!("{}.{}", pkg.name, sel.sel.name);
                    if let Some(t) = self.constant_types.get(&sel.sel.name) {
                        return Some(t.clone());
                    }
                    if let Some(t) = self.constant_types.get(&qualified) {
                        return Some(t.clone());
                    }
                }
                None
            }
            ast::Expression::Operation(op) => {
                if let Some(t) = self.infer_type_alias_from_expr(&op.x) {
                    return Some(t);
                }
                if let Some(ref y) = op.y {
                    return self.infer_type_alias_from_expr(y);
                }
                None
            }
            ast::Expression::Paren(p) => self.infer_type_alias_from_expr(&p.expr),
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(fn_ident) = call.func.as_ref() {
                    let qualified = if let Some(ref pkg) = self.current_package {
                        format!("{}.{}", pkg, fn_ident.name)
                    } else {
                        fn_ident.name.clone()
                    };
                    if self.type_aliases.contains_key(&fn_ident.name)
                        || self.type_aliases.contains_key(&qualified)
                    {
                        return Some(qualified);
                    }
                } else if let ast::Expression::Selector(sel) = call.func.as_ref() {
                    if let ast::Expression::Ident(pkg) = sel.x.as_ref() {
                        let qualified = format!("{}.{}", pkg.name, sel.sel.name);
                        if self.type_aliases.contains_key(&qualified) {
                            return Some(qualified);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub(crate) fn count_go_level_returns(go_types: &[String], gc_strings: bool) -> usize {
        let mut count = 0;
        let mut i = 0;
        while i < go_types.len() {
            count += 1;
            if !gc_strings && go_types[i] == "string" {
                i += 2;
            } else {
                i += 1;
            }
        }
        count
    }

    pub(crate) fn call_return_go_types(&self, expr: &ast::Expression, locals: &LocalAlloc) -> Vec<String> {
        if let ast::Expression::Call(call) = expr {
            if let ast::Expression::Ident(ident) = call.func.as_ref() {
                if let Some(fi) = self.find_func_in_pkg(&ident.name) {
                    return fi.result_define_types.iter().map(|dt| dt.go_type_string()).collect();
                }
            }
            if let ast::Expression::Selector(sel) = call.func.as_ref() {
                if let Some(qualified) = self.resolve_selector_method_name(sel, locals) {
                    if let Some(fi) = self.functions.iter().find(|f| f.name == qualified) {
                        return fi.result_define_types.iter().map(|dt| dt.go_type_string()).collect();
                    }
                }
            }
        }
        vec![]
    }

    pub(crate) fn resolve_selector_method_name(&self, sel: &ast::Selector, locals: &LocalAlloc) -> Option<String> {
        if let ast::Expression::Ident(recv_ident) = sel.x.as_ref() {
            if let Some(type_name) = locals.get_var_struct_name(&recv_ident.name) {
                let unresolved = format!("{}.{}", type_name, sel.sel.name);
                if self.functions.iter().any(|f| f.name == unresolved) {
                    return Some(unresolved);
                }
                if let Some(ref pkg) = self.current_package {
                    let pkg_qualified = format!("{}.{}.{}", pkg, type_name, sel.sel.name);
                    if self.functions.iter().any(|f| f.name == pkg_qualified) {
                        return Some(pkg_qualified);
                    }
                }
                for cpkg in &self.compiled_packages {
                    let pkg_qualified = format!("{}.{}.{}", cpkg, type_name, sel.sel.name);
                    if self.functions.iter().any(|f| f.name == pkg_qualified) {
                        return Some(pkg_qualified);
                    }
                }
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
        if let Some(type_name) = self.infer_struct_type_from_expr(sel.x.as_ref(), locals) {
            let qualified = format!("{}.{}", type_name, sel.sel.name);
            if self.functions.iter().any(|f| f.name == qualified) {
                return Some(qualified);
            }
        }
        let method_suffix = format!(".{}", sel.sel.name);
        let matches: Vec<_> = self.functions.iter()
            .filter(|f| f.recv_type.is_some() && f.name.ends_with(&method_suffix))
            .collect();
        if matches.len() == 1 {
            return Some(matches[0].name.clone());
        }
        None
    }
}
