use crate::parser::ast::{ArrayType, BasicLit, Expression, Ident};
use crate::parser::token::LitKind;
use crate::vm::compiler::compiler::Compiler;
use crate::vm::object::structure::TypeValue;
use crate::vm::object::Object;
use crate::vm::object::Type;
use crate::vm::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub(crate) struct SymbolTable {
    /// A vector of contexts
    /// The context at index 0 will always be the global context,
    /// any context that follows is a local (to a function) context.
    /// There can be more than one local context as functions can be nested inside other functions.
    pub contexts: Vec<Context>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Symbol {
    pub scope: Scope,
    pub index: u16,
    pub invar: bool,
}

#[derive(PartialEq, Copy, Clone, Debug, Eq)]
pub enum Scope {
    Local,
    Global,
}

// a stmt is terminating if
// `for`
//      1. there are no "break" statements referring to the "for" statement, and
//      2. the loop condition is absent, and
//      3. the "for" statement does not use a range clause.
// `if`
//      1. the "else" branch is present, and
//      2. both branches are terminating statements.
// `switch`
//      1. there are no "break" statements referring to the "switch" statement,
//      2. there is a default case, and
//      3. the statement lists in each case, including the default, end in a terminating statement, or a possibly labeled "fallthrough" statement.
//
//  `label`
//      1. A labeled statement labeling a terminating statement

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug, Hash)]
pub enum ContextType {
    //    key,  type
    Named(String, DefineType),
    Embedded(String, DefineType),
    Unnamed(DefineType),
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug, Hash)]
pub enum DefineType {
    Null,
    Var(Box<Self>),
    Const(Box<Self>),
    Struct {
        name: String,
        fields: Vec<ContextType>,
        methods: Vec<Self>,
    },
    Func {
        name: String,
        recv: Option<Box<Self>>,
        args: Vec<ContextType>,
        rt: Box<Self>,
    },
    Int,
    Byte,
    Int8,
    Int16,
    Int32,
    Int64,
    Uint,
    Uint8,
    Uint16,
    Uint32,
    Uint64,
    Complex64,
    Complex128,
    Bool,
    Float32,
    Float64,
    String,
    Rune,
    Array {
        inner_type: Box<Self>,
        len: usize,
    },
    Slice(Box<Self>),
    Map(Box<Self>, Box<Self>),
    Iter(Box<Self>),
    Ref(Box<Self>),
    Tuple(Vec<Self>),
    Type(Box<Self>, Type),
    Invar(Box<Self>),
    //should only contain functions
    Interface {
        name: String,
        methods: Vec<Self>,
    },
    Variadic(Box<Self>),
}

impl Display for DefineType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Struct { name, .. } => name.to_string(),
            _ => unimplemented!(),
        };

        f.write_str(&s)
    }
}

// impl PartialEq for DefineType {
//     fn eq(&self, other: &Self) -> bool {
//         match (self, other) {
//             (DefineType::Struct { .. }, DefineType::Struct { .. }) => {
//                 let (n1, _, _) = self.as_struct().unwrap();
//                 let (n2, _, _) = other.as_struct().unwrap();
//                 n1 == n2
//             }
//             (DefineType::Null, DefineType::Null) => true,
//             (DefineType::Array { .. }, DefineType::Array { .. }) => {
//                 let (ll, ldt) = self.as_array();
//                 let (lr, rdt) = self.as_array();
//
//                 ll == lr && ldt == rdt
//             }
//             (DefineType::Int, DefineType::Int) => true,
//             (DefineType::Float64, DefineType::Float64) => true,
//             (DefineType::Float32, DefineType::Float32) => true,
//             (DefineType::Slice(t1), DefineType::Slice(t2)) => t1 == t2,
//             (DefineType::Type(dt1, t1), DefineType::Type(dt2, t2)) => dt1 == dt2 && t1 == t2,
//             (DefineType::Map(k1, v1), DefineType::Map(k2, v2)) => k1 == k2 && v1 == v2,
//             (DefineType::)
//             _ => false,
//         }
//     }
// }

