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
pub struct Rune {
    pub value: char,
}

impl Rune {
    #[inline]
    pub(crate) unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    pub(crate) fn from_char(value: char) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Rune);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value);
        ptr
    }
}
