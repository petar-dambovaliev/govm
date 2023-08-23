use crate::vm::object::Type;

pub(crate) struct SymbolTable {
    /// A vector of contexts
    /// The context at index 0 will always be the global context,
    /// any context that follows is a local (to a function) context.
    /// There can be more than one local context as functions can be nested inside other functions.
    contexts: Vec<Context>,
}

pub(crate) struct Symbol {
    pub scope: Scope,
    pub index: u16,
}

#[derive(PartialEq, Copy, Clone)]
pub(crate) enum Scope {
    Local,
    Global,
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
pub enum ContextType {
    //    key,    identifier,  type
    Named(String, String, DefineType),
    Unnamed(DefineType),
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
pub enum DefineType {
    Null,
    Var(Box<Self>),
    Struct(String, Vec<ContextType>),
    Func(String, Vec<ContextType>, Vec<ContextType>),
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

    pub fn is_func(&self) -> bool {
        match &self {
            Self::Func(_, _, _) => true,
            _ => false,
        }
    }

    pub fn as_type(&self) -> (DefineType, Type) {
        match &self {
            Self::Type(df, t) => (*df.clone(), t.clone()),
            _ => panic!("expected Self::Type"),
        }
    }
}

impl ContextType {
    pub fn as_named(&self) -> (String, String, DefineType) {
        match &self {
            Self::Named(s, s1, t) => (s.clone(), s1.clone(), t.clone()),
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
pub(crate) struct Context {
    scope: Scope,
    max_size: usize,
    pub symbols: Vec<Vec<(String, DefineType)>>,
}

impl Context {
    fn new(scope: Scope) -> Self {
        Context {
            scope,
            max_size: 0,
            symbols: vec![Vec::new()],
        }
    }

    /// The maximum number of symbols defined in this context.
    /// Not all of these symbols may still be in scope once this context is destroyed.
    fn max_size(&self) -> usize {
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

impl SymbolTable {
    /// Creates a new symbol table with a globally scoped context
    pub fn new() -> Self {
        SymbolTable {
            contexts: vec![Context::new(Scope::Global)],
        }
    }

    /// Returns a mutable reference to the current context
    pub fn current_context(&mut self) -> &mut Context {
        self.contexts.last_mut().unwrap()
    }

    /// Create a new context to define symbols in.
    /// This will always be a local context (as there is only one global context).
    pub fn new_context(&mut self) {
        self.contexts.push(Context::new(Scope::Local));
    }

    /// Destroys the current context and returns the maximum number of symbols it had at some point in time.
    pub fn leave_context(&mut self) -> usize {
        self.contexts.pop().unwrap().max_size()
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

    /// Resolve a symbol in either the current context or the global context if no local was found.
    pub fn resolve(&mut self, name: &str) -> Option<(Symbol, DefineType)> {
        let symbol = self.current_context().resolve(name);
        if symbol.is_some() {
            return symbol;
        }

        if self.contexts.len() > 1 {
            self.contexts[0].resolve(name)
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
