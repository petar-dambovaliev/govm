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
pub struct Alias {
    pub(crate) name: String,
    pub value: Object,
    pub method_dispatch: Vec<(String, usize)>,
    pub is_transparent: bool,
}

impl Alias {
    pub(crate) unsafe fn read(ptr: &Object) -> &Self {
        ptr.get::<Self>()
    }

    pub(crate) unsafe fn read_mut(ptr: &Object) -> &mut Self {
        ptr.get_mut::<Self>()
    }

    pub fn object(
        name: String,
        value: Object,
        method_dispatch: Vec<(String, usize)>,
        is_transparent: bool,
    ) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Alias);
        let obj = unsafe { ptr.get_mut::<Self>() };

        obj.is_transparent = is_transparent;
        init!(obj.value => value);
        init!(obj.method_dispatch => method_dispatch);
        init!(obj.name => name);
        ptr
    }
}

#[repr(C)]
pub struct Struct {
    pub(crate) name: String,
    pub values: Vec<Object>,
    pub method_dispatch: Vec<(String, usize)>,
    pub tags: Vec<Option<String>>,
    pub is_anonymous: bool,
}

impl Struct {
    pub(crate) unsafe fn read(ptr: &Object) -> &Self {
        ptr.get::<Self>()
    }

    pub(crate) unsafe fn read_mut(ptr: &Object) -> &mut Self {
        ptr.get_mut::<Self>()
    }

    pub fn object(
        name: String,
        values: Vec<Object>,
        method_dispatch: Vec<(String, usize)>,
        tags: Vec<Option<String>>,
        is_anonymous: bool,
    ) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Struct);
        let obj = unsafe { ptr.get_mut::<Self>() };

        obj.is_anonymous = is_anonymous;
        init!(obj.values => values);
        init!(obj.method_dispatch => method_dispatch);
        init!(obj.tags => tags);
        init!(obj.name => name);
        ptr
    }
}

#[repr(C)]
pub struct Interface {
    pub(crate) name: String,
    pub value: Object,
    pub methods: Vec<String>,
}

impl Interface {
    pub(crate) unsafe fn read(ptr: &Object) -> &Self {
        ptr.get::<Self>()
    }

    pub fn object(name: String, methods: Vec<String>, value: Object) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Interface);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value);
        init!(obj.methods => methods);
        init!(obj.name => name);
        ptr
    }
}

#[derive(PartialEq, Clone, Debug)]
#[repr(C)]
pub struct TypeValue {
    pub(crate) value: Type,
    pub inner_k: Option<Object>,
    pub inner_v: Option<Object>,
}

impl TypeValue {
    pub(crate) unsafe fn read(ptr: &Object) -> &Self {
        ptr.get::<Self>()
    }

    pub fn object(value: Type, inner_k: Option<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Type);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value);
        init!(obj.inner_k => inner_k);
        init!(obj.inner_v => None);
        ptr
    }

    pub fn object_map(value: Type, inner_k: Option<Object>, inner_v: Option<Object>) -> Object {
        let ptr = Object::with_type(allocate(Layout::new::<Self>()), Type::Type);
        let obj = unsafe { ptr.get_mut::<Self>() };

        init!(obj.value => value);
        init!(obj.inner_k => inner_k);
        init!(obj.inner_v => inner_v);
        ptr
    }
}
