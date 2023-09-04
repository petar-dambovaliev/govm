use crate::vm::object::{allocate, Header, Object, Type};
use std::alloc::{dealloc, Layout};
use std::ptr::drop_in_place;

macro_rules! init {
    ($field: expr => $value: expr) => {
        unsafe {
            std::ptr::addr_of_mut!($field).write($value);
        }
    };
}

#[repr(C)]
struct Rune {
    header: Header,
    value: char,
}

impl Rune {
    unsafe fn destroy(ptr: Object) {
        drop_in_place(ptr.as_ptr() as *mut Self);
        dealloc(ptr.as_ptr(), Layout::new::<Self>());
    }

    fn from_string(value: char) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Rune);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value);
        ptr
    }
}
