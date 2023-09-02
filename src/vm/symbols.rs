use crate::vm::object::Type;

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

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
pub enum ContextType {
    //    key,  type
    Named(String, DefineType),
    Unnamed(DefineType),
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
pub enum DefineType {
    Null,
    Var(Box<Self>),
    Struct(String, Vec<ContextType>),
    Func(String, Vec<ContextType>, Box<Self>),
    Int,
    Bool,
    Float,
    String,
    Rune,
    Array(Box<Self>),
    Map(Box<Self>, Box<Self>),
    Iter(Box<Self>),
    Ref(Box<Self>),
    Tuple(Vec<Self>),
    Type(Box<Self>, Type),
}

impl DefineType {
    pub fn strip_var(&self) -> DefineType {
        if let Self::Var(v) = self {
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
            Self::Null => true,
            _ => false,
        }
    }
    pub fn is_nullable(&self) -> bool {
        match &self {
            Self::Ref(_) | Self::Func(_, _, _) | Self::Map(_, _) | Self::Array(_) => true,
            _ => false,
        }
    }
    pub fn is_struct(&self) -> bool {
        match &self {
            Self::Struct(_, _) => true,
            _ => false,
        }
    }

    pub fn is_var(&self) -> bool {
        match &self {
            Self::Var(_) => true,
            _ => false,
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

    pub fn is_func(&self) -> bool {
        match &self {
            Self::Func(_, _, _) => true,
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

    pub fn as_var(&self) -> DefineType {
        match &self {
            Self::Var(t) => *t.clone(),
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
    pub fn as_named(&self) -> (String, DefineType) {
        match &self {
            Self::Named(s, t) => (s.clone(), t.clone()),
            _ => panic!(),
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
    scope: Scope,
    max_size: usize,
    pub symbols: Vec<Vec<(String, DefineType)>>,
    pub is_closure: bool,
    pub enclosed_symbols: Vec<Symbol>,
}

impl Context {
    fn new(scope: Scope, is_closure: bool) -> Self {
        Context {
            scope,
            max_size: 0,
            symbols: vec![Vec::new()],
            is_closure,
            enclosed_symbols: Vec::new(),
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
    fn define(&mut self, name: &str, dt: DefineType) -> Symbol {
        let current_scope = self.symbols.last_mut().unwrap();
        current_scope.push((name.to_string(), dt));
        self.max_size += 1;

        Symbol {
            index: (self.total_len() - 1).try_into().unwrap(),
            scope: self.scope,
        }
    }

    /// Resolves a symbol in this context along with its absolute index (relative to the context its top scope)
    #[inline]
    fn resolve(&self, name: &str) -> Option<(Symbol, DefineType)> {
        let mut abs_index = self.total_len();

        for scope in self.symbols.iter().rev() {
            abs_index -= scope.len();

            if let Some(index) = scope.iter().position(|n| n.0 == name) {
                return Some((
                    Symbol {
                        index: (abs_index + index).try_into().unwrap(),
                        scope: self.scope,
                    },
                    scope[index].1.clone(),
                ));
            }
        }
        None
    }

    pub fn update_dt(&mut self, name: &str, dt: DefineType) -> bool {
        let mut abs_index = self.total_len();

        for scope in self.symbols.iter_mut().rev() {
            abs_index -= scope.len();

            if let Some(index) = scope.iter().position(|n| n.0 == name) {
                scope[index].1 = dt.clone();
                return true;
            }
        }
        false
    }
}

pub enum Resolved {
    Enclosed((Symbol, DefineType)),
    Local((Symbol, DefineType)),
}

impl Resolved {
    pub fn get_type(&self) -> DefineType {
        match &self {
            Self::Enclosed((_, t)) => t.clone(),
            Self::Local((_, t)) => t.clone(),
        }
    }

    pub fn as_local(&self) -> (Symbol, DefineType) {
        match &self {
            Self::Enclosed((_, _)) => panic!(""),
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
    pub fn define(&mut self, name: &str, dt: DefineType) -> Symbol {
        self.current_context().define(name, dt)
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
                    let ind = self
                        .current_context()
                        .enclosed_symbols
                        .iter()
                        .find(|&a| a == &s.0)
                        .cloned();

                    let enclosed_symbol = if let Some(i) = ind {
                        i
                    } else {
                        self.current_context().enclosed_symbols.push(s.0.clone());
                        Symbol {
                            scope: Scope::Local,
                            index: (self.current_context().enclosed_symbols.len() - 1) as u16,
                        }
                    };
                    return Some(Resolved::Enclosed((enclosed_symbol, s.1)));
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
}
