use crate::parser::ast::{ArrayType, BasicLit, Expression, Ident};
use crate::parser::token::LitKind;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    TypeError(String),
    SyntaxError(String),
    ReferenceError(String),
    IndexError(String),
    ArgumentError(String),
    InternalError(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TypeError(s) => write!(f, "TypeError: {}", s),
            Error::SyntaxError(s) => write!(f, "SyntaxError: {}", s),
            Error::ReferenceError(s) => write!(f, "ReferenceError: {}", s),
            Error::IndexError(s) => write!(f, "IndexError: {}", s),
            Error::ArgumentError(s) => write!(f, "ArgumentError: {}", s),
            Error::InternalError(s) => write!(f, "InternalError: {}", s),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, PartialEq, Copy, Clone, PartialOrd, Ord, Eq, Hash)]
#[repr(u8)]
pub enum RuntimeType {
    Null = 0,
    Bool,
    Function,
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
    Interface,
    Type,
    Slice,
    Variadic,
    Alias,
    Channel,
}

impl Display for RuntimeType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            RuntimeType::Null => "nil",
            RuntimeType::Bool => "bool",
            RuntimeType::Float32 => "float32",
            RuntimeType::Float64 => "float64",
            RuntimeType::Byte => "byte",
            RuntimeType::Int => "int",
            RuntimeType::UI => "uint",
            RuntimeType::I8 => "int8",
            RuntimeType::I16 => "int16",
            RuntimeType::I32 => "int32",
            RuntimeType::I64 => "int64",
            RuntimeType::UI8 => "uint8",
            RuntimeType::UI16 => "uint16",
            RuntimeType::UI32 => "uint32",
            RuntimeType::UI64 => "uint64",
            RuntimeType::String => "string",
            RuntimeType::Array => "array",
            RuntimeType::Function => "func",
            RuntimeType::Map => "map",
            RuntimeType::Iter => "iter",
            RuntimeType::Ref => "&",
            RuntimeType::Struct => "struct",
            RuntimeType::Rune => "rune",
            RuntimeType::Closure => "closure",
            RuntimeType::Complex64 => "complex64",
            RuntimeType::Complex128 => "complex128",
            RuntimeType::Interface => "interface",
            RuntimeType::Type => "type",
            RuntimeType::Slice => "slice",
            RuntimeType::Variadic => "variadic",
            RuntimeType::Alias => "alias",
            RuntimeType::Channel => "chan",
        };
        write!(f, "{}", str)
    }
}

#[derive(Debug, Clone)]
pub struct SymbolTable {
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

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug, Hash)]
pub enum ContextType {
    Named(String, DefineType),
    Embedded(String, DefineType),
    Unnamed(DefineType),
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug, Hash, Copy)]
pub enum Qualifier {
    Var,
    Const,
    Invar,
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug, Hash)]
pub enum DefineType {
    Null,
    Qualified(Qualifier, Box<Self>),
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
    Uintptr,
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
    Type(Box<Self>, RuntimeType),
    Interface {
        name: String,
        methods: Vec<Self>,
    },
    Variadic(Box<Self>),
    Spec {
        name: String,
        inner: Box<Self>,
        is_transparent: bool,
        methods: Vec<Self>,
    },
    Channel(Box<DefineType>),
    Package {
        path: String,
        alias: String,
    },
}

impl DefineType {
    pub fn udt_ident(&self) -> Option<String> {
        match self {
            Self::Spec { name, .. } => Some(name.clone()),
            Self::Ref(inner) => inner.udt_ident(),
            Self::Type(inner, _) => inner.udt_ident(),
            Self::Interface { name, .. } => Some(name.clone()),
            Self::Func { name, .. } => Some(name.clone()),
            _ => None,
        }
    }
}

