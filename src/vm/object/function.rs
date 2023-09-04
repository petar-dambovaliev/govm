use crate::vm::object::{allocate, Header, Object, Type};
use std::alloc::Layout;

#[repr(C)]
pub struct Closure {
    header: Header,
    pub ip: u32,
    pub num_locals: u16,
}

impl Closure {
    pub unsafe fn read(ptr: &Object) -> &Self {
        ptr.get::<Self>()
    }

    pub unsafe fn read_mut(ptr: &Object) -> &mut Self {
        ptr.get_mut::<Self>()
    }

    pub fn object(ip: u32, num_locals: u16) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Closure);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        obj.ip = ip;
        obj.num_locals = num_locals;

        ptr
    }
}
