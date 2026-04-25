pub(crate) mod analysis;
mod call;
pub mod compiler;
pub mod declaration;
pub mod init_order;

use crate::vm::compiler::compiler::MemVar;
use crate::vm::symbols::*;
use ahash::AHashMap;
use std::collections::HashSet;

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
    Switch(SwitchContext),
    For(LoopContext),
}

impl Context {
    pub fn to_for(self) -> LoopContext {
        match self {
            Self::For(lc) => lc,
            _ => panic!("expected For context"),
        }
    }

    pub fn as_for(&self) -> &LoopContext {
        match self {
            Self::For(lc) => lc,
            _ => panic!("expected For context"),
        }
    }

    pub fn to_switch(self) -> SwitchContext {
        match self {
            Self::Switch(sc) => sc,
            _ => panic!("expected Switch context"),
        }
    }

    fn push_break(&mut self, _depth: u32) {
        match self {
            Self::Switch(sw) => sw.has_break = true,
            Self::For(f) => f.has_break = true,
        }
    }

    fn push_continue(&mut self, _depth: u32) {
        match self {
            Self::Switch(_) => panic!("no continue on a switch"),
            Self::For(f) => f.has_continue = true,
        }
    }

    fn label(&self) -> Option<&String> {
        match self {
            Self::Switch(sw) => sw.label.as_ref(),
            Self::For(f) => f.label.as_ref(),
        }
    }
}

#[derive(Clone)]
pub struct SwitchContext {
    pub has_break: bool,
    label: Option<String>,
    pub depth: u32,
}

impl SwitchContext {
    pub fn new(label: Option<String>, depth: u32) -> Self {
        Self {
            has_break: false,
            label,
            depth,
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
    /// Symbol.index -> WASM local base index (strings occupy base and base+1).
    pub locals: AHashMap<u16, u32>,
    pub next_wasm_local: u32,
    /// WASM local holding the saved `$sp` for the current function.
    pub saved_sp_local: Option<u32>,
    /// Address-taken variables living in linear memory for the current function.
    pub mem_vars: AHashMap<String, MemVar>,
    /// WASM local holding the stack frame base pointer for the current function.
    pub frame_base_local: Option<u32>,
    /// Names of address-taken variables that escape and need heap allocation at declaration time.
    pub escaped_vars: HashSet<String>,
    /// WASM local holding the saved heap watermark for scope-based freeing.
    pub saved_heap_wm_local: Option<u32>,
}

impl FuncContext {
    pub fn new(wasm_func_idx: u32) -> Self {
        Self {
            ret_types: Vec::new(),
            expected_ret: None,
            wasm_func_idx,
            locals: AHashMap::new(),
            next_wasm_local: 0,
            saved_sp_local: None,
            mem_vars: AHashMap::new(),
            frame_base_local: None,
            escaped_vars: HashSet::new(),
            saved_heap_wm_local: None,
        }
    }
}
