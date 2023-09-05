pub mod collections;
pub mod float;
pub mod function;
pub mod int;
pub mod r#ref;
pub mod rune;
pub mod string;
pub mod structure;

use crate::vm::gc::GC;
use crate::vm::object::collections::{Array, Map, ObjIter};
use crate::vm::object::float::{Float, Float32, Float64};
use crate::vm::object::function::Closure;
use crate::vm::object::int::{
    Byte, Complex128, Complex64, Int, Int16, Int32, Int64, Int8, Uint, Uint16, Uint32, Uint64,
    Uint8,
};
use crate::vm::object::r#ref::Ref;
use crate::vm::object::rune::Rune;
use crate::vm::object::structure::Struct;
use crate::vm::Error;
use std::alloc::{alloc, handle_alloc_error, Layout};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::{Display, Write};
use std::ops::Shl;
use std::string::String as RString;
use string::String;

/// A macro for initialising a struct field (without dropping the original default value)
macro_rules! init {
    ($field: expr => $value: expr) => {
        unsafe {
            std::ptr::addr_of_mut!($field).write($value);
        }
    };
}

/// The mask to apply to get just the pointer address from a pointer object
const PTR_MASK: usize = (1 << NUM_BITS) - 1;

/// The amount of bits to shift-left the actual value in value objects (last 6 bits store the type tag)
const VALUE_SHIFT_BITS: usize = 5;

const NUM_BITS: usize = 64 - VALUE_SHIFT_BITS;

#[allow(unused)]
/// The max integer value we can store in a value object
const MAX_INT: isize = isize::MAX;

#[allow(unused)]
/// The minimum integer value we can store in a value object
const MIN_INT: isize = isize::MIN;

// ARM uses 49 bits and x86-64 uses 48 bits
// we have at least 15 bits to work with
// this is 6 bits and it supports up to 64 variants
#[derive(Debug, PartialEq, Copy, Clone, PartialOrd, Ord, Eq)]
#[repr(u8)]
pub enum Type {
    // The types below are all stored directly inside the pointer
    Null = 0b00000,
    Bool,
    Function,

    // The types below are all heap-allocated
    Int,
    Byte,
    I8,
    I16,
    I32,
    I64,
    UI,
    UI8,
    UI16,
    UI32,
    UI64,
    Float,
    Float32,
    Float64,
    Complex64,
    Complex128,
    String,
    Rune,
    Array,
    Map,
    Iter,
    Struct,
    Ref,
    Closure,
}

impl Type {
    pub fn is_numeric(&self) -> bool {
        match &self {
            Self::Int
            | Self::Byte
            | Self::I8
            | Self::I16
            | Self::I32
            | Self::I64
            | Self::UI
            | Self::UI8
            | Self::UI16
            | Self::UI32
            | Self::UI64
            | Self::Float
            | Self::Float32
            | Self::Float64 => true,
            _ => false,
        }
    }
}

impl TryFrom<&str> for Type {
    type Error = RString;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Ok(match value {
            "nil" => Self::Null,
            "int" => Self::Int,
            "bool" => Self::Bool,
            "func" => Self::Function,
            "float" => Self::Float,
            "string" => Self::String,
            _ => return Err(value.to_string()),
        })
    }
}

// Object is a wrapper over raw pointers so we can tag them with immediate values (null, bool, int)
#[derive(Copy, Clone, Eq)]
pub struct Object(*mut u8);
unsafe impl Sync for Object {}
unsafe impl Send for Object {}

impl Object {
    /// Creates a new object from the value (or address) given with the given type mask applied
    #[inline(always)]
    fn with_type(raw: *mut u8, t: Type) -> Self {
        let shift = (t as usize).shl(NUM_BITS);
        let s = Self((shift | raw as usize) as _);
        assert_eq!(s.tag(), t);
        s
    }

    /// Returns the type of this object pointer
    #[inline(always)]
    pub fn tag(self) -> Type {
        // Safety: self.0 with TAG_MASK applied will always yield a correct Type
        unsafe { std::mem::transmute((self.0 as usize >> NUM_BITS) as u8) }
    }

    /// Create a new null value
    #[inline(always)]
    pub fn null() -> Self {
        Self::with_type(0 as _, Type::Null)
    }

    /// Create a new boolean value
    #[inline(always)]
    pub fn bool(value: bool) -> Self {
        match value {
            true => Self::with_type((1 << VALUE_SHIFT_BITS) as _, Type::Bool),
            false => Self::with_type(0 as _, Type::Bool),
        }
    }