pub fn is_integer_coerceable_to(i: isize, t: &DefineType) -> bool {
    if !t.is_integer() {
        return false;
    }
    if i > 0 {
        let num = i as usize;
        let (min, max) = t.integer_max_usize();
        if num >= min && num <= max {
            return true;
        }
    } else {
        let (min, max) = t.integer_max_isize();
        if i >= min && i <= max {
            return true;
        }
    }
    false
}

pub fn is_uint_coerceable_to(i: usize, t: &DefineType) -> bool {
    if !t.is_integer() {
        return false;
    }

    let (min, max) = t.integer_max_usize();
    if i >= min && i <= max {
        return true;
    }
    false
}

impl DefineType {
    pub fn fmt_rt(&self) -> String {
        match self {
            Self::Struct { name, .. } => name.clone(),
            Self::Tuple(types) => {
                let mut s = Vec::with_capacity(types.len());

                for t in types {
                    s.push(t.fmt_rt());
                }

                let r = s.join(",");
                r
            }
            Self::Type(_, t) => t.to_string(),
            t => unimplemented!("{:#?}", t),
        }
    }
    pub fn struct_has_method(&self, name: &str) -> bool {
        let (_, _, methods) = self.as_struct().unwrap();

        for method in methods {
            let (f_name, _, _, _) = method.as_func();

            if name == f_name {
                return true;
            }
        }

        false
    }
    pub fn eq_structs(l: &Self, r: &Self) -> bool {
        let (n1, fields1, _) = l.as_struct().unwrap();
        let (n2, fields2, _) = r.as_struct().unwrap();

        n1 == n2 && fields1 == fields2
    }
    pub fn as_array(&self) -> (usize, Self) {
        match &self {
            Self::Array { len, inner_type } => (len.clone(), *inner_type.clone()),
            _ => panic!("expected Self::Tuple, got {:#?}", self),
        }
    }
    pub fn to_object(mut self) -> Object {
        self = self.strip_var();

        match self {
            DefineType::Type(dt, _t) => match *dt.clone() {
                DefineType::String => TypeValue::object(Type::String, None),
                DefineType::Int => TypeValue::object(Type::Int, None),
                _ => dt.to_object(),
            },
            DefineType::Slice(inner) => {
                let inner = inner.to_object();
                TypeValue::object(Type::Slice, Some(inner))
            }
            DefineType::Map(k, v) => {
                let k_obj = k.to_object();
                let v_obj = v.to_object();

                TypeValue::object_map(Type::Map, Some(k_obj), Some(v_obj))
            }
            // DefineType::Array { len, inner_type } => Expression::TypeArray(ArrayType {
            //     pos: (0, 0),
            //     len: Box::new(Expression::BasicLit(BasicLit {
            //         pos: 0,
            //         kind: LitKind::Integer,
            //         value: format!("{}", len),
            //     })),
            //     typ: Box::new(inner_type.to_expression()),
            // }),
            _ => unimplemented!("DefineType::to_object {:#?}", self),
        }
    }
    pub fn to_expression(self) -> Expression {
        match self {
            DefineType::Type(_, t) => Expression::Ident(Ident {
                pos: 0,
                name: t.to_string(),
            }),
            DefineType::Array { len, inner_type } => Expression::TypeArray(ArrayType {
                pos: (0, 0),
                len: Box::new(Expression::BasicLit(BasicLit {
                    pos: 0,
                    kind: LitKind::Integer,
                    value: format!("{}", len),
                })),
                typ: Box::new(inner_type.to_expression()),
            }),
            _ => unimplemented!("DefineType::to_expression {:#?}", self),
        }
    }
    pub fn get_type_name(&self) -> String {
        match self {
            Self::Struct { name: n, .. } => n.to_string(),
            Self::Ref(inner) => inner.get_type_name(),
            _ => panic!("not implemented for {:#?}", self),
        }
    }
    pub fn implements(&self, interface: &Self, c: &mut Compiler) -> bool {
        let mut expect_methods = interface.as_interface().1;

        for expect_method in &mut expect_methods {
            let (name, _, args, rts) = expect_method.as_func();

            let new_args: Vec<ContextType> = args
                .iter()
                .map(|b| {
                    if let ContextType::Named(_, c) = b {
                        ContextType::Unnamed(c.clone())
                    } else {
                        b.clone()
                    }
                })
                .collect();

            *expect_method = DefineType::Func {
                name,
                recv: None,
                args: new_args,
                rt: rts,
            };
        }

        let (strct, is_ref) = if self.is_ref() {
            (self.as_ref(), true)
        } else {
            (self.clone(), false)
        };

        let mut got_methods = if strct.is_struct() {
            let (name, fields, _) = strct.as_struct().unwrap();

            let (_, _, mut got_methods) = c
                .symbols
                .resolve(&name)
                .unwrap()
                .as_local()
                .1
                .as_struct()
                .unwrap();

            for field in fields {
                if let ContextType::Embedded(e_name, e_t) = field {
                    panic!("{:#?}", e_t);
                }
            }

            got_methods
        } else {
            vec![]
        };

        for gm in &mut got_methods {
            let (name, recv, args, rt) = gm.as_func();

            if let Some(rr) = recv {
                if is_ref != rr.is_ref() {
                    continue;
                }
            }

            let new_args: Vec<ContextType> = args
                .iter()
                .map(|b| {
                    if let ContextType::Named(_, c) = b {
                        ContextType::Unnamed(c.clone())
                    } else {
                        b.clone()
                    }
                })
                .collect();

            *gm = DefineType::Func {
                name,
                recv: None,
                args: new_args,
                rt,
            };
        }

        fn is_subset<T: PartialEq>(subset: &[T], superset: &[T]) -> bool {
            for item in subset {
                if !superset.contains(item) {
                    return false;
                }
            }
            true
        }

        is_subset(&expect_methods, &got_methods)
    }

