pub mod builtin;
pub mod compiler;
pub mod module;
pub mod object;
pub mod symbols;

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::io::{BufWriter, Write};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;
use tokio::sync::{Notify, RwLock};

use crate::vm::compiler::{bytecode_to_human, Bytecode, OpCode, SourceMap};
use crate::vm::object::channel::Channel;
use crate::vm::object::collections::{Array, Map, ObjIter, Slice, Variadic};
use crate::vm::object::float::{Float32, Float64};
use crate::vm::object::structure::{Alias, Interface, Struct, TypeValue};
use crate::vm::object::{FromString, FromVec, Object, Type};

#[derive(Copy, Clone, Debug)]
struct Frame {
    ip: usize,
    base_pointer: u16,
}

impl Frame {
    #[inline(always)]
    fn new(ip: usize, base_pointer: u16) -> Self {
        Frame { ip, base_pointer }
    }
}

struct DeferredCall {
    func: Object,
    args: Vec<Object>,
}

struct GoroutineTracker {
    count: AtomicUsize,
    done: Notify,
}

impl GoroutineTracker {
    fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            done: Notify::new(),
        }
    }

    fn spawn(&self) {
        self.count.fetch_add(1, AtomicOrdering::SeqCst);
    }

    fn finish(&self) {
        if self.count.fetch_sub(1, AtomicOrdering::SeqCst) == 1 {
            self.done.notify_waiters();
        }
    }

    async fn wait_all(&self) {
        loop {
            let notified = self.done.notified();
            if self.count.load(AtomicOrdering::SeqCst) == 0 {
                break;
            }
            notified.await;
        }
    }
}

struct SharedState {
    instructions: Vec<u8>,
    constants: Vec<Object>,
    globals: RwLock<Vec<Object>>,
    source_map: SourceMap,
    tracker: GoroutineTracker,
}

struct Goroutine {
    shared: Arc<SharedState>,
    stack: Vec<Object>,
    frames: Vec<Frame>,
    ip: usize,
    bp: u16,
    closure_ctx: Vec<Object>,
    deferred: Vec<Vec<DeferredCall>>,
    assert_stdout: Option<(String, BufWriter<Vec<u8>>)>,
    panic_value: Option<Object>,
}

impl Goroutine {
    fn new(shared: Arc<SharedState>) -> Self {
        let mut frames = Vec::with_capacity(128);
        frames.push(Frame::new(0, 0));

        Self {
            shared,
            stack: Vec::with_capacity(128),
            frames,
            ip: 0,
            bp: 0,
            closure_ctx: Vec::with_capacity(10),
            deferred: vec![Vec::new()],
            assert_stdout: None,
            panic_value: None,
        }
    }

    fn for_spawn(shared: Arc<SharedState>, ip: u32, args: Vec<Object>, num_locals: u32) -> Self {
        let mut frames = Vec::with_capacity(32);
        frames.push(Frame::new(0, 0));
        frames.push(Frame::new(ip as usize, 0));

        let mut stack = Vec::with_capacity(64);
        for arg in &args {
            stack.push(*arg);
        }
        for _ in 0..(num_locals as usize).saturating_sub(args.len()) {
            stack.push(Object::null());
        }

        Self {
            shared,
            stack,
            frames,
            ip: ip as usize,
            bp: 0,
            closure_ctx: Vec::with_capacity(4),
            deferred: vec![Vec::new()],
            assert_stdout: None,
            panic_value: None,
        }
    }

    #[inline(always)]
    fn get_local(&self, rel_idx: u16) -> Object {
        self.stack[self.bp as usize + rel_idx as usize]
    }

    #[inline(always)]
    fn set_local(&mut self, rel_idx: u16, value: Object) {
        self.stack[self.bp as usize + rel_idx as usize] = value;
    }

    #[inline(always)]
    fn copy_ll(&mut self, src_idx: u16, dst_idx: u16) {
        self.stack[self.bp as usize + dst_idx as usize] =
            self.stack[self.bp as usize + src_idx as usize];
    }

    #[inline(always)]
    fn read_u8(&mut self) -> u8 {
        let v = unsafe { *self.shared.instructions.get_unchecked(self.ip) };
        self.ip += 1;
        v
    }

    #[inline(always)]
    fn read_u16(&mut self) -> u16 {
        let start = self.ip;
        self.ip += 2;
        let bytes = unsafe { self.shared.instructions.get_unchecked(start..self.ip) };
        bytes[0] as u16 | (bytes[1] as u16) << 8
    }

    #[inline(always)]
    fn jump(&mut self, ip: u16) {
        self.ip = ip as usize;
    }

    #[inline(always)]
    fn next(&mut self) -> OpCode {
        let byte = unsafe { *self.shared.instructions.get_unchecked(self.ip) };
        self.ip += 1;
        OpCode::from(byte)
    }

    #[inline(always)]
    fn pop(&mut self) -> Object {
        debug_assert!(!self.stack.is_empty());
        unsafe {
            let new_len = self.stack.len() - 1;
            self.stack.set_len(new_len);
            ptr::read(self.stack.as_ptr().add(new_len))
        }
    }

    fn pop_ref_mut(&mut self) -> &mut Object {
        debug_assert!(!self.stack.is_empty());
        let i = self.stack.len() - 1;
        &mut self.stack[i]
    }

    #[inline(always)]
    fn push(&mut self, obj: Object) {
        self.stack.push(obj)
    }

    #[inline(always)]
    fn popframe(&mut self) {
        let frame = self.frames.pop().unwrap();
        self.stack.truncate(frame.base_pointer as usize);
        let frame = self.frames.last().unwrap();
        self.ip = frame.ip;
        self.bp = frame.base_pointer;
    }

    #[inline(always)]
    fn pushframe(&mut self, ip: u32, base_pointer: u16) {
        let frame = self.frames.last_mut().unwrap();
        frame.ip = self.ip;
        self.frames.push(Frame::new(ip as usize, base_pointer));
        self.deferred.push(Vec::new());
        self.ip = ip as usize;
        self.bp = base_pointer;
    }