    /// Create a new integer value
    #[inline(always)]
    pub fn int(value: isize) -> Self {
        Int::from_isize(value)
    }

    #[inline(always)]
    pub fn int8(value: i8) -> Self {
        Int8::from_i8(value)
    }

    #[inline(always)]
    pub fn int16(value: i16) -> Self {
        Int16::from_i16(value)
    }

    #[inline(always)]
    pub fn int32(value: i32) -> Self {
        Int32::from_i32(value)
    }

    #[inline(always)]
    pub fn int64(value: i64) -> Self {
        Int64::from_i64(value)
    }

    #[inline(always)]
    pub fn byte(value: u8) -> Self {
        Byte::from_u8(value)
    }

    #[inline(always)]
    pub fn uint8(value: u8) -> Self {
        Uint8::from_u8(value)
    }

    #[inline(always)]
    pub fn uint16(value: u16) -> Self {
        Uint16::from_u16(value)
    }

    #[inline(always)]
    pub fn uint32(value: u32) -> Self {
        Uint32::from_u32(value)
    }

    #[inline(always)]
    pub fn uint64(value: u64) -> Self {
        Uint64::from_u64(value)
    }

    #[inline(always)]
    pub fn uint(value: usize) -> Self {
        Uint::from_usize(value)
    }

    #[inline(always)]
    pub fn complex64(value: usize) -> Self {
        Uint::from_usize(value)
    }

    /// Create a new function value
    pub fn function(ip: u32, num_locals: u16) -> Self {
        let value = ((ip as isize) << 16) | num_locals as isize;
        Self::with_type((value << VALUE_SHIFT_BITS) as _, Type::Function)
    }

    #[inline]
    pub fn float(value: f64, gc: &mut GC) -> Self {
        let ptr = Float::from_f64(value);
        gc.trace(ptr);
        ptr
    }

    #[inline]
    pub fn float32(value: f32, gc: &mut GC) -> Self {
        let ptr = Float32::from_f32(value);
        gc.trace(ptr);
        ptr
    }

    #[inline]
    pub fn float64(value: f64, gc: &mut GC) -> Self {
        let ptr = Float64::from_f64(value);
        gc.trace(ptr);
        ptr
    }

    #[inline]
    pub fn ref_t(value: Object, gc: &mut GC) -> Self {
        let ptr = Ref::from_obj(value);
        gc.trace(ptr);
        ptr
    }

    /// Returns the boolean value of this object pointer
    /// Note that is up to the caller to ensure this pointer is of the correct type
    #[inline(always)]
    pub fn as_bool(self) -> bool {
        (self.0 as u8 >> VALUE_SHIFT_BITS) != 0
    }

    /// Returns the integer value of this object pointer
    /// Note that is up to the caller to ensure this pointer is of the correct type
    #[inline(always)]
    pub fn as_int(&self) -> &Int {
        assert_eq!(Type::Int, self.tag());
        unsafe { Int::read_mut(&self) }
    }

    #[inline(always)]
    pub fn as_int8(&self) -> &Int8 {
        assert_eq!(Type::I8, self.tag());
        unsafe { Int8::read_mut(&self) }
    }

    #[inline(always)]
    pub fn as_int16(&self) -> &Int16 {
        assert_eq!(Type::I16, self.tag());
        unsafe { Int16::read_mut(&self) }
    }

    #[inline(always)]
    pub fn as_int32(&self) -> &Int32 {
        assert_eq!(Type::I32, self.tag());
        unsafe { Int32::read_mut(&self) }
    }

    #[inline(always)]
    pub fn as_int64(&self) -> &Int64 {
        assert_eq!(Type::I64, self.tag());
        unsafe { Int64::read_mut(&self) }
    }

    #[inline(always)]
    pub fn as_byte(&self) -> &Byte {
        assert_eq!(Type::Byte, self.tag());
        unsafe { Byte::read_mut(&self) }
    }

    #[inline(always)]
    pub fn as_uint(&self) -> &Uint {
        assert_eq!(Type::UI, self.tag());
        unsafe { Uint::read_mut(&self) }
    }

    #[inline(always)]
    pub fn as_uint8(&self) -> &Uint8 {
        assert_eq!(Type::UI8, self.tag());
        unsafe { Uint8::read_mut(&self) }
    }

