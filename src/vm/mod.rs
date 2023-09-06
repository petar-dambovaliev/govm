pub mod builtin;
pub mod compiler;
pub mod gc;
pub mod object;
pub mod symbols;

use std::collections::BTreeMap;
//use std::default::Default;
use std::fmt::Debug;

#[cfg(feature = "debug")]
use std::io::Write;
use std::ptr;

#[cfg(feature = "debug")]
use crate::compiler::bytecode_to_human;
use crate::vm::compiler::{bytecode_to_human, Bytecode, OpCode};
use crate::vm::gc::GC;
use crate::vm::object::collections::{Map, ObjIter};
use crate::vm::object::r#ref::Ref;
use crate::vm::object::structure::Struct;
use crate::vm::object::{FromString, FromVec, Object, Type};

#[derive(Copy, Clone, Debug)]
struct Frame {
    /// Index of the current instruction
    ip: usize,

    /// Pointer to the index of the stack before function call started
    /// This is where the VM returns its stack to after the function returns
    base_pointer: u16,
}

impl Frame {
    #[inline(always)]
    fn new(ip: usize, base_pointer: u16) -> Self {
        Frame { ip, base_pointer }
    }
}

pub struct VM {
    stack: Vec<Object>,
    globals: Vec<Object>,
    frames: Vec<Frame>,
    instructions: Vec<u8>,
    ip: usize,
    bp: u16,
    closure_ctx: Vec<Object>,
    escaped: Vec<Object>,
}

impl VM {
    // #[inline]
    // fn cur_mem(&self) -> usize {
    //     self.heap.len()
    // }
    //
    // #[inline]
    // fn prev_mem(&self) -> usize {
    //     self.last_gc_round_mem
    // }
    //
    // #[inline]
    // fn run_gc(&mut self) {
    //     let percent = (self.opts.gogc * self.prev_mem() as f64) / 100.0;
    //     let target = percent as usize + self.prev_mem();
    //
    //     if self.opts.min_gc < target && self.cur_mem() >= target {
    //         dbg!(
    //             "running GC: current memory: {} target memory: {}",
    //             self.cur_mem(),
    //             target
    //         );
    //         self.heap.clean();
    //     }
    // }

    /// Creates a new VM with an empty stack and callframes vector
    pub fn new() -> Self {
        let mut frames = Vec::with_capacity(128);
        frames.push(Frame::new(0, 0));

        Self {
            stack: Vec::with_capacity(128),
            globals: Vec::with_capacity(8),
            frames,
            instructions: Vec::new(),
            ip: 0,
            bp: 0,
            closure_ctx: Vec::with_capacity(10),
            escaped: Vec::with_capacity(128),
        }
    }

    /// Get a local variable (stored on the stack)
    /// The passed index is the relative position to the base pointer of the current callframe
    /// Performance: Skipping the bounds check here does not yield any significant performance improvement
    #[inline(always)]
    fn get_local(&self, rel_idx: u16) -> Object {
        self.stack[self.bp as usize + rel_idx as usize]
    }

    /// Store a local variable (on the stack)
    /// The passed index is the relative position to the base pointer of the current callframe
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
    fn copy_gl(&mut self, src_idx: u16, dst_idx: u16) {
        self.stack[self.bp as usize + dst_idx as usize] = self.globals[src_idx as usize];
    }

    #[inline(always)]
    fn copy_lg(&mut self, src_idx: u16, dst_idx: u16) {
        self.globals[src_idx as usize] = self.stack[self.bp as usize + dst_idx as usize];
    }

    #[inline(always)]
    fn copy_gg(&mut self, src_idx: u16, dst_idx: u16) {
        self.globals[src_idx as usize] = self.globals[dst_idx as usize];
    }

    #[inline(always)]
    fn swap_ll(&mut self, src_idx: u16, dst_idx: u16) {
        self.stack.swap(src_idx as usize, dst_idx as usize);
    }

    #[inline(always)]
    fn swap_gl(&mut self, src_idx: u16, dst_idx: u16) {
        std::mem::swap(
            &mut self.stack[self.bp as usize + dst_idx as usize],
            &mut self.globals[src_idx as usize],
        );
    }