    pub fn is_float(&self) -> bool {
        match self {
            Self::Float32 => true,
            Self::Float64 => true,
            _ => false,
        }
    }

    pub fn is_const_coerceable_to(&self, other: &DefineType) -> bool {
        match self {
            Self::Const(inner) => inner.is_integer() && other.is_float(),
            _ => false,
        }
    }
    pub fn is_coerceable_to(&self, other: &DefineType) -> bool {
        if self.is_integer() && other.is_integer() {
            return true;
        }

        if self.is_integer() && other.is_rune() {
            return true;
        }

        if self.is_rune() && other.is_byte() {
            return true;
        }

        false
    }

    pub fn integer_max_usize(&self) -> (usize, usize) {
        match &self {
            Self::Int => (isize::MIN as usize, isize::MAX as usize),
            Self::Byte => (u8::MIN as usize, u8::MAX as usize),
            Self::Int8 => (i8::MIN as usize, i8::MAX as usize),
            Self::Int16 => (i16::MIN as usize, i16::MAX as usize),
            Self::Int32 => (i32::MIN as usize, i32::MAX as usize),
            Self::Int64 => (i64::MIN as usize, i64::MAX as usize),
            Self::Uint => (usize::MIN, usize::MAX),
            Self::Uint8 => (u8::MIN as usize, u8::MAX as usize),
            Self::Uint16 => (u16::MIN as usize, u16::MAX as usize),
            Self::Uint32 => (u32::MIN as usize, u32::MAX as usize),
            Self::Uint64 => (u64::MIN as usize, u64::MAX as usize),
            _ => panic!("not integer: {:#?}", self),
        }
    }