    #[inline(always)]
    pub fn as_uint16(&self) -> &Uint16 {
        assert_eq!(Type::UI16, self.tag());
        unsafe { Uint16::read_mut(&self) }
    }

    #[inline(always)]
    pub fn as_uint32(&self) -> &Uint32 {
        assert_eq!(Type::UI32, self.tag());
        unsafe { Uint32::read_mut(&self) }
    }

    #[inline(always)]
    pub fn as_uint64(&self) -> &Uint64 {
        assert_eq!(Type::UI64, self.tag());
        unsafe { Uint64::read_mut(&self) }
    }

    #[inline(always)]
    pub fn as_int_mut(&self) -> &mut Int {
        assert_eq!(Type::Int, self.tag());
        unsafe { Int::read_mut(&self) }
    }

    #[inline(always)]
    pub fn as_isize(&self) -> isize {
        assert_eq!(Type::Int, self.tag());
        unsafe { Int::read_val(&self) }
    }

    /// Returns the function value of this object
    /// Note that is up to the caller to ensure this pointer is of the correct type
    #[inline(always)]
    pub fn as_function(self) -> [u32; 2] {
        let value = self.0 as isize >> VALUE_SHIFT_BITS;

        // lower 16-bits store the number of locals
        let num_locals = (value & 0xFFFF) as u32;

        // next 32 bits stores the IP
        let ip = (value >> 16) as u32;

        // that leaves 64-32-16-3=13 bits unused
        [ip, num_locals]
    }

    #[inline]
    pub fn as_ref(&self) -> &Ref {
        assert_eq!(self.tag(), Type::Ref);
        unsafe { self.get::<Ref>() }
    }

    #[inline]
    pub fn as_ref_mut(&self) -> &mut Ref {
        assert_eq!(self.tag(), Type::Ref);
        unsafe { self.get_mut::<Ref>() }
    }

    /// Returns the f64 value of this object pointer
    ///
    /// # Safety
    ///
    /// The caller should ensure this pointer points to an actual Float type
    #[inline]
    pub unsafe fn as_float(self) -> f64 {
        Float::read(&self)
    }

    pub unsafe fn as_float32(self) -> f32 {
        Float32::read(&self)
    }

    pub fn as_float64(self) -> f64 {
        unsafe { Float64::read(&self) }
    }

    pub fn as_rune(&self) -> &Rune {
        unsafe { Rune::read(&self) }
    }

    pub fn as_complex64(&self) -> &Complex64 {
        unsafe { Complex64::read(&self) }
    }

    pub fn as_complex128(&self) -> &Complex128 {
        unsafe { Complex128::read(&self) }
    }

    /// Returns the &str value of this object pointer
    /// Panics if object does not point to a String
    #[inline]
    pub fn as_str(&self) -> &str {
        assert_eq!(self.tag(), Type::String);
        unsafe { self.as_str_unchecked() }
    }

    #[inline]
    pub fn as_string_mut(&mut self) -> &mut RString {
        assert_eq!(self.tag(), Type::String);
        unsafe { &mut self.get_mut::<String>().value }
    }

    /// Returns the &str value of this object pointer
    ///
    /// # Safety
    ///
    /// The caller should ensure this pointer points to an actual String type
    #[inline]
    pub unsafe fn as_str_unchecked(&self) -> &str {
        self.get::<String>().value.as_str()
    }

    /// Returns a reference to the Vec<Object> value this pointer points to
    /// Panics if object does not point to an Array
    #[inline]
    pub fn as_vec(&self) -> &Vec<Object> {
        assert_eq!(self.tag(), Type::Array);
        unsafe { self.as_vec_unchecked() }
    }

    #[inline]
    pub fn as_iter(&mut self) -> &mut ObjIter {
        assert_eq!(self.tag(), Type::Iter);
        unsafe { ObjIter::read(self) }
    }

    #[inline]
    pub fn as_struct(&self) -> &Struct {
        assert_eq!(self.tag(), Type::Struct);
        unsafe { Struct::read(self) }
    }

    #[inline]
    pub fn as_struct_mut(&mut self) -> &mut Struct {
        assert_eq!(self.tag(), Type::Struct);
        unsafe { Struct::read_mut(self) }
    }

    #[inline]
    pub fn as_closure(&mut self) -> &Closure {
        assert_eq!(self.tag(), Type::Closure);
        unsafe { Closure::read(self) }
    }