    #[inline(always)]
    fn enclosed_ptr_write(&mut self, rel_idx: u16, value: Object) {
        let ctx = self.closure_ctx[rel_idx as usize];
        let mut ptr = match ctx.tag() {
            Type::Ref => ctx.as_ref_mut().value,
            Type::Closure => ctx,
            _ => panic!("not supported ptr write"),
        };
        assert_eq!(ptr.tag(), value.tag());
        match ptr.tag() {
            Type::Int => {
                ptr.as_int_mut().value = value.as_isize();
            }
            Type::Closure => {
                let c = ptr.as_closure_mut();
                let v = value.as_closure();
                c.is_null = v.is_null;
                c.ip = v.ip;
                c.captured = v.captured.clone();
                c.num_locals = v.num_locals;
            }
            _ => panic!("not supported ptr write"),
        }
    }

    #[inline(always)]
    fn local_ptr_write(&mut self, rel_idx: u16, value: Object) {
        let ctx = self.stack[self.bp as usize + rel_idx as usize];
        let mut ptr = match ctx.tag() {
            Type::Ref => ctx.as_ref_mut().value,
            Type::Closure => ctx,
            _ => panic!("not supported ptr write"),
        };
        assert_eq!(ptr.tag(), value.tag());
        match ptr.tag() {
            Type::Int => {
                ptr.as_int_mut().value = value.as_isize();
            }
            Type::Closure => {
                let c = ptr.as_closure_mut();
                let v = value.as_closure();
                c.is_null = v.is_null;
                c.ip = v.ip;
                c.captured = v.captured.clone();
                c.num_locals = v.num_locals;
            }
            _ => panic!("not supported ptr write"),
        }
    }

    fn execute_deferred<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            let calls = self.deferred.pop().unwrap_or_default();
            for call in calls.into_iter().rev() {
                let num_args = call.args.len();
                for arg in &call.args {
                    self.push(arg.clone());
                }
                let base_pointer = self.stack.len() as u16 - num_args as u16;

                let (ip, num_locals) = match call.func.tag() {
                    Type::Function => {
                        let [ip, num_locals] = call.func.as_function();
                        (ip, num_locals)
                    }
                    Type::Closure => {
                        self.closure_ctx.push(call.func);
                        let closure = call.func.as_closure();
                        (closure.ip, closure.num_locals as u32)
                    }
                    _ => {
                        return Err(Error::TypeError(format!(
                            "deferred call is not a function: {:?}",
                            call.func.tag()
                        )));
                    }
                };

                for _ in 0..num_locals - num_args as u32 {
                    self.push(Object::null());
                }

                self.pushframe(ip, base_pointer);
                match self.execute_loop_inner(false).await {
                    Ok(_) => {}
                    Err(Error::GoPanic(v)) => {
                        self.panic_value = Some(v);
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        })
    }

    fn execute_loop<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Object, Error>> + Send + 'a>>
    {
        self.execute_loop_inner(true)
    }

