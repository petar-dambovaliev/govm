use std::cmp::Ordering;
use std::ops::{Add, Sub};
use broom::{Handle, Rooted, Heap};
use broom::prelude::{Trace, Tracer};
use crate::vm::Error;

// all the types that will be handled by the GC
#[derive(Clone, Debug)]
pub enum Object {
    Int64(i64),
    Float64(f64),
    String(String),
    Bool(bool),
    List(Vec<Handle<Self>>),
    Box(Rooted<Self>),
    Fn{ip: u32, num_locals: u16},
    Nil,
}


impl Object {
    pub fn type_string(&self) -> Self {
        let t = match self {
            Self::String(_) => "string",
            Self::Int64(_) => "int",
            _ => unimplemented!()
        };

        Self::String(t.to_string())
    }

    pub fn to_string_object(&self) -> Result<Self, Error> {
        let s = match self {
            Self::Int64(i) => i.to_string(),
            _ => unimplemented!()
        };
        Ok(Self::String(s))
    }
}

impl Add for Object {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Int64(a), Self::Int64(b)) => {
                return Self::Int64(a + b)
            }
            _ => unimplemented!()
        }
    }
}

impl Sub for Object {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Int64(a), Self::Int64(b)) => {
                return Self::Int64(a - b)
            }
            _ => unimplemented!()
        }
    }
}

impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int64(a), Self::Int64(b)) => a.eq(b),
            (Self::Bool(a), Self::Bool(b)) => a.eq(b),
            _=> unimplemented!()
        }
    }
}

impl PartialOrd for Object {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Int64(a), Self::Int64(b)) => a.partial_cmp(b),
            (Self::Bool(a), Self::Bool(b)) => a.partial_cmp(b),
            _=> unimplemented!()
        }
    }
}

impl Trace<Self> for Object {
    fn trace(&self, tracer: &mut Tracer<Self>) {
        match self {
            Self::Int64(_) => {}
            Self::Float64(_) => {}
            Self::List(objects) => objects.trace(tracer),
            Self::Box(root) => root.trace(tracer),
            Self::String(_) => {}
            Self::Bool(_) => {}
            Self::Fn{ .. } => {}
            Self::Nil => {}
        }
    }
}

macro_rules! impl_arith {
    ($func_name:ident, $op:tt) => {
        #[inline(always)]
        pub fn $func_name(self, rhs: Self, gc: &mut Heap<Object>) -> Result<Object, Error> {
            Ok(self $op rhs)
        }
    };
}

macro_rules! impl_logical {
    ($func_name:ident, $op:tt) => {
        #[inline(always)]
        pub fn $func_name(self, rhs: Self, _gc: &mut Heap<Object>) -> Result<Object, Error> {
            let result = match (self, rhs) {
                (Object::Bool(a), Object::Bool(b)) => Object::Bool(a $op b),
                _ => return Err(Error::TypeError(format!("for operation {#:?} invalid types {:#?} and {:#?}", stringify!($op), self, rhs)))
            };
            Ok(result)
        }
    };
}

macro_rules! impl_cmp {
    ($func_name:ident, $op:tt) => {
        #[inline(always)]
        pub fn $func_name(self, rhs: Self, _gc: &mut Heap<Object>) -> Result<Object, Error> {
            // Delegate actual comparison to PartialOrd/PartialEq implementation
            Ok(Object::Bool(self $op rhs,))
        }
    };
}

impl Object {
    impl_arith!(add, +);
    impl_arith!(sub, -);
    //impl_arith!(mul, *);
    //impl_arith!(div, /);
    //impl_arith!(rem, %);

    impl_cmp!(gt, >);
    impl_cmp!(gte, >=);
    impl_cmp!(lt, <);
    impl_cmp!(lte, <=);
    impl_cmp!(eq, ==);
    impl_cmp!(neq, !=);

    //impl_logical!(and, &&);
    //impl_logical!(or, ||);
}