    #[inline]
    pub fn as_closure_mut(&mut self) -> &mut Closure {
        assert_eq!(self.tag(), Type::Closure);
        unsafe { Closure::read_mut(self) }
    }

    /// Returns a reference to the Vec<Object> value this pointer points to
    ///
    /// # Safety
    ///
    /// The caller should ensure this pointer actually points to an Array
    #[inline]
    pub unsafe fn as_vec_unchecked(&self) -> &Vec<Object> {
        Array::read(self)
    }

    /// Returns a mutable reference to the Vec<Object> value this pointer points to
    /// Panics if object does not point to an Array
    #[inline]
    pub fn as_vec_mut(&mut self) -> &mut Vec<Object> {
        assert_eq!(self.tag(), Type::Array);
        unsafe { self.as_vec_unchecked_mut() }
    }

    /// Returns a mutable reference to the Vec<Object> value this pointer points to
    /// Panics if object does not point to an Array
    #[inline]
    pub fn as_map_mut(&mut self) -> &mut BTreeMap<Object, Object> {
        assert_eq!(self.tag(), Type::Map);
        unsafe { &mut self.get_mut::<Map>().value }
    }

    /// Returns a mutable reference to the Vec<Object> value this pointer points to
    /// Panics if object does not point to an Array
    #[inline]
    pub fn as_map(&self) -> &BTreeMap<Object, Object> {
        assert_eq!(self.tag(), Type::Map);
        unsafe { &self.get_mut::<Map>().value }
    }

    /// Returns a mutable reference to the Vec<Object> value this pointer points to
    ///
    /// # Safety
    ///
    /// The caller should ensure this pointer actually points to an Array
    #[inline]
    pub unsafe fn as_vec_unchecked_mut(&mut self) -> &mut Vec<Object> {
        &mut self.get_mut::<Array>().value
    }

    /// Returns the pointer stored in this object
    /// This can return a non-valid address if called on a non-heap allocated object value.
    #[inline]
    pub(crate) fn as_ptr(self) -> *mut u8 {
        (self.0 as usize & PTR_MASK) as _
    }

    /// Get a reference to the value this object points to
    /// It is up to the caller to ensure the object is actually heap-allocated and points to a valid memory location.
    #[inline]
    unsafe fn get<'a, T>(self) -> &'a T {
        &*(self.as_ptr() as *const T)
    }

    /// Get a mutable reference to the value this object points to
    /// It is up to the caller to ensure the object is actually heap-allocated and points to a valid memory location.
    #[inline]
    pub(crate) unsafe fn get_mut<'a, T>(self) -> &'a mut T {
        &mut *(self.as_ptr() as *mut T)
    }

    /// Frees the memory address this pointer points to
    pub fn free(self) {
        unsafe {
            match self.tag() {
                Type::Float => Float::destroy(self),
                Type::String => String::destroy(self),
                Type::Array => Array::destroy(self),
                _ => (),
            }
        }
    }

    /// Frees the memory address this pointer points to
    /// Plus all addresses inside the array (if it is an array)
    pub fn free_recursive(self) {
        if self.tag() == Type::Array {
            // Safety: We've asserted the type
            unsafe {
                for o in self.as_vec_unchecked() {
                    o.free();
                }
            }
        }

        self.free();
    }
}

pub trait FromString<T> {
    fn string(value: T, gc: &mut GC) -> Self;
}

impl FromString<RString> for Object {
    fn string(value: RString, gc: &mut GC) -> Self {
        let ptr = String::from_string(value);
        gc.trace(ptr);
        ptr
    }
}

impl FromString<&str> for Object {
    fn string(value: &str, gc: &mut GC) -> Self {
        let ptr = String::from_string(value.to_string());
        gc.trace(ptr);
        ptr
    }
}

pub trait FromVec<T> {
    fn array(value: T, gc: &mut GC) -> Self;
}

impl FromVec<Vec<Object>> for Object {
    /// Create a new (garbage-collected) Array value
    fn array(value: Vec<Object>, gc: &mut GC) -> Self {
        let ptr = Array::from_vec(value);
        gc.trace(ptr);
        ptr
    }
}

impl FromVec<&[Object]> for Object {
    /// Create a new (garbage-collected) Array value
    fn array(value: &[Object], gc: &mut GC) -> Self {
        let ptr = Array::from_slice(value);
        gc.trace(ptr);
        ptr
    }
}