    #[inline(always)]
    fn swap_lg(&mut self, src_idx: u16, dst_idx: u16) {
        std::mem::swap(
            &mut self.stack[self.bp as usize + dst_idx as usize],
            &mut self.globals[src_idx as usize],
        );
    }

    #[inline(always)]
    fn swap_gg(&mut self, src_idx: u16, dst_idx: u16) {
        self.globals.swap(src_idx as usize, dst_idx as usize);
    }

    // fn get_propagate(&mut self, rel_idx: u16) -> Object {
    //     let obj = self.closure_ctx.last_mut().unwrap();
    //     let closure = obj.as_closure();
    //
    //     closure.propagate_objects[rel_idx as usize]
    // }
    fn get_enclosed(&mut self, rel_idx: u16) -> Object {
        self.escaped[rel_idx as usize]
    }
    #[inline(always)]
    fn set_local_enclosed(&mut self, rel_idx: u16, value: Object) {
        self.escaped[rel_idx as usize] = value;
    }

    #[inline(always)]
    fn enclosed_ptr_write(&mut self, rel_idx: u16, value: Object) {
        println!("123");
        let ptr = self.escaped[rel_idx as usize].as_ref_mut();
        assert_eq!(ptr.value.tag(), value.tag());

        let (ptr_inner, val_inner) = match ptr.value.tag() {
            Type::Int => (ptr.value.as_int_mut(), value.as_isize()),
            _ => panic!("not supported ptr write"),
        };

        ptr_inner.value = val_inner;
    }

    #[inline(always)]
    fn local_ptr_write(&mut self, rel_idx: u16, value: Object) {
        let ptr = self.stack[self.bp as usize + rel_idx as usize].as_ref_mut();
        assert_eq!(ptr.value.tag(), value.tag());

        let (ptr_inner, val_inner) = match ptr.value.tag() {
            Type::Int => (ptr.value.as_int_mut(), value.as_isize()),
            _ => panic!("not supported ptr write"),
        };

        ptr_inner.value = val_inner;
    }

    #[inline(always)]
    fn global_ptr_write(&mut self, rel_idx: u16, value: Object) {
        if self.globals[rel_idx as usize].is_null() {
            panic!("nil pointer dereference");
        }
        let ptr = self.globals[rel_idx as usize].as_ref_mut();
        assert_eq!(ptr.value.tag(), value.tag());

        let (ptr_inner, val_inner) = match ptr.value.tag() {
            Type::Int => (ptr.value.as_int_mut(), value.as_isize()),
            _ => panic!("not supported ptr write"),
        };

        ptr_inner.value = val_inner;
    }

    /// Reads a u16 value from the current position in the instructions array
    #[inline(always)]
    fn read_u8(&mut self) -> u8 {
        let v = unsafe { *self.instructions.get_unchecked(self.ip) };
        self.ip += 1;
        v
    }

    /// Reads a u16 value from the current position in the instructions array
    #[inline(always)]
    fn read_u16(&mut self) -> u16 {
        let start = self.ip;
        self.ip += 2;
        let bytes = unsafe { self.instructions.get_unchecked(start..self.ip) };
        bytes[0] as u16 | (bytes[1] as u16) << 8
    }

    /// Sets the instruction pointer to the given value
    #[inline(always)]
    fn jump(&mut self, ip: u16) {
        self.ip = ip as usize;
    }

    /// Reads the next OpCode from the instructions vector
    /// This function still accounts for 25-35% of runtime right now...
    #[inline(always)]
    fn next(&mut self) -> OpCode {
        // Safety: if compiler did its job correctly, IP will always be in bounds
        // Performance: skipping the bounds check yields a 22% performance improvement
        let byte = unsafe { *self.instructions.get_unchecked(self.ip) };
        self.ip += 1;
        OpCode::from(byte)
    }

