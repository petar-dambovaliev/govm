use crate::vm::object::{allocate, Header, Object, Type};
use std::alloc::{dealloc, Layout};
use std::ptr::drop_in_place;
use std::string::String as RString;

macro_rules! init {
    ($field: expr => $value: expr) => {
        unsafe {
            std::ptr::addr_of_mut!($field).write($value);
        }
    };
}

#[repr(C)]
pub(crate) struct String {
    header: Header,
    pub(crate) value: RString,
}

impl String {
    pub(crate) unsafe fn destroy(ptr: Object) {
        drop_in_place(ptr.as_ptr() as *mut Self);
        dealloc(ptr.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_string(value: RString) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::String);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value);
        ptr
    }
}