    pub fn integer_max_isize(&self) -> (isize, isize) {
        match &self {
            Self::Int => (isize::MIN, isize::MAX),
            Self::Byte => (u8::MIN as isize, u8::MAX as isize),
            Self::Int8 => (i8::MIN as isize, i8::MAX as isize),
            Self::Int16 => (i16::MIN as isize, i16::MAX as isize),
            Self::Int32 => (i32::MIN as isize, i32::MAX as isize),
            Self::Int64 => (i64::MIN as isize, i64::MAX as isize),
            Self::Uint => (usize::MIN as isize, usize::MAX as isize),
            Self::Uint8 => (u8::MIN as isize, u8::MAX as isize),
            Self::Uint16 => (u16::MIN as isize, u16::MAX as isize),
            Self::Uint32 => (u32::MIN as isize, u32::MAX as isize),
            Self::Uint64 => (u64::MIN as isize, u64::MAX as isize),
            _ => panic!("not integer: {:#?}", self),
        }
    }

    pub fn is_integer(&self) -> bool {
        match &self {
            Self::Int
            | Self::Byte
            | Self::Int8
            | Self::Int16
            | Self::Int32
            | Self::Int64
            | Self::Uint
            | Self::Uint8
            | Self::Uint16
            | Self::Uint32
            | Self::Uint64 => true,
            _ => false,
        }
    }
    pub fn is_invar(&self) -> bool {
        match &self {
            Self::Invar(_) => true,
            _ => false,
        }
    }

    pub fn is_interface(&self) -> bool {
        match &self {
            Self::Interface { .. } => true,
            _ => false,
        }
    }

    pub fn is_numeric(&self) -> bool {
        match &self {
            Self::Int
            | Self::Byte
            | Self::Int8
            | Self::Int16
            | Self::Int32
            | Self::Int64
            | Self::Uint
            | Self::Uint8
            | Self::Uint16
            | Self::Uint32
            | Self::Uint64
            | Self::Float32
            | Self::Float64 => true,
            _ => false,
        }
    }
    pub fn strip_type(&self) -> DefineType {
        match self {
            Self::Type(v, _) => *v.clone(),
            Self::Ref(v) => Self::Ref(Box::new(v.strip_type())),
            Self::Struct {
                name,
                fields,
                methods,
            } => {
                let fields = fields
                    .iter()
                    .map(|f| {
                        let field = match f {
                            ContextType::Named(n, t) => (n, t),
                            ContextType::Embedded(n, t) => (n, t),
                            _ => unimplemented!(),
                        };
                        ContextType::Named(field.0.clone(), field.1.strip_type())
                    })
                    .collect();

                Self::Struct {
                    name: name.clone(),
                    fields,
                    methods: methods.clone(),
                }
            }
            Self::Tuple(v) => {
                let mut tuple = vec![];

                for t in v {
                    tuple.push(t.strip_var().strip_type());
                }
                DefineType::Tuple(tuple)
            }
            Self::Array { len, inner_type } => DefineType::Array {
                len: *len,
                inner_type: Box::new(inner_type.strip_type()),
            },
            _ => self.clone(),
        }
    }

    pub fn strip_ref(&self) -> DefineType {
        if let Self::Ref(v) = self {
            *v.clone()
        } else {
            self.clone()
        }
    }
    pub fn strip_var(&self) -> DefineType {
        if let Self::Var(v) = self {
            *v.clone()
        } else {
            self.clone()
        }
    }

    pub fn strip_const(&self) -> DefineType {
        if let Self::Const(v) = self {
            *v.clone()
        } else {
            self.clone()
        }
    }
    pub fn strip_ret(&self) -> DefineType {
        match &self {
            DefineType::Type(r, _) => r.clone().strip_ret(),
            DefineType::Tuple(v) => {
                let mut new_v = Vec::with_capacity(v.len());
                for rt in v {
                    new_v.push(rt.strip_ret());
                }
                DefineType::Tuple(new_v)
            }
            &r => r.clone(),
        }
    }
    pub fn type_to_val_t(&self) -> DefineType {
        match &self {
            DefineType::Type(r, _) => *r.clone(),
            DefineType::Tuple(v) => {
                let mut new_v = Vec::with_capacity(v.len());
                for rt in v {
                    new_v.push(rt.type_to_val_t());
                }
                DefineType::Tuple(new_v)
            }
            &r => r.clone(),
        }
    }