    #[inline(always)]
    fn peek_next(&self) -> OpCode {
        // Safety: if compiler did its job correctly, IP will always be in bounds
        // Performance: skipping the bounds check yields a 22% performance improvement
        let byte = unsafe { *self.instructions.get_unchecked(self.ip) };
        OpCode::from(byte)
    }

    fn peak_instruction(&self) -> Option<OpCode> {
        self.instructions.get(self.ip + 1).map(|a| OpCode::from(*a))
    }

    fn ignore_next_instruction(&mut self) {
        let new_ip = self.ip + 1;
        if self.instructions.len() < new_ip {
            self.ip = new_ip;
        }
    }

    /// Pop an object off the stack
    /// This is like `Vec::pop`, but without checking if it's empty first.
    /// Performance: -25% over a regular call to `Vec::pop()`
    #[inline(always)]
    fn pop(&mut self) -> Object {
        debug_assert!(!self.stack.is_empty());

        // Safety: if the compiler and VM are implemented correctly, the stack will never be empty
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

    /// Push a new object on the stack
    #[inline(always)]
    fn push(&mut self, obj: Object) {
        self.stack.push(obj)
    }

    /// Pop a callframe and return IP to the IP of the last callframe
    /// This also truncates the stack back to SP from when this frame was pushed
    #[inline(always)]
    fn popframe(&mut self) {
        // pop frame and return stack to frame's base pointer
        let frame = self.frames.pop().unwrap();
        self.stack.truncate(frame.base_pointer as usize);

        // copy base pointer and instruction pointer out of new current frame
        // this yields an enormous performance improvement
        let frame = self.frames.last().unwrap();
        self.ip = frame.ip;
        self.bp = frame.base_pointer;
    }

    /// Push new callframe with the given IP and Base Pointer
    #[inline(always)]
    fn pushframe(&mut self, ip: u32, base_pointer: u16) {
        // store current IP into the frame that we're leaving
        // so we can return to it later
        let frame = self.frames.last_mut().unwrap();
        frame.ip = self.ip;

        // push new frame and copy over IP and BP
        // this somehow yields an enormous performance improvent
        self.frames.push(Frame::new(ip as usize, base_pointer));
        self.ip = ip as usize;
        self.bp = base_pointer;
    }

    /// Executes the given Bytecode inside the context of this VM
    pub fn run(&mut self, code: Bytecode) -> Result<Object, Error> {
        //#[cfg(feature = "debug")]
        {
            println!("Bytecode (raw)= \n{:?}", &code.instructions);
            print!(
                "Bytecode (human)= {}\n",
                bytecode_to_human(&code.instructions, true)
            );
            println!("{:16}= {:?}", "Constants", code.constants);
            println!("{:16}= {:?}", "Frames", self.frames);
        }

        // reset some state
        self.instructions = code.instructions;
        self.ip = 0;
        self.bp = 0;
        self.frames[0].ip = 0;
        self.frames[0].base_pointer = 0;

        // Keep your friends close
        let constants = code.constants;
        let mut final_result = Object::null();

        // Construct a new garbage collector
        // And allow to manage memory for constants
        let gc = &mut GC::new();

        macro_rules! impl_binary_op_method {
            ($op:tt) => {{
                let right = self.pop();
                let left = self.pop();
                let result = left.$op(right, gc)?;
                self.push(result);
            }};
        }

        macro_rules! impl_binary_const_local_op_method {
            ($op:tt) => {{
                let local_idx = self.read_u16();
                let left = self.get_local(local_idx);
                let constant_idx = self.read_u16();
                let right = constants[constant_idx as usize];
                //println!("local: {:#?} const: {:#?}", local_idx, constant_idx);
                let result = left.$op(right, gc)?;
                self.push(result);
            }};
        }

        //#[cfg(feature = "debug")]
        //let mut debug_pause = 0;

        //#[cfg(feature = "debug")]
        // Buffer used to capture input from stdin during stepped debugging
        //let mut buffer = String::new();
        loop {
            //#[cfg(feature = "debug")]
            // {
            //     println!(
            //         "{:16}= {}/{}: {}",
            //         "Instruction",
            //         self.ip,
            //         self.instructions.len() - 1,
            //         // This prints the OpCode along with all of its operand values (in decimal form)
            //         bytecode_to_human(&self.instructions[self.ip..], false)
            //             .split(" ")
            //             .next()
            //             .unwrap()
            //     );
            //     print!("{:16}= [", "Globals");
            //     for (i, v) in self.globals.iter().enumerate() {
            //         print!("{}{}: {:?}", if i > 0 { ", " } else { "" }, i, v)
            //     }
            //     println!("]");
            //     print!("{:16}= [", "Stack");
            //     for (i, v) in self.stack.iter().enumerate() {
            //         print!("{}{}: {:?}", if i > 0 { ", " } else { "" }, i, v)
            //     }
            //     println!("]");
            //
            //     if debug_pause == 0 {
            //         print!("{} ", ">".repeat(40));
            //         std::io::stdout().flush().unwrap();
            //         buffer.clear();
            //         std::io::stdin().read_line(&mut buffer).unwrap();
            //         debug_pause = buffer.trim().parse().unwrap_or(1) - 1;
            //     } else {
            //         println!("{} ", ">".repeat(40));
            //         debug_pause -= 1;
            //     }
            // }
            //println!("{:#?}--{:#?}", self.peek_next(), self.stack);
            //println!("{:#?}", self.stack);
            match self.next() {
                OpCode::Const => {
                    let idx = self.read_u16();
                    let value = constants[idx as usize];
                    //println!("const: {:#?}", value);
                    self.push(value);
                }
                OpCode::Deref => {
                    let val = self.pop();
                    let r = val.as_ref();
                    self.push(r.value);
                }
                OpCode::Escape => {
                    let idx = self.read_u16();
                    let value = self.get_local(idx);
                    self.escaped.push(value);
                }
                OpCode::SetGlobal => {
                    let idx = self.read_u16() as usize;
                    //println!("SetGlobal-before: {:#?}", self.stack);
                    let value = self.pop();

                    while self.globals.len() <= idx {
                        self.globals.push(Object::null());
                    }

                    self.globals[idx] = value;
                    //println!("SetGlobal-after: {:#?}", self.stack);
                }
                OpCode::GetGlobal => {
                    let idx = self.read_u16();
                    //println!("GetGlobal-before: {:#?}", constants);
                    let value = self.globals[idx as usize];
                    self.push(value);
                    //println!("GetGlobal-after: {:#?}", self.stack);
                }
                OpCode::SetLocal => {
                    let idx = self.read_u16();
                    //println!("SetLocal-before: {:#?}", self.stack);
                    let value = self.pop();
                    self.set_local(idx, value);
                    //println!("SetLocal-after: {:#?}", self.stack);
                }
                OpCode::GetLocal => {
                    let idx = self.read_u16();
                    //println!("GetLocal-before: {:#?}", self.stack);
                    //println!("id: {} bp: {}", idx, self.bp);
                    let value = self.get_local(idx);
                    self.push(value);
                    //println!("GetLocal-after: {:#?}", self.stack);
                }
                OpCode::SetEnclosed => {
                    let idx = self.read_u16();
                    let value = self.pop();
                    self.set_local_enclosed(idx, value);
                }
                OpCode::GetEnclosed => {
                    let idx = self.read_u16();
                    let value = self.get_enclosed(idx);
                    self.push(value);
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
                    while self.globals.len() <= idx as usize {
                        self.globals.push(Object::null());
                    }
                    self.global_ptr_write(idx, value);
                }
                OpCode::CopyGG => {
                    let src = self.read_u16();
                    let dst = self.read_u16();

                    self.copy_gg(src, dst);
                }
                OpCode::CopyLG => {
                    let src = self.read_u16();
                    let dst = self.read_u16();

                    self.copy_lg(src, dst);
                }
                OpCode::CopyGL => {
                    let src = self.read_u16();
                    let dst = self.read_u16();

                    self.copy_gl(src, dst);
                }
                OpCode::CopyLL => {
                    let src = self.read_u16();
                    let dst = self.read_u16();

                    self.copy_ll(src, dst);
                }
                OpCode::SwapGG => {
                    let src = self.read_u16();
                    let dst = self.read_u16();

                    self.swap_gg(src, dst);
                }
                OpCode::SwapLG => {
                    let src = self.read_u16();
                    let dst = self.read_u16();

                    self.swap_lg(src, dst);
                }
                OpCode::SwapGL => {
                    let src = self.read_u16();
                    let dst = self.read_u16();

                    self.swap_gl(src, dst);
                }
                OpCode::SwapLL => {
                    let src = self.read_u16();
                    let dst = self.read_u16();

                    self.swap_ll(src, dst);
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

                    // collect garbage on every jump instruction
                    // gc.run(&[&self.stack, &constants, &self.globals, &[final_result]]);
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
                    //println!("pop: {:#?}", final_result);
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
                OpCode::Divide => impl_binary_op_method!(div),
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
                    let result = left.and(right, gc)?;
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
                        Type::Float => unsafe { Object::float(-left.as_float(), gc) },
                        Type::Int => Object::int(-left.as_isize()),
                        _ => {
                            return Err(Error::TypeError(format!(
                                "expected float or int, got: {:#?}",
                                left.tag()
                            )))
                        }
                    };
                    self.push(result);
                }
                OpCode::Call => {
                    let num_args = self.read_u8();
                    //println!("{:#?}", self.stack);
                    let base_pointer = self.stack.len() as u16 - 1 - num_args as u16;
                    let mut obj = self.pop();

                    let (ip, num_locals) = match obj.tag() {
                        Type::Function => {
                            let [ip, num_locals] = obj.as_function();
                            (ip, num_locals)
                        }
                        Type::Closure => {
                            self.closure_ctx.push(obj);
                            let closure = obj.as_closure();
                            (closure.ip, closure.num_locals as u32)
                        }
                        _ => {
                            //panic!("ins: {:#?} - {:#?}", self.peek_next(), self.stack);
                            return Err(Error::TypeError(format!(
                                "expected a function|closure, got: {:#?}",
                                obj.tag()
                            )));
                        }
                    };

                    // Make room on the stack for any local variables defined inside this function
                    // for _ in 0..num_locals - num_args as u32 {
                    //     self.push(Object::null());
                    // }

                    self.pushframe(ip, base_pointer);
                }
                OpCode::CallBuiltin => {
                    //todo
                    //this doesn't need to move memory
                    // change builtin call to accept a reversed iterator
                    // also take all arguments from the stack in 1 op

                    let builtin = self.read_u8();
                    //println!("builtin: {}", builtin);
                    let num_args = self.read_u8() as usize;
                    let mut args = Vec::with_capacity(num_args);
                    for _ in 0..num_args {
                        args.push(self.pop());
                    }
                    args.reverse();

                    let builtin = unsafe { std::mem::transmute::<u8, builtin::Builtin>(builtin) };
                    let result = builtin::call(builtin, &args, gc)?;
                    self.push(result);
                }
                OpCode::ReturnValue => {
                    let num_r = self.read_u16();

                    let mut res = Vec::with_capacity(num_r as usize);

                    for _ in 0..num_r {
                        res.push(self.pop());
                    }
                    //println!("{:#?}", res);
                    //println!("before popframe: {:#?}", self.stack);

                    self.popframe();

                    //println!("after popframe: {:#?}", self.stack);

                    for re in res {
                        self.push(re);
                    }

                    self.closure_ctx.pop();
                }
                OpCode::Return => {
                    self.popframe();
                    self.push(Object::null());
                    self.closure_ctx.pop();
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
                    self.push(Object::ref_t(val, gc));
                }
                OpCode::Array => {
                    let length = self.read_u16();
                    let mut vec = Vec::with_capacity(length as usize);
                    for _ in 0..length {
                        vec.push(self.pop());
                    }
                    vec.reverse();
                    // TODO: Re-use vector allocation here
                    let obj = Object::array(vec, gc);
                    self.push(obj);
                }
                OpCode::Map => {
                    let length = self.read_u16();
                    let mut map = BTreeMap::new();
                    for _ in 0..length {
                        let value = self.pop();
                        let key = self.pop();
                        map.insert(key, value);
                    }

                    let obj = Map::from_map(map, gc);
                    self.push(obj);
                }
                OpCode::Struct => {
                    let struct_name = self.pop();
                    let length = self.read_u16();

                    let mut fields = Vec::with_capacity(length as usize);
                    for _ in 0..length {
                        let value = self.pop();
                        fields.push(value);
                    }

                    let obj = Struct::object(struct_name.to_string(), fields);
                    self.push(obj);
                }
                OpCode::IndexGet => {
                    let index = self.pop();
                    let left = self.pop();
                    let result = index_get(left, index, gc)?;
                    self.push(result);
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
                    println!("value: {:#?} index: {:#?} left: {:#?}", value, index, left);
                    index_set(left, index, value)?;
                    self.push(left);
                }
                OpCode::Halt => {
                    gc.untrace(final_result);
                    return Ok(final_result);
                }
            }
        }
    }
}

