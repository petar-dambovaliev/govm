use crate::vm::gc::GC;
use crate::vm::object::rune::Rune;
use crate::vm::object::{allocate, Header, Object, Type};
use std::alloc::{dealloc, Layout};
use std::collections::btree_map::IntoIter;
use std::collections::BTreeMap;
use std::ptr::drop_in_place;

macro_rules! init {
    ($field: expr => $value: expr) => {
        unsafe {
            std::ptr::addr_of_mut!($field).write($value);
        }
    };
}

#[repr(C)]
pub struct Map {
    header: Header,
    pub(crate) value: BTreeMap<Object, Object>,
}

impl Map {
    pub(crate) unsafe fn read_mut(ptr: &Object) -> &mut BTreeMap<Object, Object> {
        &mut ptr.get_mut::<Self>().value
    }

    pub(crate) unsafe fn read(ptr: &Object) -> &Self {
        &ptr.get::<Self>()
    }
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
    pub(crate) value: Vec<Object>,
}

impl Array {
    pub(crate) unsafe fn read(ptr: &Object) -> &Vec<Object> {
        ptr.get::<Self>().value.as_ref()
    }

    /// Drops and deallocate this NlArray struct and its value
    pub(crate) unsafe fn destroy(ptr: Object) {
        drop_in_place(ptr.as_ptr() as *mut Self);
        dealloc(ptr.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_vec(vec: Vec<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Array);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => vec);
        ptr
    }

    pub(crate) fn from_slice(slice: &[Object]) -> Object {
        Self::from_vec(slice.to_vec())
    }
}

#[repr(C)]
pub struct Slice {
    header: Header,
    pub(crate) value: Vec<Object>,
}

impl Slice {
    pub(crate) unsafe fn read(ptr: &Object) -> &Vec<Object> {
        ptr.get::<Self>().value.as_ref()
    }

    /// Drops and deallocate this NlArray struct and its value
    pub(crate) unsafe fn destroy(ptr: Object) {
        drop_in_place(ptr.as_ptr() as *mut Self);
        dealloc(ptr.as_ptr(), Layout::new::<Self>());
    }

    pub(crate) fn from_vec(vec: Vec<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Slice);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => vec);
        ptr
    }

    pub(crate) fn from_slice(slice: &[Object]) -> Object {
        Self::from_vec(slice.to_vec())
    }
}

#[repr(C)]
pub enum IterType {
    Map(IntoIter<Object, Object>),
    Array(std::iter::Enumerate<std::vec::IntoIter<Object>>),
    String(RuneIter),
}

pub struct RuneIter {
    s: Object,
    i: usize,
}

impl RuneIter {
    pub fn next(&mut self) -> (Object, Object) {
        let s = self.s.as_string_mut();
        let r = match s.chars().nth(self.i) {
            Some(ch) => (Object::int(self.i as isize), Rune::from_char(ch)),
            None => (Object::null(), Object::null()),
        };
        self.i += 1;
        r
    }
}

#[repr(C)]
pub struct ObjIter {
    header: Header,
    value: IterType,
}

impl ObjIter {
    pub(crate) unsafe fn read(ptr: &mut Object) -> &mut ObjIter {
        ptr.get_mut::<Self>()
    }

    pub fn next(&mut self) -> (Object, Object) {
        match &mut self.value {
            IterType::Map(map_iter) => map_iter.next().unwrap_or((Object::null(), Object::null())),
            IterType::Array(iter) => iter
                .next()
                .map(|(a, b)| (Object::int(a as isize), b))
                .unwrap_or((Object::null(), Object::null())),
            IterType::String(iter) => iter.next(),
        }
    }

    pub fn from_obj(obj: Object) -> Object {
        match obj.tag() {
            Type::Map => Self::from_map(obj.as_map().clone()),
            Type::Array => Self::from_vec(obj.as_vec().clone()),
            Type::Slice => Self::from_vec(obj.as_vec().clone()),
            Type::String => Self::from_str(obj),
            Type::Variadic => {
                let v = unsafe { Variadic::read(&obj) };
                Self::from_vec(v.clone())
            }
            t => panic!("not an iterator: t: {:#?} obj:{:#?}", t, obj),
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

    pub fn from_str(s: Object) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Iter);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;

        init!(obj.value => IterType::String(RuneIter{i:0, s}));
        ptr
    }
}

#[repr(C)]
pub struct Variadic {
    header: Header,
    pub(crate) value: Vec<Object>,
}

impl Variadic {
    pub(crate) unsafe fn read(ptr: &Object) -> &Vec<Object> {
        ptr.get::<Self>().value.as_ref()
    }

    /// Drops and deallocate this NlArray struct and its value
    pub(crate) unsafe fn destroy(ptr: Object) {
        drop_in_place(ptr.as_ptr() as *mut Self);
        dealloc(ptr.as_ptr(), Layout::new::<Self>());
    }

    pub fn from_vec(vec: Vec<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Variadic);
        let obj = unsafe { ptr.get_mut::<Self>() };
        obj.header.marked = false;
        init!(obj.value => vec);
        ptr
    }

    pub(crate) fn from_slice(slice: &[Object]) -> Object {
        Self::from_vec(slice.to_vec())
    }
}
