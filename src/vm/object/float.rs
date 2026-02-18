#![allow(dead_code)]
#![allow(unsafe_op_in_unsafe_fn)]

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
pub struct Float32 {
    value: f32,
}

impl Float32 {
    #[inline]
    pub(crate) unsafe fn read(obj: &Object) -> f32 {
        obj.get::<Self>().value
    }

    pub(crate) fn from_f32(value: f32) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Float32);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Float64 {
    value: f64,
}

impl Float64 {
    #[inline]
    pub(crate) unsafe fn read(obj: &Object) -> f64 {
        obj.get::<Self>().value
    }

    pub(crate) fn from_f64(value: f64) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Float64);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}