fn index_get(left: Object, index: Object, gc: &mut GC) -> Result<Object, Error> {
    let let_obj = match left.tag() {
        Type::Ref => left.as_ref().value,
        _ => left,
    };

    let result = match let_obj.tag() {
        Type::Array => {
            if index.tag() != Type::Int {
                return Err(Error::TypeError(format!(
                    "expected int index: {}",
                    index.tag()
                )));
            }
            index_get_array(let_obj, index.as_isize())
        }
        Type::String => {
            if index.tag() != Type::Int {
                return Err(Error::TypeError(format!(
                    "expected int index: {}",
                    index.tag()
                )));
            }

            index_get_string(let_obj, index.as_isize(), gc)
        }
        Type::Map => index_get_map(let_obj, index, gc),
        Type::Struct => index_get_struct(let_obj, index, gc),
        _ => {
            return Err(Error::TypeError(format!(
                "object cannot be indexed: {}",
                left.tag()
            )))
        }
    }?;

    Ok(result)
}

fn index_get_struct(obj: Object, key: Object, gc: &mut GC) -> Result<Object, Error> {
    let strct = obj.as_struct();
    let i = key.as_isize();

    if i < 0 {
        panic!("impossible");
    }

    Ok(strct.values[i as usize].clone())
}

fn index_get_map(obj: Object, key: Object, gc: &mut GC) -> Result<Object, Error> {
    let map = obj.as_map();
    let res = map.get(&key).cloned().unwrap_or(Object::null());
    Ok(res)
}

