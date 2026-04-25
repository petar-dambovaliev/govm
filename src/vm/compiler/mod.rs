pub(crate) mod analysis;
mod call;
pub mod compiler;
pub mod declaration;
pub mod init_order;

use crate::vm::symbols::*;

pub fn make_method_name(pkg_name: &str, dt: DefineType, f_name: &str) -> String {
    let digest = md5::compute(pkg_name);
    let p = if dt.is_struct() {
        let (name, _, _) = dt.as_struct().unwrap();
        name
    } else if dt.is_spec() {
        let (name, _, _, _) = dt.as_spec().unwrap();
        name
    } else {
        format!("{:#?}", dt)
    };
    format!("0x{:x}{:#?}{}", digest, p, f_name)
}

pub(crate) fn make_ident_name(pkg_name: &str, ident: &str) -> String {
    let digest = md5::compute(pkg_name);
    format!("0x{:x}_{}", digest, ident)
}

#[derive(Clone)]
pub enum Context {
    For(LoopContext),
}

impl Context {
    pub fn to_for(self) -> LoopContext {
        match self {
            Self::For(lc) => lc,
        }
    }

    pub fn as_for(&self) -> &LoopContext {
        match self {
            Self::For(lc) => lc,
        }
    }

    fn push_break(&mut self, _depth: u32) {
        match self {
            Self::For(f) => f.has_break = true,
        }
    }

    fn push_continue(&mut self, _depth: u32) {
        match self {
            Self::For(f) => f.has_continue = true,
        }
    }

    fn label(&self) -> Option<&String> {
        match self {
            Self::For(f) => f.label.as_ref(),
        }
    }
}

#[derive(Clone)]
pub struct LoopContext {
    pub has_break: bool,
    pub has_continue: bool,
    label: Option<String>,
    pub depth: u32,
}

impl LoopContext {
    pub fn new(label: Option<String>, depth: u32) -> Self {
        Self {
            has_break: false,
            has_continue: false,
            label,
            depth,
        }
    }
}

#[derive(Clone)]
pub(crate) struct FuncContext {
    pub ret_types: Vec<(DefineType, bool)>,
    pub expected_ret: Option<DefineType>,
    pub wasm_func_idx: u32,
}

impl FuncContext {
    pub fn new(wasm_func_idx: u32) -> Self {
        Self {
            ret_types: Vec::new(),
            expected_ret: None,
            wasm_func_idx,
        }
    }
}