impl Display for DefineType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Struct { name, .. } => name.to_string(),
            Self::Func { name, .. } => format!("func {}", name),
            Self::Int => "int".to_string(),
            Self::Byte => "byte".to_string(),
            Self::Int8 => "int8".to_string(),
            Self::Int16 => "int16".to_string(),
            Self::Int32 => "int32".to_string(),
            Self::Int64 => "int64".to_string(),
            Self::Uint => "uint".to_string(),
            Self::Uint8 => "uint8".to_string(),
            Self::Uint16 => "uint16".to_string(),
            Self::Uint32 => "uint32".to_string(),
            Self::Uint64 => "uint64".to_string(),
            Self::Uintptr => "uintptr".to_string(),
            Self::Bool => "bool".to_string(),
            Self::Float32 => "float32".to_string(),
            Self::Float64 => "float64".to_string(),
            Self::String => "string".to_string(),
            Self::Rune => "rune".to_string(),
            Self::Complex64 => "complex64".to_string(),
            Self::Complex128 => "complex128".to_string(),
            Self::Null => "null".to_string(),
            Self::Array { inner_type, len } => format!("[{}]{}", len, inner_type),
            Self::Slice(inner) => format!("[]{}", inner),
            Self::Map(k, v) => format!("map[{}]{}", k, v),
            Self::Ref(inner) => format!("*{}", inner),
            Self::Tuple(types) => {
                let parts: Vec<_> = types.iter().map(|t| t.to_string()).collect();
                format!("({})", parts.join(", "))
            }
            Self::Type(_, rt) => rt.to_string(),
            Self::Interface { name, .. } => format!("interface {}", name),
            Self::Variadic(inner) => format!("...{}", inner),
            Self::Spec { name, .. } => name.to_string(),
            Self::Iter(inner) => format!("iter({})", inner),
            Self::Qualified(_, inner) => inner.to_string(),
            Self::Channel(inner) => format!("chan {}", inner),
            Self::Package { path, .. } => format!("package {}", path),
        };
        f.write_str(&s)
    }
}

fn normalize_alias(t: DefineType) -> DefineType {
    match t {
        DefineType::Byte => DefineType::Uint8,
        DefineType::Rune => DefineType::Int32,
        DefineType::Tuple(v) => DefineType::Tuple(v.into_iter().map(normalize_alias).collect()),
        other => other,
    }
}

pub fn types_equal(a: &DefineType, b: &DefineType) -> bool {
    normalize_alias(a.unwrap_to_base_type()) == normalize_alias(b.unwrap_to_base_type())
}

pub fn is_integer_coerceable_to(i: isize, t: &DefineType) -> bool {
    if !t.is_integer() {
        return false;
    }
    let val = i as i128;
    match t.integer_range_i128() {
        Ok((min, max)) => val >= min && val <= max,
        Err(_) => false,
    }
}

pub fn is_uint_coerceable_to(i: usize, t: &DefineType) -> bool {
    if !t.is_integer() {
        return false;
    }
    let val = i as i128;
    match t.integer_range_i128() {
        Ok((min, max)) => val >= min && val <= max,
        Err(_) => false,
    }
}

impl DefineType {
    pub fn fmt_rt(&self) -> String {
        match self {
            Self::Struct { name, .. } => name.clone(),
            Self::Tuple(types) => {
                let s: Vec<_> = types.iter().map(|t| t.fmt_rt()).collect();
                s.join(",")
            }
            Self::Type(_, t) => t.to_string(),
            other => other.to_string(),
        }
    }

    pub fn struct_has_method(&self, name: &str) -> bool {
        let Ok((_, _, methods)) = self.as_struct() else {
            return false;
        };
        for method in methods {
            if let Ok((f_name, _, _, _)) = method.as_func() {
                if name == f_name {
                    return true;
                }
            }
        }
        false
    }

    pub fn eq_structs(l: &Self, r: &Self) -> bool {
        let (Ok((n1, fields1, _)), Ok((n2, fields2, _))) = (l.as_struct(), r.as_struct()) else {
            return false;
        };
        n1 == n2 && fields1 == fields2
    }

    pub fn as_array(&self) -> Result<(usize, Self), Error> {
        match &self {
            Self::Array { len, inner_type } => Ok((*len, *inner_type.clone())),
            _ => Err(Error::InternalError(format!(
                "expected Self::Array, got {:#?}",
                self
            ))),
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
            other => Expression::Ident(Ident {
                pos: 0,
                name: other.to_string(),
            }),
        }
    }

    pub fn get_type_name(&self) -> Result<String, Error> {
        match self {
            Self::Struct { name: n, .. } => Ok(n.to_string()),
            Self::Ref(inner) => inner.get_type_name(),
            Self::Spec { name: n, .. } => Ok(n.to_string()),
            Self::Interface { name: n, .. } => Ok(n.to_string()),
            Self::Func { name: n, .. } => Ok(n.to_string()),
            Self::Qualified(_, inner) => inner.get_type_name(),
            Self::Type(inner, _) => inner.get_type_name(),
            other => Ok(other.to_string()),
        }
    }

