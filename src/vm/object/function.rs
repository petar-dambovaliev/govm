use crate::vm::object::{allocate, Object, Type};
use std::alloc::Layout;

macro_rules! init {
    ($field: expr => $value: expr) => {
        unsafe {
            std::ptr::addr_of_mut!($field).write($value);
        }
    };
}

#[repr(C)]
pub struct Closure {
    pub ip: u32,
    pub num_locals: u16,
    pub captured: Vec<Object>,
    pub is_null: bool,
}

impl Closure {
    pub unsafe fn read(ptr: &Object) -> &Self {
        ptr.get::<Self>()
    }

    pub unsafe fn read_mut(ptr: &Object) -> &mut Self {
        ptr.get_mut::<Self>()
    }

    pub fn null() -> Object {
        let ptr = Self::object(0, 0, vec![]);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.is_null = true;
        ptr
    }

    pub fn object(ip: u32, num_locals: u16, captured: Vec<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Closure);
        let obj = unsafe { ptr.get_mut::<Self>() };

        obj.ip = ip;
        obj.num_locals = num_locals;
        obj.is_null = false;
        init!(obj.captured => captured);

        ptr
    }
}
