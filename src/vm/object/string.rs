use crate::vm::object::{allocate, Object, Type};
use std::alloc::Layout;
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
    pub(crate) value: RString,
}

impl String {
    pub(crate) fn from_string(value: RString) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::String);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value);
        ptr
    }
}
