use crate::vm::gc::GC;
use crate::vm::Error;
use std::alloc::{alloc, dealloc, handle_alloc_error, Layout};
use std::cmp::Ordering;
use std::collections::btree_map::IntoIter;
use std::collections::BTreeMap;
use std::fmt::{Display, Write};
use std::io::Write as IoWrite;
use std::ptr::drop_in_place;
use std::string::String as RString;

/// A macro for initialising a struct field (without dropping the original default value)
macro_rules! init {
    ($field: expr => $value: expr) => {
        unsafe {
            std::ptr::addr_of_mut!($field).write($value);
        }
    };
}

/// The mask to apply to get just the type (tag) from a value object
const TAG_MASK: usize = 0b1111;

/// The mask to apply to get just the pointer address from a pointer object
const PTR_MASK: usize = !TAG_MASK;

/// The amount of bits to shift-left the actual value in value objects (last 4 bits store the type tag)
const VALUE_SHIFT_BITS: usize = 4;

#[allow(unused)]
/// The max integer value we can store in a value object
const MAX_INT: isize = isize::MAX >> VALUE_SHIFT_BITS;

#[allow(unused)]
/// The minimum integer value we can store in a value object
const MIN_INT: isize = isize::MIN >> VALUE_SHIFT_BITS;

// ARM uses 49 bits and x86-64 uses 48 bits
// we have at least 15 bits to work with
// this is 4 bits and it supports up to 16 variants
#[derive(Debug, PartialEq, Copy, Clone, PartialOrd, Ord, Eq)]
#[repr(u8)]
pub enum Type {
    // The types below are all stored directly inside the pointer
    Null = 0b0000,
    Bool,
    Function,

    // The types below are all heap-allocated
    Int,
    Float,
    String,
    Rune,
    Array,
    Map,
    Iter,
    Struct,
    Ref,
    Closure,
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
        let s = Self((raw as usize | t as usize) as _);
        assert_eq!(s.tag(), t);
        s
    }

    /// Returns the type of this object pointer
    #[inline(always)]
    pub fn tag(self) -> Type {
        // Safety: self.0 with TAG_MASK applied will always yield a correct Type
        unsafe { std::mem::transmute((self.0 as usize & TAG_MASK) as u8) }
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
    pub fn as_int(self) -> isize {
        assert_eq!(Type::Int, self.tag());
        unsafe { Int::read(&self) }
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

    /// Returns the f64 value of this object pointer
    /// Panics if object does not point to a Float
    #[inline]
    pub fn as_f64(self) -> f64 {
        assert_eq!(self.tag(), Type::Float);
        unsafe { self.as_f64_unchecked() }
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
    pub unsafe fn as_f64_unchecked(self) -> f64 {
        Float::read(&self)
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

    /// Returns true if this pointer does not contain an immediate value
    /// But points to a heap allocated type (like Float, String or Array)
    #[inline]
    pub fn is_heap_allocated(self) -> bool {
        self.0 as usize & TAG_MASK >= Type::Float as usize
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
        // TODO: Maybe delay type check (on other object) to here
        //  (and then only for heap-allocated objects)
        match self.tag() {
            Type::Null | Type::Bool | Type::Int | Type::Function => self.0 == other.0,
            Type::Float => unsafe { self.as_f64_unchecked() == other.as_f64_unchecked() },
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
            Type::Null | Type::Bool | Type::Int => self.0.partial_cmp(&other.0),
            Type::Float => unsafe { self.as_f64_unchecked().partial_cmp(&other.as_f64()) },
            Type::String => unsafe { self.as_str_unchecked().partial_cmp(other.as_str()) },
            Type::Array
            | Type::Function
            | Type::Ref
            | Type::Map
            | Type::Iter
            | Type::Struct
            | Type::Rune
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
                Type::Int => Object::int(self.as_int() $op rhs.as_int()),

                // Safety: We've already asserted the object type
                Type::Float => unsafe {
                    Object::float(self.as_f64_unchecked() $op rhs.as_f64_unchecked(), gc)
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

#[repr(C)]
pub struct Closure {
    header: Header,
    pub ip: u32,
    pub num_locals: u16,
    pub enclosed_objects: Vec<Object>,
}

impl Closure {
    pub unsafe fn read(ptr: &Object) -> &Self {
        ptr.get::<Self>()
    }

    pub unsafe fn read_mut(ptr: &Object) -> &mut Self {
        ptr.get_mut::<Self>()
    }

    pub fn object(ip: u32, num_locals: u16, enclosed_objects: Vec<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Closure);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        obj.ip = ip;
        obj.num_locals = num_locals;

        init!(obj.enclosed_objects => enclosed_objects);
        ptr
    }
}

#[repr(C)]
pub struct Ref {
    header: Header,
    pub value: Object,
}

impl Ref {
    fn from_obj(value: Object) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Ref);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
struct Int {
    header: Header,
    value: isize,
}

impl Int {
    #[inline]
    unsafe fn read(obj: &Object) -> isize {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    fn from_isize(value: isize) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Int);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
struct Float {
    header: Header,
    value: f64,
}

impl Float {
    #[inline]
    unsafe fn read(obj: &Object) -> f64 {
        obj.get::<Self>().value
    }

    #[inline]
    unsafe fn destroy(obj: Object) {
        drop_in_place(obj.as_ptr() as *mut Self);
        dealloc(obj.as_ptr(), Layout::new::<Self>());
    }

    fn from_f64(value: f64) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Float);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value );
        ptr
    }
}

#[repr(C)]
struct String {
    header: Header,
    value: RString,
}

impl String {
    unsafe fn destroy(ptr: Object) {
        drop_in_place(ptr.as_ptr() as *mut Self);
        dealloc(ptr.as_ptr(), Layout::new::<Self>());
    }

    fn from_string(value: RString) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::String);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => value);
        ptr
    }
}

#[repr(C)]
pub struct Map {
    header: Header,
    value: BTreeMap<Object, Object>,
}

impl Map {
    pub(crate) fn from_map(map: BTreeMap<Object, Object>, gc: &mut GC) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Map);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => map);
        ptr
    }
}

