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
pub struct Float {
    header: Header,
    value: f64,
}

impl Float {
    #[inline]
    pub(crate) unsafe fn read(obj: &Object) -> f64 {
        obj.get::<Self>().value
    }

    #[inline]
    pub(crate) unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_f64(value: f64) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Float);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Float32 {
    header: Header,
    value: f32,
}

impl Float32 {
    #[inline]
    unsafe fn read(obj: &Object) -> f32 {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_f32(value: f32) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Float32);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Float64 {
    header: Header,
    value: f64,
}

impl Float64 {
    #[inline]
    unsafe fn read(obj: &Object) -> f64 {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_f64(value: f64) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Float32);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}
