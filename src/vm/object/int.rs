use crate::vm::object::{allocate, Header, Object, Type};
use std::alloc::{dealloc, Layout};
use std::ptr::drop_in_place;

/// A macro for initialising a struct field (without dropping the original default value)
macro_rules! init {
    ($field: expr => $value: expr) => {
        unsafe {
            std::ptr::addr_of_mut!($field).write($value);
        }
    };
}

#[repr(C)]
pub struct Int {
    header: Header,
    pub(crate) value: isize,
}

impl Int {
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

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_isize(value: isize) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Int);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Int8 {
    header: Header,
    pub(crate) value: i8,
}

impl Int8 {
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[inline]
    unsafe fn read_val(obj: &Object) -> i8 {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_i8(value: i8) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::I8);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Int16 {
    header: Header,
    pub(crate) value: i16,
}

impl Int16 {
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[inline]
    unsafe fn read_val(obj: &Object) -> i16 {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_i16(value: i16) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::I16);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Int32 {
    header: Header,
    pub(crate) value: i32,
}

impl Int32 {
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[inline]
    unsafe fn read_val(obj: &Object) -> i32 {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_i32(value: i32) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::I32);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Int64 {
    header: Header,
    pub(crate) value: i64,
}

impl Int64 {
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[inline]
    unsafe fn read_val(obj: &Object) -> i64 {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_i64(value: i64) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::I64);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Uint {
    header: Header,
    pub(crate) value: usize,
}

impl Uint {
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[inline]
    unsafe fn read_val(obj: &Object) -> usize {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_usize(value: usize) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::UI);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Uint8 {
    header: Header,
    pub(crate) value: u8,
}

impl Uint8 {
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[inline]
    unsafe fn read_val(obj: &Object) -> u8 {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_u8(value: u8) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::UI8);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Uint16 {
    header: Header,
    pub(crate) value: u16,
}

impl Uint16 {
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[inline]
    unsafe fn read_val(obj: &Object) -> u16 {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_u16(value: u16) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::UI16);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Uint32 {
    header: Header,
    pub(crate) value: u32,
}

impl Uint32 {
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[inline]
    unsafe fn read_val(obj: &Object) -> u32 {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_u32(value: u32) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::UI32);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Uint64 {
    header: Header,
    pub(crate) value: u64,
}

impl Uint64 {
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[inline]
    unsafe fn read_val(obj: &Object) -> u64 {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_u64(value: u64) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::UI64);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
pub struct Byte {
    header: Header,
    pub(crate) value: u8,
}

impl Byte {
    #[inline]
    unsafe fn read(obj: &Object) -> &Self {
        obj.get::<Self>()
    }

    #[inline]
    pub(crate) unsafe fn read_mut(obj: &Object) -> &mut Self {
        obj.get_mut::<Self>()
    }

    #[inline]
    unsafe fn read_val(obj: &Object) -> u8 {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_u8(value: u8) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Byte);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}
