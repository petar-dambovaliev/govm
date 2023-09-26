use crate::vm::object::rune::Rune;
use crate::vm::object::structure::TypeValue;
use crate::vm::object::{allocate, Object, Type};
use std::alloc::Layout;
use std::collections::btree_map::IntoIter;
use std::collections::BTreeMap;

macro_rules! init {
    ($field: expr => $value: expr) => {
        unsafe {
            std::ptr::addr_of_mut!($field).write($value);
        }
    };
}

#[repr(C)]
pub struct Map {
    pub(crate) value: BTreeMap<Object, Object>,
}

impl Map {
    pub(crate) unsafe fn read_mut(ptr: &Object) -> &mut BTreeMap<Object, Object> {
        &mut ptr.get_mut::<Self>().value
    }

    #[allow(unused)]
    pub(crate) unsafe fn read(ptr: &Object) -> &Self {
        &ptr.get::<Self>()
    }
    pub(crate) fn from_map(map: BTreeMap<Object, Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Map);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => map);
        ptr
    }
}

#[repr(C)]
pub struct Array {
    pub(crate) value: Vec<Object>,
}

impl Array {
    pub(crate) unsafe fn read(ptr: &Object) -> &Vec<Object> {
        ptr.get::<Self>().value.as_ref()
    }

    pub(crate) fn from_vec(vec: Vec<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Array);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => vec);
        ptr
    }

    pub(crate) fn from_slice(slice: &[Object]) -> Object {
        Self::from_vec(slice.to_vec())
    }
}

#[repr(C)]
pub struct Slice {
    pub(crate) value: Vec<Object>,
    pub(crate) type_value: TypeValue,
    pub(crate) is_null: bool,
}

impl Slice {
    pub(crate) fn null(type_value: TypeValue) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Slice);
        let obj = unsafe { ptr.get_mut::<Self>() };

        obj.is_null = true;
        init!(obj.value => vec![]);
        init!(obj.type_value => type_value);
        ptr
    }
    pub(crate) unsafe fn read(ptr: &Object) -> &Vec<Object> {
        ptr.get::<Self>().value.as_ref()
    }

    pub(crate) fn from_vec(vec: Vec<Object>, type_value: TypeValue) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Slice);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => vec);
        init!(obj.type_value => type_value);
        ptr
    }

    pub(crate) fn from_slice(slice: &[Object], type_value: TypeValue) -> Object {
        Self::from_vec(slice.to_vec(), type_value)
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

        init!(obj.value => IterType::Map(map.into_iter()));
        ptr
    }

    pub fn from_vec(vec: Vec<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Iter);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => IterType::Array(vec.into_iter().enumerate()));
        ptr
    }

    pub fn from_str(s: Object) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Iter);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => IterType::String(RuneIter{i:0, s}));
        ptr
    }
}

#[repr(C)]
pub struct Variadic {
    pub(crate) value: Vec<Object>,
}

impl Variadic {
    pub(crate) unsafe fn read(ptr: &Object) -> &Vec<Object> {
        ptr.get::<Self>().value.as_ref()
    }

    pub fn from_vec(vec: Vec<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Variadic);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => vec);
        ptr
    }

    #[allow(unused)]
    pub(crate) fn from_slice(slice: &[Object]) -> Object {
        Self::from_vec(slice.to_vec())
    }
}