impl PartialEq for Object {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        match (self.tag(), other.tag()) {
            (Type::Ref, Type::Null) => {
                let reference = self.as_ref();
                return reference.value.tag() == Type::Null;
            }
            (Type::Null, Type::Ref) => {
                let reference = other.as_ref();
                return reference.value.tag() == Type::Null;
            }
            _ => {
                if self.tag() != other.tag() {
                    return false;
                }
            }
        }
        //panic!("{:#?}=={:#?}", self.tag(), other.tag());
        // TODO: Maybe delay type check (on other object) to here
        //  (and then only for heap-allocated objects)
        match self.tag() {
            Type::Null => true,
            Type::Function => self.0 == other.0,
            Type::Bool => {
                let l = self.as_bool();
                let r = self.as_bool();
                l == r
            }
            Type::Int => {
                let l = self.as_isize();
                let r = self.as_isize();
                l == r
            }
            Type::I8 => {
                let l = self.as_int8();
                let r = self.as_int8();
                l.value == r.value
            }
            Type::I16 => {
                let l = self.as_int16();
                let r = self.as_int16();
                l.value == r.value
            }
            Type::I32 => {
                let l = self.as_int32();
                let r = self.as_int32();
                l.value == r.value
            }
            Type::I64 => {
                let l = self.as_int64();
                let r = self.as_int64();
                l.value == r.value
            }
            Type::UI => {
                let l = self.as_uint();
                let r = self.as_uint();
                l.value == r.value
            }
            Type::UI8 => {
                let l = self.as_uint8();
                let r = self.as_uint8();
                l.value == r.value
            }
            Type::UI16 => {
                let l = self.as_uint16();
                let r = self.as_uint16();
                l.value == r.value
            }
            Type::UI32 => {
                let l = self.as_uint32();
                let r = self.as_uint32();
                l.value == r.value
            }
            Type::UI64 => {
                let l = self.as_uint64();
                let r = self.as_uint64();
                l.value == r.value
            }
            Type::Byte => {
                let l = self.as_byte();
                let r = self.as_byte();
                l.value == r.value
            }
            Type::Complex64 => {
                let l = self.as_complex64();
                let r = self.as_complex64();
                l.value == r.value
            }
            Type::Complex128 => {
                let l = self.as_complex128();
                let r = self.as_complex128();
                l.value == r.value
            }
            Type::Float32 => unsafe { self.as_float32() == other.as_float32() },
            Type::Float64 => unsafe { self.as_float64() == other.as_float64() },
            Type::Float => unsafe { self.as_float() == other.as_float() },
            Type::String => unsafe { self.as_str_unchecked() == other.as_str_unchecked() },
            //Type::Rune => unsafe{self.as_rune() == other.as_rune()},
            Type::Array
            | Type::Ref
            | Type::Map
            | Type::Iter
            | Type::Struct
            | Type::Rune
            | Type::Closure => {
                unimplemented!(
                    "Can not yet compare objects of type {} and {}",
                    self.tag(),
                    other.tag()
                )
            }
        }
    }
}

impl PartialOrd for Object {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // we assert this in the various wrapper functions, eg Object::lt
        debug_assert_eq!(self.tag(), other.tag());

        match self.tag() {
            Type::Null | Type::Bool => self.0.partial_cmp(&other.0),
            Type::Int => unsafe { self.as_int().value.partial_cmp(&other.as_int().value) },
            Type::I8 => unsafe { self.as_int8().value.partial_cmp(&other.as_int8().value) },
            Type::I16 => unsafe { self.as_int16().value.partial_cmp(&other.as_int16().value) },
            Type::I32 => unsafe { self.as_int32().value.partial_cmp(&other.as_int32().value) },
            Type::I64 => unsafe { self.as_int64().value.partial_cmp(&other.as_int64().value) },
            Type::Byte => unsafe { self.as_byte().value.partial_cmp(&other.as_byte().value) },
            Type::UI => unsafe { self.as_uint().value.partial_cmp(&other.as_uint().value) },
            Type::UI8 => unsafe { self.as_uint8().value.partial_cmp(&other.as_uint8().value) },
            Type::UI16 => unsafe { self.as_uint16().value.partial_cmp(&other.as_uint16().value) },
            Type::UI32 => unsafe { self.as_uint32().value.partial_cmp(&other.as_uint32().value) },
            Type::UI64 => unsafe { self.as_uint64().value.partial_cmp(&other.as_uint64().value) },
            Type::Float => unsafe { self.as_float().partial_cmp(&other.as_float()) },
            Type::Float64 => unsafe { self.as_float64().partial_cmp(&other.as_float64()) },
            Type::Float32 => unsafe { self.as_float32().partial_cmp(&other.as_float32()) },
            Type::String => unsafe { self.as_str_unchecked().partial_cmp(other.as_str()) },
            Type::Complex64 | Type::Complex128 => {
                unimplemented!()
            }
            Type::Rune => self.as_rune().value.partial_cmp(&other.as_rune().value),
            Type::Array
            | Type::Function
            | Type::Ref
            | Type::Map
            | Type::Iter
            | Type::Struct
            | Type::Closure => {
                unimplemented!("cannot compare {}", self.tag())
            }
        }
    }
}

