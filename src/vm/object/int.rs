#![allow(dead_code)]
#![allow(unsafe_op_in_unsafe_fn)]

use crate::vm::object::{allocate, Object, Type};
use num::Complex;
use std::alloc::Layout;

/// A macro for initialising a struct field (without dropping the original default value)
macro_rules! init {
    ($field: expr => $value: expr) => {
        unsafe {
            std::ptr::addr_of_mut!($field).write($value);
        }
    };
}

#[repr(C)]
pub struct Complex64 {
    pub(crate) value: Complex<f32>,
}

impl Complex64 {
    #[inline]
    pub(crate) unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[allow(unused)]
    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[allow(unused)]
    #[inline]
    pub(crate) unsafe fn read_val(obj: &Object) -> Complex<f32> {
        obj.get::<Self>().value
    }

    #[allow(unused)]
    pub(crate) fn from_isize(value: Complex<f32>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Complex64);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Complex128 {
    pub(crate) value: Complex<f64>,
}

impl Complex128 {
    #[inline]
    pub(crate) unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[allow(unused)]
    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[allow(unused)]
    #[inline]
    pub(crate) unsafe fn read_val(obj: &Object) -> Complex<f64> {
        obj.get::<Self>().value
    }

    #[allow(unused)]
    pub(crate) fn from_isize(value: Complex<f64>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Complex128);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Int {
    pub(crate) value: isize,
}

impl Int {
    #[allow(unused)]
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_val(obj: &Object) -> isize {
        obj.get::<Self>().value
    }

    pub(crate) fn from_isize(value: isize) -> Object {
        let raw = allocate(Layout::new::<Self>());
        let ptr = Object::with_type(raw, Type::Int);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Int8 {
    pub(crate) value: i8,
}

impl Int8 {
    #[allow(unused)]
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[allow(unused)]
    #[inline]
    unsafe fn read_val(obj: &Object) -> i8 {
        obj.get::<Self>().value
    }

    pub(crate) fn from_i8(value: i8) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::I8);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Int16 {
    pub(crate) value: i16,
}

impl Int16 {
    #[allow(unused)]
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[allow(unused)]
    #[inline]
    unsafe fn read_val(obj: &Object) -> i16 {
        obj.get::<Self>().value
    }
    pub(crate) fn from_i16(value: i16) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::I16);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Int32 {
    pub(crate) value: i32,
}

impl Int32 {
    #[allow(unused)]
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[allow(unused)]
    #[inline]
    unsafe fn read_val(obj: &Object) -> i32 {
        obj.get::<Self>().value
    }

    pub(crate) fn from_i32(value: i32) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::I32);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Int64 {
    pub(crate) value: i64,
}

impl Int64 {
    #[allow(unused)]
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[allow(unused)]
    #[inline]
    unsafe fn read_val(obj: &Object) -> i64 {
        obj.get::<Self>().value
    }

    pub(crate) fn from_i64(value: i64) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::I64);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Uint {
    pub(crate) value: usize,
}

impl Uint {
    #[allow(unused)]
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[allow(unused)]
    #[inline]
    unsafe fn read_val(obj: &Object) -> usize {
        obj.get::<Self>().value
    }
    pub(crate) fn from_usize(value: usize) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::UI);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Uint8 {
    pub(crate) value: u8,
}

impl Uint8 {
    #[allow(unused)]
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[allow(unused)]
    #[inline]
    unsafe fn read_val(obj: &Object) -> u8 {
        obj.get::<Self>().value
    }

    pub(crate) fn from_u8(value: u8) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::UI8);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Uint16 {
    pub(crate) value: u16,
}

impl Uint16 {
    #[allow(unused)]
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[allow(unused)]
    #[inline]
    unsafe fn read_val(obj: &Object) -> u16 {
        obj.get::<Self>().value
    }

    pub(crate) fn from_u16(value: u16) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::UI16);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Uint32 {
    pub(crate) value: u32,
}

impl Uint32 {
    #[allow(unused)]
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[allow(unused)]
    #[inline]
    unsafe fn read_val(obj: &Object) -> u32 {
        obj.get::<Self>().value
    }

    pub(crate) fn from_u32(value: u32) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::UI32);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Uint64 {
    pub(crate) value: u64,
}

impl Uint64 {
    #[allow(unused)]
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[allow(unused)]
    #[inline]
    unsafe fn read_val(obj: &Object) -> u64 {
        obj.get::<Self>().value
    }

    pub(crate) fn from_u64(value: u64) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::UI64);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Byte {
    pub(crate) value: u8,
}

impl Byte {
    #[allow(unused)]
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[allow(unused)]
    #[inline]
    unsafe fn read_val(obj: &Object) -> u8 {
        obj.get::<Self>().value
    }

    pub(crate) fn from_u8(value: u8) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Byte);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value );
        ptr
    }
}