#[repr(C)]
pub struct Array {
    header: Header,
    value: Vec<Object>,
}

impl Array {
    unsafe fn read(ptr: &Object) -> &Vec<Object> {
        ptr.get::<Self>().value.as_ref()
    }

    /// Drops and deallocate this NlArray struct and its value
    unsafe fn destroy(ptr: Object) {
        drop_in_place(ptr.as_ptr() as *mut Self);
        dealloc(ptr.as_ptr(), Layout::new::<Self>());
    }

    fn from_vec(vec: Vec<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Array);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => vec);
        ptr
    }

    fn from_slice(slice: &[Object]) -> Object {
        Self::from_vec(slice.to_vec())
    }
}

impl Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.tag() {
            Type::Null => f.write_str("nil")?,
            Type::Bool => f.write_str(if self.as_bool() { "true" } else { "false" })?,
            Type::Float => unsafe { f.write_str(&self.as_f64_unchecked().to_string())? },
            Type::Int => f.write_str(&self.as_int().to_string())?,
            Type::String => unsafe { f.write_str(self.as_str_unchecked())? },
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
            Type::Rune => unimplemented!(),
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
                std::fmt::Display::fmt(&self.as_ref().value, f)?;
            }
        }
        Ok(())
    }
}

#[repr(C)]
pub struct Struct {
    header: Header,
    name: RString,
    pub values: Vec<Object>,
}

impl Struct {
    unsafe fn read(ptr: &Object) -> &Self {
        ptr.get::<Self>()
    }

    unsafe fn read_mut(ptr: &Object) -> &mut Self {
        ptr.get_mut::<Self>()
    }

    pub fn object(name: RString, values: Vec<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Struct);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.values => values);
        init!(obj.name => name);
        ptr
    }
}