impl Ord for Object {
    fn cmp(&self, other: &Self) -> Ordering {
        debug_assert_eq!(self.tag(), other.tag());

        let ord = self.partial_cmp(other).unwrap_or(Ordering::Less);
        ord
    }
}

macro_rules! impl_arith {
    ($func_name:ident, $op:tt) => {
        #[inline(always)]
        pub(crate) fn $func_name(self, rhs: Self, gc: &mut GC) -> Result<Object, Error> {
            if self.tag() != rhs.tag() {
                return Err(Error::TypeError(format!("invalid op {} for types ({} and {})", stringify!($op), self.tag(), rhs.tag())))
            }

            let result = match self.tag() {
                Type::Int => Object::int(self.as_isize() $op rhs.as_isize()),
                Type::I8 => Object::int8(self.as_int8().value $op rhs.as_int8().value),
                Type::I16 => Object::int16(self.as_int16().value $op rhs.as_int16().value),
                Type::I32 => Object::int32(self.as_int32().value $op rhs.as_int32().value),
                Type::I64 => Object::int64(self.as_int64().value $op rhs.as_int64().value),
                Type::UI8 => Object::uint8(self.as_uint8().value $op rhs.as_uint8().value),
                Type::UI => Object::uint(self.as_uint().value $op rhs.as_uint().value),
                Type::UI8 => Object::uint8(self.as_uint8().value $op rhs.as_uint8().value),
                Type::UI16 => Object::uint16(self.as_uint16().value $op rhs.as_uint16().value),
                Type::UI32 => Object::uint32(self.as_uint32().value $op rhs.as_uint32().value),
                Type::UI64 => Object::uint64(self.as_uint64().value $op rhs.as_uint64().value),

                // Safety: We've already asserted the object type
                Type::Float => unsafe {
                    Object::float(self.as_float() $op rhs.as_float(), gc)
                }
                Type::Float64 => unsafe {
                    Object::float64(self.as_float64() $op rhs.as_float64(), gc)
                }
                Type::Float32 => unsafe {
                    Object::float32(self.as_float32() $op rhs.as_float32(), gc)
                }
                _ => return Err(Error::TypeError(format!("unsupported op {} for type {}", stringify!($op), self.tag()))),
            };

            Ok(result)
        }
    };
}

macro_rules! impl_logical {
    ($func_name:ident, $op:tt) => {
        #[inline(always)]
        pub fn $func_name(self, rhs: Self, _gc: &mut GC) -> Result<Object, Error> {
            let result = match (self.tag(), rhs.tag()) {
                (Type::Bool, Type::Bool) => Object::bool(self.as_bool() $op rhs.as_bool()),
                _ => return Err(Error::TypeError(format!("invalid op {} for types {} and {}", stringify!($op), self.tag(), rhs.tag())))
            };
            Ok(result)
        }
    };
}

macro_rules! impl_cmp {
    ($func_name:ident, $op:tt) => {
        #[inline(always)]
        pub fn $func_name(self, rhs: Self, _gc: &mut GC) -> Result<Object, Error> {
            match (self.tag(), rhs.tag()) {
                (Type::Ref, Type::Null) | (Type::Null, Type::Ref) => {}
                _ => {
                    if self.tag() != rhs.tag() {
                        return Err(Error::TypeError(format!("invalid types {} and {}", self.tag(), rhs.tag())));
                    }
                }
            }

            // Delegate actual comparison to PartialOrd/PartialEq implementation
            Ok(Object::bool(self $op rhs,))
        }
    };
}

impl Object {
    impl_arith!(add, +);
    impl_arith!(sub, -);
    impl_arith!(mul, *);
    impl_arith!(div, /);
    impl_arith!(rem, %);