    pub fn is_float(&self) -> bool {
        match self {
            Self::Float32 | Self::Float64 => true,
            Self::Qualified(_, inner) => inner.is_float(),
            _ => false,
        }
    }

    pub fn is_const_coerceable_to(&self, other: &DefineType) -> bool {
        match self {
            Self::Qualified(Qualifier::Const, inner) => inner.is_integer() && other.is_float(),
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
        if self.is_numeric() && other.is_numeric() {
            return true;
        }
        if self.is_spec() {
            if let Ok((_, inner, _, _)) = self.as_spec() {
                return other == &inner.strip_type();
            }
        }
        if other.is_spec() {
            if let Ok((_, _, _, is_transparent)) = other.as_spec() {
                return is_transparent;
            }
        }
        false
    }

    pub fn integer_range_i128(&self) -> Result<(i128, i128), Error> {
        match &self {
            Self::Int => Ok((isize::MIN as i128, isize::MAX as i128)),
            Self::Byte => Ok((u8::MIN as i128, u8::MAX as i128)),
            Self::Int8 => Ok((i8::MIN as i128, i8::MAX as i128)),
            Self::Int16 => Ok((i16::MIN as i128, i16::MAX as i128)),
            Self::Int32 | Self::Rune => Ok((i32::MIN as i128, i32::MAX as i128)),
            Self::Int64 => Ok((i64::MIN as i128, i64::MAX as i128)),
            Self::Uint => Ok((usize::MIN as i128, usize::MAX as i128)),
            Self::Uint8 => Ok((u8::MIN as i128, u8::MAX as i128)),
            Self::Uint16 => Ok((u16::MIN as i128, u16::MAX as i128)),
            Self::Uint32 => Ok((u32::MIN as i128, u32::MAX as i128)),
            Self::Uint64 => Ok((u64::MIN as i128, u64::MAX as i128)),
            Self::Uintptr => Ok((u32::MIN as i128, u32::MAX as i128)),
            Self::Qualified(_, inner) => inner.integer_range_i128(),
            _ => Err(Error::InternalError(format!(
                "integer_range_i128 called on non-integer type: {:#?}",
                self
            ))),
        }
    }

    pub fn is_integer(&self) -> bool {
        match &self {
            Self::Int | Self::Byte | Self::Int8 | Self::Int16 | Self::Int32 | Self::Int64
            | Self::Uint | Self::Uint8 | Self::Uint16 | Self::Uint32 | Self::Uint64
            | Self::Uintptr | Self::Rune => true,
            Self::Qualified(_, inner) => inner.is_integer(),
            _ => false,
        }
    }

    pub fn is_invar(&self) -> bool {
        matches!(self, Self::Qualified(Qualifier::Invar, _))
    }

    pub fn is_interface(&self) -> bool {
        matches!(self, Self::Interface { .. })
    }

    pub fn is_numeric(&self) -> bool {
        match &self {
            Self::Int | Self::Byte | Self::Int8 | Self::Int16 | Self::Int32 | Self::Int64
            | Self::Uint | Self::Uint8 | Self::Uint16 | Self::Uint32 | Self::Uint64
            | Self::Uintptr | Self::Float32 | Self::Float64
            | Self::Rune => true,
            Self::Qualified(_, inner) => inner.is_numeric(),
            _ => false,
        }
    }

    pub fn strip_type(&self) -> DefineType {
        match self {
            Self::Type(v, _) => *v.clone(),
            Self::Ref(v) => Self::Ref(Box::new(v.strip_type())),
            Self::Struct { name, fields, methods } => {
                let fields = fields
                    .iter()
                    .map(|f| match f {
                        ContextType::Named(n, t) => {
                            ContextType::Named(n.clone(), t.strip_type())
                        }
                        ContextType::Embedded(n, t) => {
                            ContextType::Named(n.clone(), t.strip_type())
                        }
                        ContextType::Unnamed(t) => {
                            ContextType::Unnamed(t.strip_type())
                        }
                    })
                    .collect();
                Self::Struct {
                    name: name.clone(),
                    fields,
                    methods: methods.clone(),
                }
            }
            Self::Tuple(v) => {
                DefineType::Tuple(v.iter().map(|t| t.unwrap_to_base_type()).collect())
            }
            Self::Array { len, inner_type } => DefineType::Array {
                len: *len,
                inner_type: Box::new(inner_type.strip_type()),
            },
            _ => self.clone(),
        }
    }

    pub fn strip_ref(&self) -> DefineType {
        if let Self::Ref(v) = self { *v.clone() } else { self.clone() }
    }

    pub fn strip_var(&self) -> DefineType {
        if let Self::Qualified(Qualifier::Var, v) = self { *v.clone() } else { self.clone() }
    }

    pub fn strip_const(&self) -> DefineType {
        if let Self::Qualified(Qualifier::Const, v) = self { *v.clone() } else { self.clone() }
    }

    pub fn unwrap_to_base_type(&self) -> DefineType {
        match self {
            Self::Qualified(_, inner) | Self::Ref(inner) => inner.unwrap_to_base_type(),
            Self::Type(inner, _) => inner.unwrap_to_base_type(),
            Self::Tuple(v) => {
                DefineType::Tuple(v.iter().map(|el| el.unwrap_to_base_type()).collect())
            }
            other => other.clone(),
        }
    }

    pub fn strip_ret(&self) -> DefineType {
        match self {
            DefineType::Type(r, _) => r.clone().strip_ret(),
            DefineType::Tuple(v) => {
                DefineType::Tuple(v.iter().map(|rt| rt.strip_ret()).collect())
            }
            other => other.clone(),
        }
    }

    pub fn type_to_val_t(&self) -> DefineType {
        match self {
            DefineType::Type(r, _) => *r.clone(),
            DefineType::Tuple(v) => {
                DefineType::Tuple(v.iter().map(|rt| rt.type_to_val_t()).collect())
            }
            other => other.clone(),
        }
    }

    pub fn is_nil(&self) -> bool {
        match &self {
            Self::Ref(r) => r.is_nil(),
            Self::Type(t, _) => t.is_nil(),
            Self::Null => true,
            _ => false,
        }
    }

    pub fn is_nullable(&self) -> bool {
        matches!(
            self,
            Self::Ref(_) | Self::Func { .. } | Self::Map(_, _) | Self::Slice { .. } | Self::Channel(_)
        )
    }

    pub fn is_slice(&self) -> bool { matches!(self, Self::Slice { .. }) }
    pub fn is_struct(&self) -> bool { matches!(self, Self::Struct { .. }) }
    pub fn is_spec(&self) -> bool { matches!(self, Self::Spec { .. }) }
    pub fn is_byte(&self) -> bool { matches!(self, Self::Byte) }
    pub fn is_rune(&self) -> bool { matches!(self, Self::Rune) }
    pub fn is_var(&self) -> bool { matches!(self, Self::Qualified(Qualifier::Var, _)) }
    pub fn is_variadic(&self) -> bool { matches!(self, Self::Variadic(_)) }

    pub fn as_variadic(&self) -> Result<DefineType, Error> {
        match &self {
            Self::Variadic(t) => Ok(*t.clone()),
            _ => Err(Error::InternalError(format!(
                "expected Variadic, got {:#?}",
                self
            ))),
        }
    }

    pub fn is_ref(&self) -> bool { matches!(self, Self::Ref(_)) }
    pub fn is_type(&self) -> bool { matches!(self, Self::Type(_, _)) }
    pub fn is_tuple(&self) -> bool { matches!(self, Self::Tuple(_)) }
    pub fn is_func(&self) -> bool { matches!(self, Self::Func { .. }) }

    pub fn as_type(&self) -> Result<(DefineType, RuntimeType), Error> {
        match &self {
            Self::Type(df, t) => Ok((*df.clone(), *t)),
            _ => Err(Error::InternalError(format!(
                "expected Self::Type, got {:#?}",
                self
            ))),
        }
    }

    pub fn as_tuple(&self) -> Result<Vec<DefineType>, Error> {
        match &self {
            Self::Tuple(t) => Ok(t.clone()),
            _ => Err(Error::InternalError(format!(
                "expected Self::Tuple, got {:#?}",
                self
            ))),
        }
    }

    pub fn as_struct(&self) -> Result<(String, Vec<ContextType>, Vec<DefineType>), Error> {
        match &self {
            Self::Struct { name, fields, methods } => {
                Ok((name.clone(), fields.clone(), methods.clone()))
            }
            _ => Err(Error::InternalError(format!(
                "expected Self::Struct, got {:#?}",
                self
            ))),
        }
    }

    pub fn as_spec(&self) -> Result<(String, DefineType, Vec<DefineType>, bool), Error> {
        match &self {
            Self::Spec { name, inner, methods, is_transparent } => Ok((
                name.clone(),
                *inner.clone(),
                methods.clone(),
                *is_transparent,
            )),
            _ => Err(Error::InternalError(format!(
                "expected Self::Spec, got {:#?}",
                self
            ))),
        }
    }

    pub fn as_interface(&self) -> Result<(String, Vec<DefineType>), Error> {
        match &self {
            Self::Interface { methods, name } => Ok((name.clone(), methods.clone())),
            _ => Err(Error::InternalError(format!(
                "expected Self::Interface, got {:#?}",
                self
            ))),
        }
    }

    pub fn as_var(&self) -> Result<DefineType, Error> {
        match &self {
            Self::Qualified(Qualifier::Var, t) => Ok(*t.clone()),
            _ => Err(Error::InternalError(format!(
                "expected Qualified(Var, ...), got {:#?}",
                self
            ))),
        }
    }

    pub fn as_func(&self) -> Result<(String, Option<Box<DefineType>>, Vec<ContextType>, Box<DefineType>), Error> {
        match &self {
            Self::Func { name, recv, args, rt } => {
                Ok((name.clone(), recv.clone(), args.clone(), rt.clone()))
            }
            _ => Err(Error::InternalError(format!(
                "expected Self::Func, got {:#?}",
                self
            ))),
        }
    }

    pub fn as_ref_type(&self) -> Result<DefineType, Error> {
        match &self {
            Self::Ref(t) => Ok(*t.clone()),
            _ => Err(Error::InternalError(format!(
                "expected Self::Ref, got {:#?}",
                self
            ))),
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
                "ContextType::as_named: expected named got: {:#?}",
                self
            ))),
        }
    }

    pub fn as_unnamed(&self) -> Result<DefineType, Error> {
        match &self {
            Self::Unnamed(t) => Ok(t.clone()),
            _ => Err(Error::InternalError(format!(
                "expected Unnamed, got {:#?}",
                self
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Context {
    pub scope: Scope,
    max_size: usize,
    pub symbols: Vec<Vec<(String, DefineType, String)>>,
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

    pub fn max_size(&self) -> usize {
        self.max_size
    }

    #[inline]
    fn total_len(&self) -> usize {
        self.symbols.iter().fold(0, |acc, s| acc + s.len())
    }

    fn define(&mut self, pkg: &str, name: &str, dt: DefineType, invar: bool) -> Symbol {
        let current_scope = self.symbols.last_mut().expect("symbol table has no scopes");
        current_scope.push((name.to_string(), dt, pkg.to_string()));
        self.max_size += 1;

        Symbol {
            index: (self.total_len() - 1)
                .try_into()
                .expect("symbol index overflow (too many symbols)"),
            scope: self.scope,
            invar,
        }
    }

    #[inline]
    pub fn resolve(&self, pkg: &str, name: &str) -> Option<(Symbol, DefineType)> {
        let mut abs_index = self.total_len();

        for scope in self.symbols.iter().rev() {
            abs_index -= scope.len();

            if let Some((index, _)) = scope.iter().enumerate().rev().find(|(_index, n)| {
                n.0 == name && (n.2 == pkg || n.2 == "0xbuiltin")
            }) {
                return Some((
                    Symbol {
                        index: (abs_index + index)
                            .try_into()
                            .expect("symbol index overflow (too many symbols)"),
                        scope: self.scope,
                        invar: scope[index].1.is_invar(),
                    },
                    scope[index].1.clone(),
                ));
            }
        }
        None
    }

    pub fn update_dt(&mut self, pkg: &str, name: &str, dt: DefineType) -> bool {
        for scope in self.symbols.iter_mut().rev() {
            if let Some(index) = scope.iter().position(|n| n.0 == name && n.2 == pkg) {
                scope[index].1 = dt.clone();
                return true;
            }
        }
        false
    }

    pub fn update_struct_fields(&mut self, pkg: &str, name: &str, fields: Vec<ContextType>) -> bool {
        for scope in self.symbols.iter_mut().rev() {
            if let Some(index) = scope.iter().position(|n| n.0 == name && n.2 == pkg) {
                assert!(scope[index].1.is_struct());
                let (name, _, methods) = scope[index].1.as_struct().unwrap();
                scope[index].1 = DefineType::Struct { name, fields, methods };
                return true;
            }
        }
        false
    }
}

#[derive(Debug)]
pub enum Resolved {
    Enclosed((Symbol, DefineType, String)),
    Local((Symbol, DefineType, String)),
}

impl Resolved {
    pub fn get_symbol(&self) -> Symbol {
        match &self {
            Self::Local((s, _, _)) | Self::Enclosed((s, _, _)) => s.clone(),
        }
    }

    pub fn get_type(&self) -> (DefineType, String) {
        match &self {
            Self::Local((_, t, pkg)) | Self::Enclosed((_, t, pkg)) => (t.clone(), pkg.clone()),
        }
    }

    pub fn as_local(&self) -> Result<(Symbol, DefineType, String), Error> {
        match &self {
            Self::Enclosed(_) => Err(Error::InternalError(format!(
                "expected Local, got Enclosed: {:#?}",
                self
            ))),
            Self::Local(s) => Ok(s.clone()),
        }
    }
}

impl SymbolTable {
    pub fn new() -> Self {
        SymbolTable {
            contexts: vec![Context::new(Scope::Global, false)],
        }
    }

    pub fn current_context(&mut self) -> &mut Context {
        self.contexts
            .last_mut()
            .expect("SymbolTable has no contexts")
    }

    pub fn new_context(&mut self, is_closure: bool) {
        self.contexts.push(Context::new(Scope::Local, is_closure));
    }

    pub fn leave_context(&mut self) -> Context {
        self.contexts
            .pop()
            .expect("cannot leave context: no contexts on stack")
    }

    pub fn enter_scope(&mut self) {
        self.current_context().symbols.push(Vec::new());
    }

    pub fn leave_scope(&mut self) {
        self.current_context()
            .symbols
            .pop()
            .expect("cannot leave scope: no scopes on stack");
    }

    pub fn define(&mut self, pkg: &str, name: &str, dt: DefineType, invar: bool) -> Symbol {
        self.current_context().define(pkg, name, dt, invar)
    }

    pub fn resolve(&mut self, pkg: &str, name: &str) -> Option<Resolved> {
        for (i, ctx) in self.contexts.iter().rev().enumerate() {
            let symbol = ctx.resolve(pkg, name);
            if let Some(s) = symbol {
                if i != 0 {
                    for (k, ctxk) in self.contexts.iter_mut().rev().enumerate() {
                        let exists = ctxk.captured.iter().any(|a| a == name);
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
                        return Some(Resolved::Enclosed((
                            Symbol {
                                scope: Scope::Local,
                                index: st.try_into().unwrap(),
                                invar: false,
                            },
                            s.1.clone(),
                            pkg.to_string(),
                        )));
                    }
                }
                return Some(Resolved::Local((s.0, s.1, pkg.to_string())));
            }

            if !ctx.is_closure {
                break;
            }
        }

        if self.contexts.len() > 1 {
            self.contexts[0]
                .resolve(pkg, name)
                .map(|a| Resolved::Local((a.0, a.1, pkg.to_string())))
        } else {
            None
        }
    }

    pub fn ident_is_package(&mut self, name: &str) -> bool {
        self.resolve("", name).is_some()
    }

    pub fn get_package_path(&mut self, name: &str) -> Option<String> {
        let dt = self.resolve("", name)?;
        let (t, _) = dt.get_type();
        match t {
            DefineType::Package { path, .. } => Some(path),
            _ => None,
        }
    }

    pub fn update_dt(&mut self, pkg: &str, name: &str, dt: DefineType) -> bool {
        let len = self.contexts.len();
        if self.current_context().update_dt(pkg, name, dt.clone()) {
            return true;
        }
        if len > 1 {
            self.contexts[0].update_dt(pkg, name, dt)
        } else {
            false
        }
    }

    pub fn update_struct_fields(&mut self, pkg: &str, name: &str, fields: Vec<ContextType>) -> bool {
        let len = self.contexts.len();
        if self.current_context().update_struct_fields(pkg, name, fields.clone()) {
            return true;
        }
        if len > 1 {
            self.contexts[0].update_struct_fields(pkg, name, fields)
        } else {
            false
        }
    }
}
