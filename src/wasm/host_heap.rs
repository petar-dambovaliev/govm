//! Host-side bump allocator over the **upper half** of WASM linear memory.
//!
//! This is intentionally minimal: a Rust-owned policy (`align`, bump pointer). It does **not**
//! use WebAssembly GC proposal types.

use crate::wasm::layout::HEAP_BASE;

#[derive(Debug, Clone)]
pub struct HostHeapBump {
    /// Next allocation offset (absolute byte index into linear memory).
    next: u32,
}

impl Default for HostHeapBump {
    fn default() -> Self {
        Self::new()
    }
}

impl HostHeapBump {
    pub fn new() -> Self {
        Self {
            next: HEAP_BASE as u32,
        }
    }

    pub fn reset(&mut self) {
        self.next = HEAP_BASE as u32;
    }

    pub fn next_offset(&self) -> u32 {
        self.next
    }

    /// Bump-allocate using only the current memory **length** (no `Memory::data_mut` borrow).
    /// Call after any required `memory.grow` so `mem_len` is sufficient.
    pub fn reserve(&mut self, size: i32, mem_len: usize) -> Option<i32> {
        if size <= 0 {
            return Some(self.next as i32);
        }
        let aligned = (size as u32).saturating_add(7) & !7;
        let base = self.next;
        let new_next = base.checked_add(aligned)?;
        if (base as usize) < HEAP_BASE as usize || (new_next as usize) > mem_len {
            return None;
        }
        self.next = new_next;
        Some(base as i32)
    }

    /// Like [`Self::reserve`], but validates against an actual `&mut [u8]` (tests / tooling).
    pub fn alloc(&mut self, mem: &mut [u8], size: i32) -> i32 {
        match self.reserve(size, mem.len()) {
            Some(p) => p,
            None => -1,
        }
    }
}