    impl_cmp!(gt, >);
    impl_cmp!(gte, >=);
    impl_cmp!(lt, <);
    impl_cmp!(lte, <=);
    impl_cmp!(eq, ==);
    impl_cmp!(neq, !=);

    impl_logical!(and, &&);
    impl_logical!(or, ||);
}

#[repr(C)]
pub(crate) struct Header {
    pub(crate) marked: bool,
}

impl Header {
    #[inline]
    pub unsafe fn read(obj: &mut Object) -> &mut Header {
        obj.get_mut::<Self>()
    }
}

impl Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.tag() {
            Type::Null => f.write_str("nil")?,
            Type::Bool => f.write_str(if self.as_bool() { "true" } else { "false" })?,
            Type::Float => unsafe { f.write_str(&self.as_float().to_string())? },
            Type::Float32 => unsafe { f.write_str(&self.as_float32().to_string())? },
            Type::Float64 => unsafe { f.write_str(&self.as_float64().to_string())? },
            Type::Int => f.write_str(&self.as_int().value.to_string())?,
            Type::I8 => f.write_str(&self.as_int8().value.to_string())?,
            Type::I16 => f.write_str(&self.as_int16().value.to_string())?,
            Type::I32 => f.write_str(&self.as_int32().value.to_string())?,
            Type::I64 => f.write_str(&self.as_int64().value.to_string())?,
            Type::UI => f.write_str(&self.as_uint().value.to_string())?,
            Type::UI8 => f.write_str(&self.as_uint8().value.to_string())?,
            Type::UI16 => f.write_str(&self.as_uint16().value.to_string())?,
            Type::UI32 => f.write_str(&self.as_uint32().value.to_string())?,
            Type::UI64 => f.write_str(&self.as_uint64().value.to_string())?,
            Type::Byte => f.write_str(&self.as_byte().value.to_string())?,
            Type::String => unsafe { f.write_str(self.as_str_unchecked())? },
            Type::Complex64 => f.write_str(&self.as_complex64().value.to_string())?,
            Type::Complex128 => f.write_str(&self.as_complex128().value.to_string())?,
            Type::Array => {
                let values = unsafe { self.as_vec_unchecked() };
                f.write_char('[')?;
                for (i, obj) in values.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    std::fmt::Display::fmt(&obj, f)?;
                }
                f.write_char(']')?;
            }
            Type::Struct => {
                let strct = unsafe { self.as_struct() };

                f.write_str(strct.name.as_str())?;
                f.write_char('{')?;
                for (i, obj) in strct.values.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    std::fmt::Display::fmt(&obj, f)?;
                }
                f.write_char('}')?;
            }
            Type::Rune => {
                f.write_str(&format!("rune({})", self.as_rune().value.to_string()))?;
            }
            Type::Map => {
                let strct = unsafe { self.as_map() };

                f.write_char('{')?;
                for (i, obj) in strct.iter() {
                    std::fmt::Display::fmt(&i, f)?;
                    f.write_str(":")?;
                    std::fmt::Display::fmt(&obj, f)?;
                }
                f.write_char('}')?;
            }
            Type::Closure => {
                f.write_str("func(){}").expect("");
            }
            Type::Iter => {
                f.write_str("iter<k, v>").expect("");
            }
            Type::Function => f.write_str("func")?,
            Type::Ref => {
                f.write_char('&')?;
                let hex = format!("{:p}", self.0);
                f.write_str("{addr:")?;
                std::fmt::Display::fmt(&hex, f)?;
                f.write_str(", value: ")?;
                std::fmt::Display::fmt(&self.as_ref().value, f)?;
                f.write_str("}")?;
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            Type::Null => "nil",
            Type::Bool => "bool",
            Type::Float => "float",
            Type::Float32 => "float32",
            Type::Float64 => "float64",
            Type::Byte => "byte",
            Type::Int => "int",
            Type::UI => "uint",
            Type::I8 => "int8",
            Type::I16 => "int16",
            Type::I32 => "int32",
            Type::I64 => "int64",
            Type::UI8 => "uint8",
            Type::UI16 => "uint16",
            Type::UI32 => "uint32",
            Type::UI64 => "uint64",
            Type::String => "string",
            Type::Array => "array",
            Type::Function => "func",
            Type::Map => "map",
            Type::Iter => "iter",
            Type::Ref => "&",
            Type::Struct => "struct",
            Type::Rune => "rune",
            Type::Closure => "closure",
            Type::Complex64 => "complex64",
            Type::Complex128 => "complex128",
        };
        f.write_str(str)
    }
}

