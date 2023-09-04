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
pub struct Struct {
    header: Header,
    pub(crate) name: String,
    pub values: Vec<Object>,
}

impl Struct {
    pub(crate) unsafe fn read(ptr: &Object) -> &Self {
        ptr.get::<Self>()
    }

    pub(crate) unsafe fn read_mut(ptr: &Object) -> &mut Self {
        ptr.get_mut::<Self>()
    }

    pub fn object(name: String, values: Vec<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Struct);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.values => values);
        init!(obj.name => name);
        ptr
    }
}
