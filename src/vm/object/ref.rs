use crate::vm::object::{allocate, Header, Object, Type};
use std::alloc::Layout;

macro_rules! init {
    ($field: expr => $value: expr) => {
        unsafe {
            std::ptr::addr_of_mut!($field).write($value);
        }
    };
}

#[repr(C)]
pub struct Ref {
    header: Header,
    pub value: Object,
}

impl Ref {
    pub(crate) fn from_obj(value: Object) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Ref);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        //obj.value = value;
        init!(obj.value => value );
        ptr
    }
}