/// Allocate a chunk of memory with the given layout
#[inline]
fn allocate(layout: Layout) -> *mut u8 {
    // Safety: we only call this function for types with a non-zero layout
    let ptr = unsafe { alloc(layout) };

    if ptr.is_null() {
        handle_alloc_error(layout);
    } else {
        ptr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_object_function() {
        assert_eq!(Object::function(1, 1).tag(), Type::Function);
        assert_eq!(Object::function(1, 1).as_function(), [1, 1]);
        assert_eq!(
            Object::function(1, 0xFFFF_u16).as_function(),
            [1, 0xFFFF_u32]
        );
        assert_eq!(
            Object::function(0xFFFFFFFF, 1).as_function(),
            [0xFFFFFFFF, 1]
        );
    }

    #[test]
    fn test_object_null() {
        assert_eq!(Object::null().tag(), Type::Null);
    }

    #[test]
    fn test_object_bool() {
        let t = Object::bool(true);
        let f = Object::bool(false);

        assert_eq!(t.tag(), Type::Bool);
        assert_eq!(f.tag(), Type::Bool);

        assert_eq!(t.as_bool(), true);
        assert_eq!(f.as_bool(), false);
    }

    #[test]
    fn test_object_int() {
        let obj = Object::int(1);
        assert_eq!(obj.tag(), Type::Int);
        assert_eq!(obj.as_isize(), 1);

        let obj = Object::int(-1);
        assert_eq!(obj.tag(), Type::Int);
        assert_eq!(obj.as_isize(), -1);

        let obj = Object::int(MAX_INT);
        assert_eq!(obj.tag(), Type::Int);
        assert_eq!(obj.as_isize(), MAX_INT);

        let obj = Object::int(MIN_INT);
        assert_eq!(obj.tag(), Type::Int);
        assert_eq!(obj.as_isize(), MIN_INT);
    }

    // #[test]
    // #[should_panic]
    // fn test_object_int_overflow() {
    //     assert_eq!(Object::int(MAX_INT + 1).tag(), Type::Int);
    //     assert_eq!(Object::int(MAX_INT + 1).as_isize(), MAX_INT + 1);
    //
    //     assert_eq!(Object::int(MIN_INT - 1).tag(), Type::Int);
    //     assert_eq!(Object::int(MIN_INT - 1).as_isize(), MIN_INT - 1);
    // }

    #[test]
    fn test_object_string() {
        let mut gc = GC::new();
        let obj = Object::string("Hello, world!", &mut gc);
        assert_eq!(obj.tag(), Type::String);
        assert_eq!(obj.as_str(), "Hello, world!");
    }

    #[test]
    fn test_pointer_float() {
        let mut gc = GC::new();
        let obj = Object::float(std::f64::consts::PI, &mut gc);
        assert_eq!(obj.tag(), Type::Float);
        assert_eq!(obj.as_float64(), std::f64::consts::PI);
    }

    #[test]
    fn test_pointer_empty_array() {
        let ptr = Array::from_slice(&[]);
        assert_eq!(ptr.tag(), Type::Array);
        assert_eq!(ptr.as_vec().len(), 0);
        ptr.free();
    }

    #[test]
    fn test_pointer_array() {
        let ptr = Array::from_slice(&[Object::null()]);
        assert_eq!(ptr.tag(), Type::Array);
        assert_eq!(ptr.as_vec().len(), 1);
        assert_eq!(ptr.as_vec().get(0), Some(&Object::null()));
        ptr.free();
    }

    #[test]
    fn test_iter() {
        let mut ptr = ObjIter::from_vec(vec![Object::int(1)]);
        assert_eq!(ptr.tag(), Type::Iter);
        let iter = ptr.as_iter();
        assert_eq!((Object::int(0), Object::int(1)), iter.next());
    }

    #[test]
    fn test_ord() {
        let mut gc = GC::new();
        let left = Object::string("foo", &mut gc);
        let right = Object::string("foo", &mut gc);
        assert_eq!(left.cmp(&right), Ordering::Equal)
    }
    #[test]
    fn test_ref() {
        let mut gc = GC::new();
        let left = Object::ref_t(Object::int(5), &mut gc);
        let right = left;
        right.as_ref_mut().value = Object::int(6);
        println!("{:#?}--{:#?}", left, right);
    }
    // TODO: Test PartialEq & PartialOrd implementations
}