    //nil for interfaces, slices, channels, maps, pointers and functions.
    pub fn is_nil(&self) -> bool {
        match &self {
            Self::Ref(r) => r.is_nil(),
            Self::Type(t, _) => t.is_nil(),
            Self::Null => true,
            _ => false,
        }
    }
    pub fn is_nullable(&self) -> bool {
        match &self {
            Self::Ref(_) | Self::Func { .. } | Self::Map(_, _) | Self::Slice { .. } => true,
            _ => false,
        }
    }

    pub fn is_slice(&self) -> bool {
        match &self {
            Self::Slice { .. } => true,
            _ => false,
        }
    }
    pub fn is_struct(&self) -> bool {
        match &self {
            Self::Struct { .. } => true,
            _ => false,
        }
    }

    pub fn is_byte(&self) -> bool {
        match &self {
            Self::Byte => true,
            _ => false,
        }
    }

    pub fn is_rune(&self) -> bool {
        match &self {
            Self::Rune => true,
            _ => false,
        }
    }

    pub fn is_var(&self) -> bool {
        match &self {
            Self::Var(_) => true,
            _ => false,
        }
    }

    pub fn is_variadic(&self) -> bool {
        match &self {
            Self::Variadic(_) => true,
            _ => false,
        }
    }

    pub fn as_variadic(&self) -> DefineType {
        match &self {
            Self::Variadic(t) => *t.clone(),
            _ => panic!(),
        }
    }

    pub fn is_ref(&self) -> bool {
        match &self {
            Self::Ref(_) => true,
            _ => false,
        }
    }

    pub fn is_type(&self) -> bool {
        match &self {
            Self::Type(_, _) => true,
            _ => false,
        }
    }

    pub fn is_tuple(&self) -> bool {
        match &self {
            Self::Tuple(_) => true,
            _ => false,
        }
    }

    pub fn is_func(&self) -> bool {
        match &self {
            Self::Func { .. } => true,
            _ => false,
        }
    }

    pub fn as_type(&self) -> (DefineType, Type) {
        match &self {
            Self::Type(df, t) => (*df.clone(), t.clone()),
            _ => panic!("expected Self::Type, got {:#?}", self),
        }
    }

    pub fn as_tuple(&self) -> Vec<DefineType> {
        match &self {
            Self::Tuple(t) => t.clone(),
            _ => panic!("expected Self::Tuple, got {:#?}", self),
        }
    }

    pub fn as_struct(&self) -> Result<(String, Vec<ContextType>, Vec<DefineType>), Error> {
        match &self {
            Self::Struct {
                name,
                fields,
                methods,
            } => Ok((name.clone(), fields.clone(), methods.clone())),
            _ => Err(Error::InternalError(format!(
                "expected Self::Struct, got {:#?}",
                self
            ))),
        }
    }

    pub fn as_interface(&self) -> (String, Vec<DefineType>) {
        match &self {
            Self::Interface { methods, name } => (name.clone(), methods.clone()),
            _ => panic!("expected Self::Interface, got {:#?}", self),
        }
    }

    pub fn as_var(&self) -> DefineType {
        match &self {
            Self::Var(t) => *t.clone(),
            _ => panic!("expected Self::Var, got {:#?}", self),
        }
    }

    pub fn as_func(
        &self,
    ) -> (
        String,
        Option<Box<DefineType>>,
        Vec<ContextType>,
        Box<DefineType>,
    ) {
        match &self {
            Self::Func {
                name,
                recv,
                args,
                rt,
            } => (name.clone(), recv.clone(), args.clone(), rt.clone()),
            _ => panic!("expected Self::Var, got {:#?}", self),
        }
    }

    pub fn as_ref(&self) -> DefineType {
        match &self {
            Self::Ref(t) => *t.clone(),
            _ => panic!("expected Self::Var, got {:#?}", self),
        }
    }
}

