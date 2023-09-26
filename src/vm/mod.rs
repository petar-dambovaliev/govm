pub mod builtin;
pub mod compiler;
pub mod object;
pub mod symbols;

use std::collections::{BTreeMap, VecDeque};
//use std::default::Default;
use std::fmt::Debug;

#[cfg(feature = "debug")]
use std::io::Write;
use std::ptr;

#[cfg(feature = "debug")]
use crate::compiler::bytecode_to_human;
use crate::vm::compiler::{bytecode_to_human, Bytecode, OpCode};
use crate::vm::object::collections::{Array, Map, ObjIter, Slice, Variadic};
use crate::vm::object::structure::{Interface, Struct, TypeValue};
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

    #[inline(always)]
    fn global_ptr_write(&mut self, rel_idx: u16, value: Object) {
        if self.globals[rel_idx as usize].is_null() {
            panic!("global_ptr_write: nil pointer dereference");
        }
        let ctx = self.globals[rel_idx as usize];

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

    #[allow(unused)]
    #[inline(always)]
    fn peek_next(&self) -> OpCode {
        // Safety: if compiler did its job correctly, IP will always be in bounds
        // Performance: skipping the bounds check yields a 22% performance improvement
        let byte = unsafe { *self.instructions.get_unchecked(self.ip) };
        OpCode::from(byte)
    }

    #[allow(unused)]
    fn peak_instruction(&self) -> Option<OpCode> {
        self.instructions.get(self.ip + 1).map(|a| OpCode::from(*a))
    }

    #[allow(unused)]
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
                let right = constants[constant_idx as usize];
                //println!("local: {:#?} const: {:#?}", local_idx, constant_idx);
                let result = left.$op(right)?;
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

            // println!(
            //     "instr=>{:#?} const6=>{}",
            //     self.peek_next(),
            //     constants[7].as_isize()
            // );
            match self.next() {
                OpCode::TypedNull => {
                    let num = self.read_u16() as usize;
                    let _p = self.pop();
                    //println!("popped: {:#?}", p);
                    let c = constants[num];

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
                            _ => unimplemented!(),
                        }
                    } else {
                        let vec: Vec<Object> = Vec::from(args);
                        self.push(Variadic::from_vec(vec));
                    }
                }
                OpCode::Slice => {
                    let ctv_id = self.read_u16();
                    let num = self.read_u8();
                    let ctv = constants[ctv_id as usize].as_type_value().clone();

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
                OpCode::IncLocal => {
                    let id = self.read_u16();
                    //println!("{:#?}-{:#?}-{:#?}", id, self.bp, self.stack);
                    let val = &mut self.stack[self.bp as usize + id as usize];
                    let new_val = val.as_int_mut();
                    new_val.value += 1;
                }
                OpCode::IncGlobal => {
                    let id = self.read_u16();
                    let val = &mut self.globals[self.bp as usize + id as usize];
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
                        panic!("OpCode::PanicIfFalse: got false");
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
                    unimplemented!()
                    // let value = self.pop();
                    //
                    // if value.is_ref() {}
                    //
                    // self.push(TypeValue::object(value.tag()))
                }
                OpCode::Downcast => {
                    let value = self.pop();
                    //println!("{}", value);
                    let iface = unsafe { Interface::read(&value) };
                    //println!("{}", rref);
                    self.push(iface.value);
                }
                OpCode::DynamicDispatch => {
                    let num_args = self.read_u16();
                    let method_id = self.read_u16();

                    let value = self.pop();
                    let iface = unsafe { Interface::read(&value) };
                    let method_name = &iface.methods[method_id as usize];

                    let strct = iface.value.as_struct();

                    for (name, ip) in &strct.method_dispatch {
                        if method_name == name {
                            let base_pointer = self.stack.len() as u16 - num_args;
                            //println!("base_pointer: {:#?}", name);
                            self.pushframe(*ip as u32, base_pointer);
                            break;
                        }
                    }
                }
                OpCode::Upcast => {
                    let iface_id = self.read_u16();
                    let value = self.pop();
                    let c = self.globals[iface_id as usize];
                    if c.tag() != Type::Interface {
                        panic!(
                            "expected interface: got {:#?} globals: {:#?} id: {:#?}",
                            c, self.globals, iface_id
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
                    let value = constants[idx as usize];
                    // if idx == 8 {
                    //     println!("const: {:#?} tag: {:#?}", value, value.as_ptr());
                    // }
                    //println!("const: {:#?} tag: {:#?}", value, value.tag());
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
                    //println!("propagate: {:#?}", value.as_ptr());
                    *c = value;
                }
                OpCode::SetGlobal => {
                    let idx = self.read_u16() as usize;
                    //println!("SetGlobal-before: {:#?}", constants);
                    let value = self.pop();

                    while self.globals.len() <= idx {
                        self.globals.push(Object::null());
                    }

                    self.globals[idx] = value;
                    //println!("SetGlobal-after: id {:#?} => value {:#?}", idx, value);
                }
                OpCode::GetGlobal => {
                    let idx = self.read_u16();
                    //println!("GetGlobal: {:#?} id: {}", self.globals, idx);
                    let value = self.globals[idx as usize];
                    self.push(value);
                    //println!("GetGlobal-after: {:#?}", self.stack);
                }
                OpCode::SetLocal => {
                    let idx = self.read_u16();
                    let value = self.pop();
                    // if idx == 0 {
                    //     println!("setlocal: {:#?}", value);
                    // }
                    // println!(
                    //     "setlocal: {:#?} to {:#?} {:#?}",
                    //     idx + self.bp,
                    //     value,
                    //     value.as_ptr()
                    // );
                    //println!("id: {:#?} bp: {:#?}", idx, self.bp);
                    self.set_local(idx, value.clone());
                }
                OpCode::GetLocal => {
                    let idx = self.read_u16();

                    //println!("id: {} bp: {} stack: {:#?}", idx, self.bp, self.stack);
                    let value = self.get_local(idx);
                    //
                    self.push(value);
                    //println!("GetLocal-after: {:#?}", self.stack);
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
                        //panic!("{:#?}", self.stack);
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
                OpCode::Divide => {
                    let right = self.pop();
                    let left = self.pop();
                    let result = left.div(right)?;
                    self.push(result);
                }
                //impl_binary_op_method!(div),
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
                        Type::Float => unsafe { Object::float(-left.as_float()) },
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
                    //println!("CALL");
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
                            //panic!("ins: {:#?} - {:#?}", self.peek_next(), self.stack);
                            return Err(Error::TypeError(format!(
                                "expected a function|closure, got: {:#?}",
                                obj.tag()
                            )));
                        }
                    };

                    // Make room on the stack for any local variables defined inside this function
                    for _ in 0..num_locals - num_args as u32 {
                        self.push(Object::null());
                    }

                    self.pushframe(ip, base_pointer);
                }
                OpCode::CallBuiltin => {
                    //panic!("{:#?}", self.stack);
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
                    let result = builtin::call(builtin, &args)?;
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

                    //println!("res: {:#?}", num_r);

                    for re in res.iter().rev() {
                        self.push(re.clone());
                    }

                    self.closure_ctx.pop();
                }
                OpCode::Return => {
                    //println!("return");
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
                    /// here is the bug
                    let r = Object::ref_t(val);
                    ////
                    self.push(r);
                }
                OpCode::MakeSlice => {
                    let ctv_id = self.read_u16();
                    let ctv = constants[ctv_id as usize].as_type_value().clone();

                    let length = self.read_u16();
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
                        strct.is_anonymous,
                    );
                    // remove struct const from the stack
                    self.pop();

                    self.push(obj);
                    //println!("{:#?}", self.stack);
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
                    //println!("{:#?}", self.stack);
                    let value = self.pop();
                    let index = self.pop();
                    let left = self.pop();
                    //println!("value: {:#?} index: {:#?} left: {:#?}", value, index, left);
                    index_set(left, index, value)?;
                    self.push(left);
                }
                OpCode::Halt => {
                    //println!("Halt: {:#?}", self.stack);
                    return Ok(final_result);
                }
            }
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
    //todo create default value if not found
    //let map_obj = unsafe{Map::read(&obj)};
    let r = match map.get(&key).cloned() {
        Some(v) => (v, Some(true)),
        None => (Object::null(), Some(false)),
    };
    Ok(r)
}

fn index_set_map(mut left: Object, index: Object, value: Object) -> Result<(), Error> {
    let map = left.as_map_mut();
    //isert returns the old value
    // later for the gc
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
    let str = obj.as_str();
    if index < 0 {
        return Err(Error::IndexError("i: out of bounds".to_string()));
    }

    let i = index as usize + 1;
    if i >= str.len() - 1 {
        return Err(Error::IndexError("out of bounds".to_string()));
    }

    let ch = str.chars().nth(i).unwrap();
    let result = Object::string(ch.to_string());
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