fn index_set_map(mut left: Object, index: Object, value: Object) -> Result<(), Error> {
    let map = left.as_map_mut();
    //isert returns the old value
    // later for the gc
    map.insert(index, value);

    Ok(())
}

fn index_get_array(obj: Object, mut index: isize) -> Result<Object, Error> {
    let array = obj.as_vec();
    if index < 0 {
        index += array.len() as isize;
    }
    let index = index as usize;
    if index >= array.len() {
        return Err(Error::IndexError("out of bounds".to_string()));
    }

    Ok(array[index])
}

fn index_get_string(obj: Object, index: isize, gc: &mut GC) -> Result<Object, Error> {
    let str = obj.as_str();
    if index < 0 {
        return Err(Error::IndexError("i: out of bounds".to_string()));
    }

    let i = index as usize + 1;
    if i >= str.len() - 1 {
        return Err(Error::IndexError("out of bounds".to_string()));
    }

    let ch = str.chars().nth(i).unwrap();
    let result = Object::string(ch.to_string(), gc);
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
}

// #[derive(Default, Debug)]
// pub struct Opts {
//     // percantage of the heap increasing
//     // to trigger a garbage collection cycle
//     pub gogc: f64,
//     // min heap size in bytes to trigger a
//     // garbage collection cycle
//     pub min_gc: usize,
// }

// #[derive(Default, Debug)]
// struct Stats {
//     allocs: usize,
//     prev_allocs: usize,
// }