impl ContextType {
    pub fn get_type(&self) -> DefineType {
        match self {
            Self::Unnamed(t) => t.clone(),
            Self::Named(_, t) => t.clone(),
            Self::Embedded(_, t) => t.clone(),
        }
    }
    pub fn as_named(&self) -> Result<(String, DefineType), Error> {
        match &self {
            Self::Named(s, t) => Ok((s.clone(), t.clone())),
            _ => Err(Error::InternalError(format!(
                "ContextType:as_named: expected named got: {:#?}",
                self
            ))),
        }
    }

    pub fn as_unnamed(&self) -> DefineType {
        match &self {
            Self::Unnamed(t) => t.clone(),
            _ => panic!(),
        }
    }
}

/// A context is a type of environment to store values in. This can be either a global context or a local (to a function) context.
#[derive(Debug)]
pub(crate) struct Context {
    pub scope: Scope,
    max_size: usize,
    pub symbols: Vec<Vec<(String, DefineType)>>,
    pub is_closure: bool,
    pub captured: Vec<String>,
}

impl Context {
    fn new(scope: Scope, is_closure: bool) -> Self {
        Context {
            scope,
            max_size: 0,
            symbols: vec![Vec::new()],
            is_closure,
            captured: Vec::new(),
        }
    }

    /// The maximum number of symbols defined in this context.
    /// Not all of these symbols may still be in scope once this context is destroyed.
    pub(crate) fn max_size(&self) -> usize {
        self.max_size
    }

    /// The (current) number of defined symbols in this context.
    #[inline]
    fn total_len(&self) -> usize {
        self.symbols.iter().fold(0, |acc, s| acc + s.len())
    }

    /// Defines a new symbol in the current context its inner-most scope.
    fn define(&mut self, name: &str, dt: DefineType, invar: bool) -> Symbol {
        //println!("define: {}", name);
        let current_scope = self.symbols.last_mut().unwrap();
        current_scope.push((name.to_string(), dt));
        self.max_size += 1;

        Symbol {
            index: (self.total_len() - 1).try_into().unwrap(),
            scope: self.scope,
            invar,
        }
    }

    /// Resolves a symbol in this context along with its absolute index (relative to the context its top scope)
    #[inline]
    fn resolve(&self, name: &str) -> Option<(Symbol, DefineType)> {
        let mut abs_index = self.total_len();

        for scope in self.symbols.iter().rev() {
            abs_index -= scope.len();

            if let Some((index, _)) = scope
                .iter()
                .enumerate()
                .rev()
                .find(|(_index, n)| n.0 == name)
            {
                return Some((
                    Symbol {
                        index: (abs_index + index).try_into().unwrap(),
                        scope: self.scope,
                        invar: scope[index].1.is_invar(),
                    },
                    scope[index].1.clone(),
                ));
            }
        }
        None
    }

    pub fn update_dt(&mut self, name: &str, dt: DefineType) -> bool {
        for scope in self.symbols.iter_mut().rev() {
            if let Some(index) = scope.iter().position(|n| n.0 == name) {
                scope[index].1 = dt.clone();
                return true;
            }
        }
        false
    }

    pub fn update_struct_fields(&mut self, name: &str, fields: Vec<ContextType>) -> bool {
        for scope in self.symbols.iter_mut().rev() {
            if let Some(index) = scope.iter().position(|n| n.0 == name) {
                assert!(scope[index].1.is_struct());
                let (name, _, methods) = scope[index].1.as_struct().unwrap();

                scope[index].1 = DefineType::Struct {
                    name,
                    fields,
                    methods,
                };
                return true;
            }
        }
        false
    }
}

#[derive(Debug)]
pub enum Resolved {
    Enclosed((Symbol, DefineType)),
    Local((Symbol, DefineType)),
}