#[repr(C)]
pub enum IterType {
    Map(IntoIter<Object, Object>),
    Array(std::iter::Enumerate<std::vec::IntoIter<Object>>),
}

#[repr(C)]
pub struct ObjIter {
    header: Header,
    value: IterType,
}

impl ObjIter {
    unsafe fn read(ptr: &mut Object) -> &mut ObjIter {
        ptr.get_mut::<Self>()
    }

    pub fn next(&mut self) -> (Object, Object) {
        match &mut self.value {
            IterType::Map(map_iter) => map_iter.next().unwrap_or((Object::null(), Object::null())),
            IterType::Array(iter) => iter
                .next()
                .map(|(a, b)| (Object::int(a as isize), b))
                .unwrap_or((Object::null(), Object::null())),
        }
    }

    pub fn from_obj(obj: Object) -> Object {
        match obj.tag() {
            Type::Map => Self::from_map(obj.as_map().clone()),
            Type::Array => Self::from_vec(obj.as_vec().clone()),
            _ => panic!("not an iterator: {:#?}", obj),
        }
    }

    pub fn from_map(map: BTreeMap<Object, Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Iter);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => IterType::Map(map.into_iter()));
        ptr
    }

    pub fn from_vec(vec: Vec<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Iter);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => IterType::Array(vec.into_iter().enumerate()));
        ptr
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
            Type::Int => "int",
            Type::String => "string",
            Type::Array => "array",
            Type::Function => "func",
            Type::Map => "map",
            Type::Iter => "iter",
            Type::Ref => "&",
            Type::Struct => "struct",
            Type::Rune => "rune",
            Type::Closure => "closure",
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
        assert!(!Object::null().is_heap_allocated());
    }

    #[test]
    fn test_object_bool() {
        let t = Object::bool(true);
        let f = Object::bool(false);

        assert_eq!(t.tag(), Type::Bool);
        assert_eq!(f.tag(), Type::Bool);

        assert_eq!(t.as_bool(), true);
        assert_eq!(f.as_bool(), false);

        assert!(!t.is_heap_allocated());
        assert!(!f.is_heap_allocated());
    }

    #[test]
    fn test_object_int() {
        let obj = Object::int(1);
        assert_eq!(obj.tag(), Type::Int);
        assert_eq!(obj.as_int(), 1);

        let obj = Object::int(-1);
        assert_eq!(obj.tag(), Type::Int);
        assert_eq!(obj.as_int(), -1);

        let obj = Object::int(MAX_INT);
        assert_eq!(obj.tag(), Type::Int);
        assert_eq!(obj.as_int(), MAX_INT);

        let obj = Object::int(MIN_INT);
        assert_eq!(obj.tag(), Type::Int);
        assert_eq!(obj.as_int(), MIN_INT);
    }

    #[test]
    #[should_panic]
    fn test_object_int_overflow() {
        assert_eq!(Object::int(MAX_INT + 1).tag(), Type::Int);
        assert_eq!(Object::int(MAX_INT + 1).as_int(), MAX_INT + 1);

        assert_eq!(Object::int(MIN_INT - 1).tag(), Type::Int);
        assert_eq!(Object::int(MIN_INT - 1).as_int(), MIN_INT - 1);
    }

    #[test]
    fn test_object_string() {
        let mut gc = GC::new();
        let obj = Object::string("Hello, world!", &mut gc);
        assert_eq!(obj.tag(), Type::String);
        assert!(obj.is_heap_allocated());
        assert_eq!(obj.as_str(), "Hello, world!");
    }

    #[test]
    fn test_pointer_float() {
        let mut gc = GC::new();
        let obj = Object::float(std::f64::consts::PI, &mut gc);
        assert_eq!(obj.tag(), Type::Float);
        assert!(obj.is_heap_allocated());
        assert_eq!(obj.as_f64(), std::f64::consts::PI);
    }

    #[test]
    fn test_pointer_empty_array() {
        let ptr = Array::from_slice(&[]);
        assert_eq!(ptr.tag(), Type::Array);
        assert!(ptr.is_heap_allocated());
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