    fn execute_loop_inner<'a>(
        &'a mut self,
        check_panic: bool,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Object, Error>> + Send + 'a>>
    {
        Box::pin(async move {
        let initial_depth = self.frames.len();
        let mut final_result = Object::null();

        macro_rules! impl_binary_op_method {
            ($op:tt) => {{
                let right = self.pop();
                let left = self.pop();
                let result = left.$op(right)?;
                self.push(result);
            }};
        }

        macro_rules! impl_binary_const_local_op_method {
            ($op:tt) => {{
                let local_idx = self.read_u16();
                let left = self.get_local(local_idx);
                let constant_idx = self.read_u16();
                let right = self.shared.constants[constant_idx as usize];
                let result = left.$op(right)?;
                self.push(result);
            }};
        }

        loop {
            if check_panic && self.panic_value.is_some() {
                self.execute_deferred().await?;
                if self.panic_value.is_none() {
                    self.popframe();
                    self.push(Object::null());
                    self.closure_ctx.pop();
                    if self.frames.len() < initial_depth {
                        return Ok(Object::null());
                    }
                    continue;
                }
                self.deferred.pop();
                self.popframe();
                self.closure_ctx.pop();
                if self.frames.len() < initial_depth {
                    return Err(Error::GoPanic(self.panic_value.take().unwrap()));
                }
                continue;
            }
            match self.next() {
                OpCode::CastToAlias => {
                    let alias_id = self.read_u16();
                    let value = self.pop();
                    let c = self.shared.globals.read().await[alias_id as usize];
                    if c.tag() != Type::Alias {
                        panic!(
                            "expected alias: got {:#?} id: {:#?}",
                            c, alias_id
                        );
                    }
                    let alias = unsafe { Alias::read(&c) };
                    self.push(Alias::object(
                        alias.name.clone(),
                        value,
                        alias.method_dispatch.clone(),
                        alias.is_transparent,
                    ));
                }
                OpCode::CastToFloat64 => {
                    let i = self.read_u8();
                    let len = self.stack.len();
                    let n = &mut self.stack[len - 1 - i as usize];
                    let f: f64 = match n.tag() {
                        Type::Int => n.as_int().value as f64,
                        Type::I8 => n.as_int8().value as f64,
                        Type::I16 => n.as_int16().value as f64,
                        Type::I32 => n.as_int32().value as f64,
                        Type::I64 => n.as_int64().value as f64,
                        Type::UI => n.as_uint().value as f64,
                        Type::UI8 => n.as_uint8().value as f64,
                        Type::UI16 => n.as_uint16().value as f64,
                        Type::UI32 => n.as_uint32().value as f64,
                        Type::UI64 => n.as_uint64().value as f64,
                        Type::Float32 => n.as_float32() as f64,
                        Type::Float64 => continue,
                        _ => {
                            return Err(Error::TypeError(format!(
                                "cannot cast {:?} to float64",
                                n.tag()
                            )))
                        }
                    };
                    *n = Float64::from_f64(f);
                }
                OpCode::CastToFloat32 => {
                    let i = self.read_u8();
                    let len = self.stack.len();
                    let n = &mut self.stack[len - 1 - i as usize];
                    let f: f32 = match n.tag() {
                        Type::Int => n.as_int().value as f32,
                        Type::I8 => n.as_int8().value as f32,
                        Type::I16 => n.as_int16().value as f32,
                        Type::I32 => n.as_int32().value as f32,
                        Type::I64 => n.as_int64().value as f32,
                        Type::UI => n.as_uint().value as f32,
                        Type::UI8 => n.as_uint8().value as f32,
                        Type::UI16 => n.as_uint16().value as f32,
                        Type::UI32 => n.as_uint32().value as f32,
                        Type::UI64 => n.as_uint64().value as f32,
                        Type::Float64 => n.as_float64() as f32,
                        Type::Float32 => continue,
                        _ => {
                            return Err(Error::TypeError(format!(
                                "cannot cast {:?} to float32",
                                n.tag()
                            )))
                        }
                    };
                    *n = Float32::from_f32(f);
                }
                OpCode::TypedNull => {
                    let num = self.read_u16() as usize;
                    let _p = self.pop();
                    let c = self.shared.constants[num];
                    self.push(c.typed_null());
                }
                OpCode::Variadic => {
                    let num = self.read_u16() as usize;
                    let mut args = VecDeque::with_capacity(num);
                    for _ in 0..num {
                        args.push_front(self.pop());
                    }
                    if num == 1 {
                        match args[0].tag() {
                            Type::Slice => {
                                let args = args[0].as_slice();
                                self.push(Variadic::from_vec(args.clone()));
                            }
                            Type::Array => {
                                let arr = unsafe { Array::read(&args[0]) };
                                self.push(Variadic::from_vec(arr.clone()));
                            }
                            Type::Variadic => {
                                self.push(args[0]);
                            }
                            _ => {
                                let vec: Vec<Object> = Vec::from(args);
                                self.push(Variadic::from_vec(vec));
                            }
                        }
                    } else {
                        let vec: Vec<Object> = Vec::from(args);
                        self.push(Variadic::from_vec(vec));
                    }
                }
                OpCode::Slice => {
                    let num = self.read_u16();
                    let ctv_id = self.read_u16();
                    let ctv = self.shared.constants[ctv_id as usize].as_type_value().clone();
                    match num {
                        0 => {
                            let slice = self.pop();
                            self.push(Slice::from_slice(&slice.as_slice()[..], ctv));
                        }
                        1 => {
                            let start = self.pop();
                            let slice = self.pop();
                            self.push(Slice::from_slice(
                                &slice.as_slice()[start.as_isize() as usize..],
                                ctv,
                            ));
                        }
                        2 => {
                            let end = self.pop();
                            let slice = self.pop();
                            self.push(Slice::from_slice(
                                &slice.as_slice()[..end.as_isize() as usize],
                                ctv,
                            ));
                        }
                        3 => {
                            let end = self.pop();
                            let start = self.pop();
                            let slice = self.pop();
                            self.push(Slice::from_slice(
                                &slice.as_slice()
                                    [start.as_isize() as usize..end.as_isize() as usize],
                                ctv,
                            ));
                        }
                        i => unreachable!("{:#?}", i),
                    }
                }
                OpCode::IncCaptured => {
                    let id = self.read_u16();
                    let closure = self.closure_ctx.last_mut().unwrap().as_closure_mut();
                    let val = unsafe { closure.captured.get_unchecked_mut(id as usize) };
                    let new_val = val.as_int_mut();
                    new_val.value += 1;
                }
                OpCode::IncLocal => {
                    let id = self.read_u16();
                    let val = &mut self.stack[self.bp as usize + id as usize];
                    let new_val = val.as_int_mut();
                    new_val.value += 1;
                }
                OpCode::IncGlobal => {
                    let id = self.read_u16();
                    let mut globals = self.shared.globals.write().await;
                    let val = &mut globals[id as usize];
                    let new_val = val.as_int_mut();
                    new_val.value += 1;
                }
                OpCode::SetDefault => {
                    let def = self.pop();
                    let value = self.pop();
                    if def.tag() == value.tag() {
                        self.push(value);
                    } else {
                        self.push(def)
                    }
                }
                OpCode::PanicIfFalse => {
                    let val = self.pop();
                    let b = val.as_bool();
                    if !b {
                        self.panic_value = Some(Object::string("assertion failed"));
                    }
                }
                OpCode::TypeCmp => {
                    let left = self.pop();
                    let right = self.pop();
                    match (left.tag(), right.tag()) {
                        (Type::Interface, Type::Interface) => {
                            let left_i = unsafe { Interface::read(&left) };
                            let right_i = unsafe { Interface::read(&right) };
                            self.push(Object::bool(
                                left.tag() == right.tag() && left_i.name == right_i.name,
                            ));
                        }
                        (Type::Struct, Type::Struct) => {
                            let left_i = unsafe { Struct::read(&left) };
                            let right_i = unsafe { Struct::read(&right) };
                            self.push(Object::bool(
                                left.tag() == right.tag() && left_i.name == right_i.name,
                            ));
                        }
                        (Type::Type, _) => {
                            let left_i = unsafe { TypeValue::read(&left) };
                            self.push(Object::bool(left_i.value == right.tag()));
                        }
                        (_, Type::Type) => {
                            let right_i = unsafe { TypeValue::read(&right) };
                            self.push(Object::bool(right_i.value == left.tag()));
                        }
                        _ => {
                            self.push(Object::bool(false));
                        }
                    };
                }
                OpCode::TypeOf => {
                    let value = self.pop();
                    let tag = if value.tag() == Type::Ref {
                        value.as_ref().value.tag()
                    } else {
                        value.tag()
                    };
                    self.push(TypeValue::object(tag, None));
                }
                OpCode::Downcast => {
                    let value = self.pop();
                    let iface = unsafe { Interface::read(&value) };
                    self.push(iface.value);
                }
                OpCode::DynamicDispatch => {
                    let num_args = self.read_u16();
                    let method_id = self.read_u16();
                    let value = self.pop();
                    let iface = unsafe { Interface::read(&value) };
                    let method_name = &iface.methods[method_id as usize];
                    let strct = iface.value.as_struct();
                    let mut found = false;
                    for (name, ip) in &strct.method_dispatch {
                        if method_name == name {
                            let base_pointer = self.stack.len() as u16 - num_args;
                            self.pushframe(*ip as u32, base_pointer);
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        return Err(Error::TypeError(format!(
                            "method '{}' not found in dynamic dispatch",
                            method_name
                        )));
                    }
                }
                OpCode::Upcast => {
                    let iface_id = self.read_u16();
                    let value = self.pop();
                    let c = self.shared.globals.read().await[iface_id as usize];
                    if c.tag() != Type::Interface {
                        panic!(
                            "expected interface: got {:#?} id: {:#?}",
                            c, iface_id
                        );
                    }
                    let v = if value.is_ref() {
                        value.as_ref().value
                    } else {
                        value
                    };
                    let interface = unsafe { Interface::read(&c) };
                    let iface =
                        Interface::object(interface.name.clone(), interface.methods.clone(), v);
                    self.push(iface);
                }
                OpCode::Const => {
                    let idx = self.read_u16();
                    let value = self.shared.constants[idx as usize];
                    self.push(value.deep_copy());
                }
                OpCode::Deref => {
                    let val = self.pop();
                    let r = val.as_ref();
                    self.push(r.value);
                }
                OpCode::Propagate => {
                    let idx = self.read_u16();
                    let value = self.pop();
                    let closure = self.pop_ref_mut().as_closure_mut();
                    let c = unsafe { closure.captured.get_unchecked_mut(idx as usize) };
                    *c = value;
                }
                OpCode::SetGlobal => {
                    let idx = self.read_u16() as usize;
                    let value = self.pop();
                    let mut globals = self.shared.globals.write().await;
                    while globals.len() <= idx {
                        globals.push(Object::null());
                    }
                    globals[idx] = value;
                }
                OpCode::GetGlobal => {
                    let idx = self.read_u16();
                    let value = self.shared.globals.read().await[idx as usize];
                    self.push(value);
                }
                OpCode::SetLocal => {
                    let idx = self.read_u16();
                    let value = self.pop();
                    self.set_local(idx, value.clone());
                }
                OpCode::GetLocal => {
                    let idx = self.read_u16();
                    let value = self.get_local(idx);
                    self.push(value);
                }
                OpCode::SetCaptured => {
                    let idx = self.read_u16();
                    let value = self.pop();
                    let closure = self.closure_ctx.last_mut().unwrap().as_closure_mut();
                    let c = unsafe { closure.captured.get_unchecked_mut(idx as usize) };
                    *c = value;
                }
                OpCode::GetCaptured => {
                    let idx = self.read_u16();
                    let closure = self.closure_ctx.last().unwrap().as_closure();
                    let c = unsafe { closure.captured.get_unchecked(idx as usize) };
                    self.push(c.clone());
                }
                OpCode::EnclosedPtrWrite => {
                    let idx = self.read_u16();
                    let value = self.pop();
                    self.enclosed_ptr_write(idx, value);
                }
                OpCode::LocalPtrWrite => {
                    let idx = self.read_u16();
                    let value = self.pop();
                    self.local_ptr_write(idx, value);
                }
                OpCode::GlobalPtrWrite => {
                    let idx = self.read_u16();
                    let value = self.pop();
                    let mut globals = self.shared.globals.write().await;
                    while globals.len() <= idx as usize {
                        globals.push(Object::null());
                    }
                    let ctx = globals[idx as usize];
                    if ctx.is_null() {
                        panic!("global_ptr_write: nil pointer dereference");
                    }
                    let mut ptr = match ctx.tag() {
                        Type::Ref => ctx.as_ref_mut().value,
                        Type::Closure => ctx,
                        _ => panic!("global_ptr_write: not supported ptr write"),
                    };
                    assert_eq!(ptr.tag(), value.tag());
                    match ptr.tag() {
                        Type::Int => {
                            ptr.as_int_mut().value = value.as_isize();
                        }
                        Type::Closure => {
                            let c = ptr.as_closure_mut();
                            let v = value.as_closure();
                            c.is_null = v.is_null;
                            c.ip = v.ip;
                            c.captured = v.captured.clone();
                            c.num_locals = v.num_locals;
                        }
                        _ => panic!("global_ptr_write: not supported ptr write"),
                    }
                }
                OpCode::CopyGG => {
                    let src = self.read_u16();
                    let dst = self.read_u16();
                    let mut globals = self.shared.globals.write().await;
                    globals[src as usize] = globals[dst as usize];
                }
                OpCode::CopyLG => {
                    let src = self.read_u16();
                    let dst = self.read_u16();
                    let mut globals = self.shared.globals.write().await;
                    globals[src as usize] = self.stack[self.bp as usize + dst as usize];
                }
                OpCode::CopyGL => {
                    let src = self.read_u16();
                    let dst = self.read_u16();
                    let globals = self.shared.globals.read().await;
                    self.stack[self.bp as usize + dst as usize] = globals[src as usize];
                }
                OpCode::CopyLL => {
                    let src = self.read_u16();
                    let dst = self.read_u16();
                    self.copy_ll(src, dst);
                }
                OpCode::SwapGG => {
                    let src = self.read_u16();
                    let dst = self.read_u16();
                    let mut globals = self.shared.globals.write().await;
                    globals.swap(src as usize, dst as usize);
                }
                OpCode::SwapLG => {
                    let local_idx = self.read_u16();
                    let global_idx = self.read_u16();
                    let mut globals = self.shared.globals.write().await;
                    std::mem::swap(
                        &mut self.stack[self.bp as usize + local_idx as usize],
                        &mut globals[global_idx as usize],
                    );
                }
                OpCode::SwapGL => {
                    let global_idx = self.read_u16();
                    let local_idx = self.read_u16();
                    let mut globals = self.shared.globals.write().await;
                    std::mem::swap(
                        &mut globals[global_idx as usize],
                        &mut self.stack[self.bp as usize + local_idx as usize],
                    );
                }
                OpCode::SwapLL => {
                    let src = self.read_u16();
                    let dst = self.read_u16();
                    self.stack.swap(self.bp as usize + src as usize, self.bp as usize + dst as usize);
                }
                OpCode::Range => {
                    let key_idx = self.read_u16();
                    let value_idx = self.read_u16();
                    let mut obj = self.pop();
                    let iter = obj.as_iter();
                    let (k, v) = iter.next();
                    self.set_local(key_idx, k);
                    self.set_local(value_idx, v);
                }
                OpCode::Jump => {
                    let pos = self.read_u16();
                    self.jump(pos);
                }
                OpCode::JumpIfFalse => {
                    let condition = self.pop();
                    if condition.tag() != Type::Bool {
                        return Err(Error::TypeError(format!(
                            "expected a bool type got: {:#?} stack: {:#?}",
                            condition, self.stack
                        )));
                    }
                    let pos = self.read_u16();
                    if !condition.as_bool() {
                        self.jump(pos);
                    }
                }
                OpCode::Pop => {
                    final_result = self.pop();
                }
                OpCode::Null => {
                    self.push(Object::null());
                }
                OpCode::True => {
                    self.push(Object::bool(true));
                }
                OpCode::False => {
                    self.push(Object::bool(false));
                }
                OpCode::Add => impl_binary_op_method!(add),
                OpCode::Subtract => impl_binary_op_method!(sub),
                OpCode::Divide => {
                    let right = self.pop();
                    let left = self.pop();
                    let result = left.div(right)?;
                    self.push(result);
                }
                OpCode::Multiply => impl_binary_op_method!(mul),
                OpCode::Gt => impl_binary_op_method!(gt),
                OpCode::Gte => impl_binary_op_method!(gte),
                OpCode::Lt => impl_binary_op_method!(lt),
                OpCode::Lte => impl_binary_op_method!(lte),
                OpCode::Eq => impl_binary_op_method!(eq),
                OpCode::Neq => impl_binary_op_method!(neq),
                OpCode::Modulo => impl_binary_op_method!(rem),
                OpCode::And => {
                    let right = self.pop();
                    let left = self.pop();
                    let result = left.and(right)?;
                    self.push(result);
                }
                OpCode::Or => impl_binary_op_method!(or),
                OpCode::Not => {
                    let left = self.pop();
                    if left.tag() != Type::Bool {
                        return Err(Error::TypeError(format!(
                            "OpCode::Not: expected a boolean got: {:#?}",
                            left.tag()
                        )));
                    }
                    let result = Object::bool(!left.as_bool());
                    self.push(result);
                }
                OpCode::Negate => {
                    let left = self.pop();
                    let result = match left.tag() {
                        Type::Float64 => unsafe { Object::float64(-left.as_float64()) },
                        Type::Float32 => unsafe { Object::float32(-left.as_float32()) },
                        Type::Int => Object::int(-left.as_isize()),
                        Type::I8 => Object::int8(-left.as_int8().value),
                        Type::I16 => Object::int16(-left.as_int16().value),
                        Type::I32 => Object::int32(-left.as_int32().value),
                        Type::I64 => Object::int64(-left.as_int64().value),
                        _ => {
                            return Err(Error::TypeError(format!(
                                "cannot negate type: {:#?}",
                                left.tag()
                            )))
                        }
                    };
                    self.push(result);
                }
                OpCode::Call => {
                    let num_args = self.read_u8();
                    let base_pointer = self.stack.len() as u16 - 1 - num_args as u16;
                    let obj = self.pop();
                    let (ip, num_locals) = match obj.tag() {
                        Type::Function => {
                            let [ip, num_locals] = obj.as_function();
                            (ip, num_locals)
                        }
                        Type::Closure => {
                            self.closure_ctx.push(obj);
                            let closure = obj.as_closure();
                            if closure.is_null {
                                panic!("closure is nil: {:#?}", obj);
                            }
                            (closure.ip, closure.num_locals as u32)
                        }
                        _ => {
                            return Err(Error::TypeError(format!(
                                "expected a function|closure, got: {:#?}",
                                obj.tag()
                            )));
                        }
                    };
                    for _ in 0..num_locals - num_args as u32 {
                        self.push(Object::null());
                    }
                    self.pushframe(ip, base_pointer);
                }
                OpCode::Defer => {
                    let num_args = self.read_u8();
                    let func = self.pop();
                    let mut args = Vec::with_capacity(num_args as usize);
                    for _ in 0..num_args {
                        args.push(self.pop());
                    }
                    args.reverse();
                    self.deferred
                        .last_mut()
                        .unwrap()
                        .push(DeferredCall { func, args });
                }
                OpCode::CallBuiltin => {
                    let builtin_id = self.read_u8();
                    let num_args = self.read_u8() as usize;
                    let mut args = Vec::with_capacity(num_args);
                    for _ in 0..num_args {
                        args.push(self.pop());
                    }
                    args.reverse();
                    let b = unsafe { std::mem::transmute::<u8, builtin::Builtin>(builtin_id) };
                    if matches!(b, builtin::Builtin::Recover) {
                        let val = self.panic_value.take().unwrap_or(Object::null());
                        self.push(val);
                    } else {
                        match builtin::call(
                            b,
                            &args,
                            self.assert_stdout.as_mut().map(|a| &mut a.1),
                        ) {
                            Ok(result) => self.push(result),
                            Err(Error::GoPanic(v)) => {
                                self.panic_value = Some(v);
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }
                OpCode::ReturnValue => {
                    let num_r = self.read_u16();
                    let mut res = Vec::with_capacity(num_r as usize);
                    for _ in 0..num_r {
                        res.push(self.pop());
                    }
                    self.execute_deferred().await?;
                    self.popframe();
                    for re in res.iter().rev() {
                        self.push(re.clone());
                    }
                    self.closure_ctx.pop();
                    if self.frames.len() < initial_depth {
                        return Ok(final_result);
                    }
                }
                OpCode::Return => {
                    self.execute_deferred().await?;
                    self.popframe();
                    self.push(Object::null());
                    self.closure_ctx.pop();
                    if self.frames.len() < initial_depth {
                        return Ok(Object::null());
                    }
                }
                OpCode::GtLocalConst => impl_binary_const_local_op_method!(gt),
                OpCode::GteLocalConst => impl_binary_const_local_op_method!(gte),
                OpCode::LtLocalConst => impl_binary_const_local_op_method!(lt),
                OpCode::LteLocalConst => impl_binary_const_local_op_method!(lte),
                OpCode::EqLocalConst => impl_binary_const_local_op_method!(eq),
                OpCode::NeqLocalConst => impl_binary_const_local_op_method!(neq),
                OpCode::AddLocalConst => impl_binary_const_local_op_method!(add),
                OpCode::SubtractLocalConst => impl_binary_const_local_op_method!(sub),
                OpCode::MultiplyLocalConst => impl_binary_const_local_op_method!(mul),
                OpCode::DivideLocalConst => impl_binary_const_local_op_method!(div),
                OpCode::ModuloLocalConst => impl_binary_const_local_op_method!(rem),
                OpCode::Ref => {
                    let val = self.pop();
                    let r = Object::ref_t(val);
                    self.push(r);
                }
                OpCode::MakeSlice => {
                    let length = self.read_u16();
                    let ctv_id = self.read_u16();
                    let ctv = self.shared.constants[ctv_id as usize].as_type_value().clone();
                    let mut vec = Vec::with_capacity(length as usize);
                    for _ in 0..length {
                        vec.push(self.pop());
                    }
                    vec.reverse();
                    self.push(Slice::from_vec(vec, ctv));
                }
                OpCode::MakeArray => {
                    let length = self.read_u16();
                    let mut vec = Vec::with_capacity(length as usize);
                    for _ in 0..length {
                        vec.push(self.pop());
                    }
                    vec.reverse();
                    self.push(Object::array(vec));
                }
                OpCode::Map => {
                    let length = self.read_u16();
                    let mut map = BTreeMap::new();
                    for _ in 0..length {
                        let value = self.pop();
                        let key = self.pop();
                        map.insert(key, value);
                    }
                    let obj = Map::from_map(map);
                    self.push(obj);
                }
                OpCode::Struct => {
                    let length = self.read_u16();
                    let mut fields = Vec::with_capacity(length as usize);
                    for _ in 0..length {
                        let value = self.pop();
                        fields.push(value);
                    }
                    let strct = self.pop_ref_mut();
                    let strct = strct.as_struct();
                    let obj = Struct::object(
                        strct.name.clone(),
                        fields,
                        strct.method_dispatch.clone(),
                        strct.tags.clone(),
                        strct.is_anonymous,
                    );
                    self.pop();
                    self.push(obj);
                }
                OpCode::IndexGet => {
                    let index = self.pop();
                    let left = self.pop();
                    let (obj, found) = index_get(left, index)?;
                    self.push(obj);
                    if let Some(b) = found {
                        self.push(Object::bool(b))
                    }
                }
                OpCode::IntoIter => {
                    let obj = self.pop();
                    let iter = ObjIter::from_obj(obj);
                    self.push(iter);
                }
                OpCode::IndexSet => {
                    let value = self.pop();
                    let index = self.pop();
                    let left = self.pop();
                    index_set(left, index, value)?;
                    self.push(left);
                }
                OpCode::Halt => {
                    if let Some((expected, got_buf)) = &self.assert_stdout {
                        let got = String::from_utf8(got_buf.buffer().to_vec()).unwrap();
                        println!("asserting VM output");
                        assert_eq!(&got, expected);
                    }
                    self.shared.tracker.wait_all().await;
                    return Ok(final_result);
                }
                OpCode::GoSpawn => {
                    let num_args = self.read_u8();
                    let obj = self.pop();

                    let mut args = Vec::with_capacity(num_args as usize);
                    for _ in 0..num_args {
                        args.push(self.pop());
                    }
                    args.reverse();

                    let shared = Arc::clone(&self.shared);

                    let (ip, num_locals, closure_obj) = match obj.tag() {
                        Type::Function => {
                            let [ip, num_locals] = obj.as_function();
                            (ip, num_locals, None)
                        }
                        Type::Closure => {
                            let closure = obj.as_closure();
                            (closure.ip, closure.num_locals as u32, Some(obj))
                        }
                        _ => {
                            return Err(Error::TypeError(format!(
                                "go: expected function|closure, got: {:?}",
                                obj.tag()
                            )));
                        }
                    };

                    shared.tracker.spawn();

                    tokio::task::spawn(async move {
                        let mut g = Goroutine::for_spawn(
                            Arc::clone(&shared),
                            ip,
                            args,
                            num_locals,
                        );
                        if let Some(cl) = closure_obj {
                            g.closure_ctx.push(cl);
                        }
                        let _ = g.execute_loop().await;
                        shared.tracker.finish();
                    });
                }
                OpCode::ChanSend => {
                    let value = self.pop();
                    let ch_obj = self.pop();
                    if ch_obj.tag() != Type::Channel {
                        return Err(Error::TypeError(format!(
                            "send: expected channel, got {}",
                            ch_obj.tag()
                        )));
                    }
                    let ch = unsafe { Channel::read(&ch_obj) };
                    if ch.sender.send(value).await.is_err() {
                        return Err(Error::InternalError(
                            "send on closed channel".to_string(),
                        ));
                    }
                }
                OpCode::ChanRecv => {
                    let ch_obj = self.pop();
                    if ch_obj.tag() != Type::Channel {
                        return Err(Error::TypeError(format!(
                            "recv: expected channel, got {}",
                            ch_obj.tag()
                        )));
                    }
                    let ch = unsafe { Channel::read(&ch_obj) };
                    match ch.receiver.recv().await {
                        Ok(val) => self.push(val),
                        Err(_) => self.push(Object::null()),
                    }
                }
                OpCode::MakeChan => {
                    let cap_obj = self.pop();
                    let cap = if cap_obj.tag() == Type::Int {
                        cap_obj.as_isize() as usize
                    } else {
                        0
                    };
                    self.push(Channel::new(cap));
                }
                OpCode::ChanClose => {
                    let ch_obj = self.pop();
                    if ch_obj.tag() != Type::Channel {
                        return Err(Error::TypeError(format!(
                            "close: expected channel, got {}",
                            ch_obj.tag()
                        )));
                    }
                    let ch = unsafe { Channel::read_mut(&ch_obj) };
                    if ch.closed {
                        return Err(Error::GoPanic(Object::string("close of closed channel")));
                    }
                    ch.closed = true;
                    ch.sender.close();
                }
                OpCode::Select => {
                    let num_cases = self.read_u8() as usize;

                    const CASE_RECV: u8 = 0;
                    const CASE_SEND: u8 = 1;
                    const CASE_DEFAULT: u8 = 2;

                    let mut case_descs: Vec<(u8, u16)> = Vec::with_capacity(num_cases);
                    for _ in 0..num_cases {
                        let kind = self.read_u8();
                        let body_ip = self.read_u16();
                        case_descs.push((kind, body_ip));
                    }

                    struct CaseData {
                        kind: u8,
                        body_ip: u16,
                        channel: Option<Object>,
                        send_value: Option<Object>,
                    }

                    let mut case_data: Vec<CaseData> = Vec::with_capacity(num_cases);
                    for &(kind, body_ip) in case_descs.iter().rev() {
                        match kind {
                            CASE_SEND => {
                                let value = self.pop();
                                let ch = self.pop();
                                case_data.push(CaseData { kind, body_ip, channel: Some(ch), send_value: Some(value) });
                            }
                            CASE_RECV => {
                                let ch = self.pop();
                                case_data.push(CaseData { kind, body_ip, channel: Some(ch), send_value: None });
                            }
                            CASE_DEFAULT | _ => {
                                case_data.push(CaseData { kind, body_ip, channel: None, send_value: None });
                            }
                        }
                    }
                    case_data.reverse();

                    let mut selected: Option<usize> = None;
                    let mut recv_val: Option<Object> = None;
                    let mut default_idx: Option<usize> = None;

                    for (i, case) in case_data.iter().enumerate() {
                        match case.kind {
                            CASE_RECV => {
                                let ch_obj = case.channel.unwrap();
                                let ch = unsafe { Channel::read(&ch_obj) };
                                if let Ok(val) = ch.receiver.try_recv() {
                                    if let Some(ref ack_tx) = ch.ack_sender {
                                        let _ = ack_tx.try_send(());
                                    }
                                    selected = Some(i);
                                    recv_val = Some(val);
                                    break;
                                }
                            }
                            CASE_SEND => {
                                let ch_obj = case.channel.unwrap();
                                let ch = unsafe { Channel::read(&ch_obj) };
                                let value = case.send_value.unwrap();
                                if ch.sender.try_send(value).is_ok() {
                                    selected = Some(i);
                                    break;
                                }
                            }
                            CASE_DEFAULT => {
                                default_idx = Some(i);
                            }
                            _ => {}
                        }
                    }

                    if selected.is_none() {
                        if let Some(def_idx) = default_idx {
                            selected = Some(def_idx);
                        }
                    }

                    if selected.is_none() {
                        loop {
                            for (i, case) in case_data.iter().enumerate() {
                                match case.kind {
                                    CASE_RECV => {
                                        let ch_obj = case.channel.unwrap();
                                        let ch = unsafe { Channel::read(&ch_obj) };
                                        if let Ok(val) = ch.receiver.try_recv() {
                                            if let Some(ref ack_tx) = ch.ack_sender {
                                                let _ = ack_tx.try_send(());
                                            }
                                            selected = Some(i);
                                            recv_val = Some(val);
                                            break;
                                        }
                                    }
                                    CASE_SEND => {
                                        let ch_obj = case.channel.unwrap();
                                        let ch = unsafe { Channel::read(&ch_obj) };
                                        let value = case.send_value.unwrap();
                                        if ch.sender.try_send(value).is_ok() {
                                            selected = Some(i);
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            if selected.is_some() {
                                break;
                            }
                            tokio::task::yield_now().await;
                        }
                    }

                    let sel = selected.unwrap();
                    if let Some(val) = recv_val {
                        self.push(val);
                    }
                    self.ip = case_data[sel].body_ip as usize;
                }
            }
        }
        }) // Box::pin(async move { ... })
    }
}

pub struct VM {
    ip: usize,
    source_map: SourceMap,
}

impl VM {
    pub fn new() -> Self {
        Self {
            ip: 0,
            source_map: SourceMap::default(),
        }
    }

    pub async fn run(&mut self, code: Bytecode) -> Result<Object, Error> {
        self.source_map = code.source_map.clone();

        let shared = Arc::new(SharedState {
            instructions: code.instructions,
            constants: code.constants,
            globals: RwLock::new(Vec::with_capacity(8)),
            source_map: code.source_map,
            tracker: GoroutineTracker::new(),
        });

        let mut main_goroutine = Goroutine::new(Arc::clone(&shared));
        main_goroutine.assert_stdout = code.assert_stdout;
        self.ip = 0;

        let result = main_goroutine.execute_loop().await;
        self.ip = main_goroutine.ip;
        result
    }

    pub fn error_with_location(&self, err: &Error) -> String {
        if let Some(span) = self.source_map.lookup(self.ip) {
            format!("{}: {}", span, err)
        } else {
            format!("{}", err)
        }
    }
}

fn index_get(left: Object, index: Object) -> Result<(Object, Option<bool>), Error> {
    let let_obj = match left.tag() {
        Type::Ref => left.as_ref().value,
        _ => left,
    };

    match let_obj.tag() {
        Type::Array => {
            if index.tag() != Type::Int {
                return Err(Error::TypeError(format!(
                    "expected int index: {}",
                    index.tag()
                )));
            }
            Ok((index_get_array(let_obj.as_vec(), index.as_isize())?, None))
        }
        Type::Slice => {
            if index.tag() != Type::Int {
                return Err(Error::TypeError(format!(
                    "expected int index: {}",
                    index.tag()
                )));
            }
            Ok((index_get_array(let_obj.as_slice(), index.as_isize())?, None))
        }
        Type::String => {
            if index.tag() != Type::Int {
                return Err(Error::TypeError(format!(
                    "expected int index: {}",
                    index.tag()
                )));
            }
            Ok((index_get_string(let_obj, index.as_isize())?, None))
        }
        Type::Map => index_get_map(let_obj, index),
        Type::Struct => Ok((index_get_struct(let_obj, index)?, None)),
        _ => Err(Error::TypeError(format!(
            "object cannot be indexed: {}",
            left.tag()
        ))),
    }
}

fn index_get_struct(obj: Object, key: Object) -> Result<Object, Error> {
    let strct = obj.as_struct();
    let i = key.as_isize();
    if i < 0 {
        panic!("impossible");
    }
    Ok(strct.values[i as usize].clone())
}

fn index_get_map(obj: Object, key: Object) -> Result<(Object, Option<bool>), Error> {
    let map = obj.as_map();
    let r = match map.get(&key).cloned() {
        Some(v) => (v, Some(true)),
        None => (Object::null(), Some(false)),
    };
    Ok(r)
}

fn index_set_map(mut left: Object, index: Object, value: Object) -> Result<(), Error> {
    let map = left.as_map_mut();
    map.insert(index, value);
    Ok(())
}

fn index_get_array(array: &Vec<Object>, mut index: isize) -> Result<Object, Error> {
    if index < 0 {
        index += array.len() as isize;
    }
    let index = index as usize;
    if index >= array.len() {
        return Err(Error::IndexError("out of bounds".to_string()));
    }
    Ok(array[index])
}

fn index_get_string(obj: Object, index: isize) -> Result<Object, Error> {
    let s = obj.as_str();
    if index < 0 {
        return Err(Error::IndexError("index out of bounds".to_string()));
    }
    let i = index as usize;
    if i >= s.len() {
        return Err(Error::IndexError("index out of bounds".to_string()));
    }
    let result = Object::uint8(s.as_bytes()[i]);
    Ok(result)
}

fn index_set(mut left: Object, index: Object, value: Object) -> Result<(), Error> {
    if left.tag() == Type::Map {
        return index_set_map(left, index, value);
    }
    if index.tag() != Type::Int {
        return Err(Error::TypeError(format!(
            "index should be int {}",
            index.tag()
        )));
    }
    match left.tag() {
        Type::Array => index_set_array(left.as_vec_mut(), index.as_isize(), value)?,
        Type::Slice => index_set_array(left.as_slice_mut(), index.as_isize(), value)?,
        Type::String => index_set_string(left.as_string_mut(), index.as_isize(), value)?,
        Type::Struct => index_set_struct(left, index.as_isize() as usize, value)?,
        Type::Ref => index_set(left.as_ref().value, index, value)?,
        _ => {
            return Err(Error::TypeError(format!(
                "index_set: invalid type {:#?}",
                left
            )))
        }
    }
    Ok(())
}

fn index_set_struct(mut left: Object, index: usize, value: Object) -> Result<(), Error> {
    let strct = left.as_struct_mut();
    strct.values[index] = value;
    Ok(())
}

fn index_set_array(array: &mut Vec<Object>, mut index: isize, value: Object) -> Result<(), Error> {
    if index < 0 {
        index += array.len() as isize;
    }
    let index = index as usize;
    if index >= array.len() {
        return Err(Error::IndexError(
            "index_set_array: out of bounds".to_string(),
        ));
    }
    array[index] = value;
    Ok(())
}

fn index_set_string(string: &mut String, mut index: isize, value: Object) -> Result<(), Error> {
    let strlen = string.chars().count();
    if index < 0 {
        index += strlen as isize;
    }
    let index = index as usize;
    if index >= strlen {
        return Err(Error::IndexError("out of bounds".to_string()));
    }
    if value.tag() != Type::String {
        return Err(Error::TypeError("expected string".to_string()));
    }
    string.replace_range(
        string
            .char_indices()
            .nth(index)
            .map(|(pos, ch)| (pos..pos + ch.len_utf8()))
            .unwrap(),
        value.as_str(),
    );
    Ok(())
}

#[derive(Debug, PartialEq)]
pub enum Error {
    TypeError(String),
    SyntaxError(String),
    ReferenceError(String),
    IndexError(String),
    ArgumentError(String),
    InternalError(String),
    GoPanic(Object),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TypeError(s) => write!(f, "TypeError: {}", s),
            Error::SyntaxError(s) => write!(f, "SyntaxError: {}", s),
            Error::ReferenceError(s) => write!(f, "ReferenceError: {}", s),
            Error::IndexError(s) => write!(f, "IndexError: {}", s),
            Error::ArgumentError(s) => write!(f, "ArgumentError: {}", s),
            Error::InternalError(s) => write!(f, "InternalError: {}", s),
            Error::GoPanic(obj) => write!(f, "panic: {}", obj),
        }
    }
}