impl Resolved {
    pub fn get_symbol(&self) -> Symbol {
        match &self {
            Self::Local((s, _)) | Self::Enclosed((s, _)) => s.clone(),
        }
    }
    pub fn get_type(&self) -> DefineType {
        match &self {
            Self::Local((_, t)) | Self::Enclosed((_, t)) => t.clone(),
        }
    }

    pub fn as_local(&self) -> (Symbol, DefineType) {
        match &self {
            Self::Enclosed(_) => panic!("as_local: {:#?}", self),
            Self::Local(s) => s.clone(),
        }
    }
}

impl SymbolTable {
    /// Creates a new symbol table with a globally scoped context
    pub fn new() -> Self {
        SymbolTable {
            contexts: vec![Context::new(Scope::Global, false)],
        }
    }

    /// Returns a mutable reference to the current context
    pub fn current_context(&mut self) -> &mut Context {
        self.contexts.last_mut().unwrap()
    }

    /// Create a new context to define symbols in.
    /// This will always be a local context (as there is only one global context).
    pub fn new_context(&mut self, is_closure: bool) {
        self.contexts.push(Context::new(Scope::Local, is_closure));
    }

    /// Destroys the current context and returns the maximum number of symbols it had at some point in time.
    pub fn leave_context(&mut self) -> Context {
        self.contexts.pop().unwrap()
    }

    /// Enter a new scope in the current context
    /// For example, at the start of a block statement.
    pub fn enter_scope(&mut self) {
        self.current_context().symbols.push(Vec::new());
    }

    /// Leave scope in the current context.
    /// For example, at the end of a block statement.
    pub fn leave_scope(&mut self) {
        self.current_context().symbols.pop().unwrap();
    }

    /// Define a symbol in the current context (and current scope within that context).
    pub fn define(&mut self, name: &str, dt: DefineType, invar: bool) -> Symbol {
        self.current_context().define(name, dt, invar)
    }

    ///Resolve a symbol in either the current context or the global context if no local was found.
    /// For closures, keep looking in outer scopes (not global) and return if the symbol is from the outer scope
    pub fn resolve(&mut self, name: &str) -> Option<Resolved> {
        for (i, ctx) in self.contexts.iter().rev().enumerate() {
            let symbol = ctx.resolve(name);
            if let Some(s) = symbol {
                //if its not in the current scope and not already inserted
                // put it in the enclosed symbols

                if i != 0 {
                    for (k, ctxk) in self.contexts.iter_mut().rev().enumerate() {
                        let exists = ctxk.captured.iter().find(|&a| a == name).is_some();

                        if !exists {
                            ctxk.captured.push(name.to_string());
                        }

                        if k == i {
                            break;
                        }
                    }

                    if let Some(st) = self
                        .current_context()
                        .captured
                        .iter()
                        .position(|a| a == name)
                    {
                        let r = Resolved::Enclosed((
                            Symbol {
                                scope: Scope::Local,
                                index: st.try_into().unwrap(),
                                invar: false,
                            },
                            s.1.clone(),
                        ));

                        return Some(r);
                    }
                }
                return Some(Resolved::Local(s));
            }

            if !ctx.is_closure {
                break;
            }
        }

        if self.contexts.len() > 1 {
            self.contexts[0].resolve(name).map(|a| Resolved::Local(a))
        } else {
            None
        }
    }

    pub fn update_dt(&mut self, name: &str, dt: DefineType) -> bool {
        let len = self.contexts.len();

        // Try getting a mutable reference from the current context
        if self.current_context().update_dt(name, dt.clone()) {
            return true;
        }

        if len > 1 {
            self.contexts[0].update_dt(name, dt)
        } else {
            false
        }
    }

    pub fn update_struct_fields(&mut self, name: &str, fields: Vec<ContextType>) -> bool {
        let len = self.contexts.len();

        // Try getting a mutable reference from the current context
        if self
            .current_context()
            .update_struct_fields(name, fields.clone())
        {
            return true;
        }

        if len > 1 {
            self.contexts[0].update_struct_fields(name, fields)
        } else {
            false
        }
    }
}
