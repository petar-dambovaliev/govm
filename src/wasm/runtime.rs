//! Wasmtime embedding: host [`crate::wasm::host_heap::HostHeapBump`] services `env.rt_alloc`
//! on the upper half of linear memory (see [`crate::wasm::layout::HEAP_BASE`]).

use crate::wasm::host_heap::HostHeapBump;
use crate::wasm::layout::{HEAP_BASE, WASM_PAGE_SIZE};
use wasmtime::{Caller, Engine, Linker, Module, Store};

#[derive(Debug)]
pub struct HostState {
    pub heap: HostHeapBump,
}

fn host_rt_alloc(mut caller: Caller<'_, HostState>, size: i32) -> i32 {
    let align = if size <= 0 {
        0u32
    } else {
        (size as u32).saturating_add(7) & !7
    };
    let base = caller.data().heap.next_offset();
    let need_end = base.saturating_add(align) as usize;

    let mem = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("memory export");

    let mut current_len = mem.data_size(&caller);
    if need_end > current_len {
        let grow_by = need_end - current_len;
        let pages = (grow_by + WASM_PAGE_SIZE as usize - 1) / WASM_PAGE_SIZE as usize;
        if mem.grow(&mut caller, pages as u64).is_err() {
            return -1;
        }
        current_len = mem.data_size(&caller);
    }

    caller
        .data_mut()
        .heap
        .reserve(size, current_len)
        .unwrap_or(-1)
}

/// Loads [`crate::wasm::emit::build_smoke_module`], wires `rt_alloc`, runs export `demo`.
pub fn run_smoke_demo() -> Result<i32, String> {
    let engine = Engine::default();
    let module = Module::new(&engine, crate::wasm::emit::build_smoke_module())
        .map_err(|e| e.to_string())?;

    let mut linker = Linker::new(&engine);
    linker
        .func_wrap("env", "rt_alloc", host_rt_alloc)
        .map_err(|e| e.to_string())?;

    let mut store = Store::new(
        &engine,
        HostState {
            heap: HostHeapBump::new(),
        },
    );

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| e.to_string())?;

    let demo = instance
        .get_func(&mut store, "demo")
        .ok_or_else(|| "missing export demo".to_string())?;

    let mut results = [wasmtime::Val::I32(0)];
    demo.call(&mut store, &[], &mut results)
        .map_err(|e| e.to_string())?;

    match results[0] {
        wasmtime::Val::I32(ptr) => {
            if ptr < HEAP_BASE {
                return Err(format!("allocator returned {ptr}, expected >= {HEAP_BASE}"));
            }
            Ok(ptr)
        }
        _ => Err("demo did not return i32".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::layout;

    #[test]
    fn smoke_demo_returns_heap_pointer() {
        let ptr = run_smoke_demo().expect("demo");
        assert!(ptr >= layout::HEAP_BASE);
    }
}
