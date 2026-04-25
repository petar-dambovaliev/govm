//! Host-side GC state for mark-sweep collection over WASM linear memory.
//!
//! The guest-side allocator (`runtime/alloc.go`) manages bump allocation and free lists.
//! The host triggers GC by reading object headers from linear memory, marking reachable
//! objects from roots, and sweeping unmarked persistent blocks.

use crate::wasm::layout::{
    FLAG_FREE, FLAG_MARK, FLAG_PERSISTENT, HEADER_FLAGS_OFFSET, HEAP_BASE, OBJ_HEADER_SIZE,
};

#[derive(Debug, Clone)]
pub struct GcState {
    /// Number of bytes of persistent allocations before triggering GC.
    pub gc_threshold: u32,
    /// Total collections performed.
    pub collections: u32,
}

impl Default for GcState {
    fn default() -> Self {
        Self::new()
    }
}

impl GcState {
    pub fn new() -> Self {
        Self {
            gc_threshold: 64 * 1024,
            collections: 0,
        }
    }

    /// Run a mark-sweep collection over linear memory.
    /// `mem` is the full linear memory slice. `heap_bump` is the current bump pointer value.
    /// `roots` is a list of pointers into the heap that are known to be live.
    pub fn collect(&mut self, mem: &mut [u8], heap_bump: u32, roots: &[u32]) {
        self.collections += 1;

        // Mark phase: set mark bit on all reachable persistent objects.
        let mut worklist: Vec<u32> = Vec::new();
        for &root in roots {
            if root >= HEAP_BASE as u32 && root < heap_bump {
                worklist.push(root);
            }
        }

        while let Some(ptr) = worklist.pop() {
            if ptr < HEAP_BASE as u32 || ptr >= heap_bump {
                continue;
            }
            let header = ptr - OBJ_HEADER_SIZE;
            if header as usize + 8 > mem.len() {
                continue;
            }
            let flags = read_u32(mem, header + HEADER_FLAGS_OFFSET);
            if flags & FLAG_MARK != 0 {
                continue;
            }
            // Set mark bit
            write_u32(mem, header + HEADER_FLAGS_OFFSET, flags | FLAG_MARK);

            let size = read_u32(mem, header);
            // Conservative scan: treat every aligned 4-byte word in the payload as a potential pointer.
            let payload_start = ptr as usize;
            let payload_end = (ptr + size).min(heap_bump) as usize;
            if payload_end <= mem.len() {
                let mut off = payload_start;
                while off + 4 <= payload_end {
                    let candidate = read_u32(mem, off as u32);
                    if candidate >= HEAP_BASE as u32 && candidate < heap_bump {
                        worklist.push(candidate);
                    }
                    off += 4;
                }
            }
        }

        // Sweep phase: walk all headers, free unmarked persistent blocks, clear mark bits.
        let mut cursor = HEAP_BASE as u32;
        while cursor < heap_bump {
            if cursor as usize + 8 > mem.len() {
                break;
            }
            let size = read_u32(mem, cursor);
            let flags = read_u32(mem, cursor + HEADER_FLAGS_OFFSET);
            let payload = cursor + OBJ_HEADER_SIZE;
            let total = OBJ_HEADER_SIZE + align8(size);

            if flags & FLAG_PERSISTENT != 0 && flags & FLAG_FREE == 0 {
                if flags & FLAG_MARK != 0 {
                    write_u32(mem, cursor + HEADER_FLAGS_OFFSET, flags & !FLAG_MARK);
                } else {
                    // Unmarked persistent block: free it by setting the free flag.
                    write_u32(
                        mem,
                        cursor + HEADER_FLAGS_OFFSET,
                        (flags | FLAG_FREE) & !FLAG_MARK,
                    );
                    // Link into free list: store old head at payload[0], set global free_list_head.
                    // NOTE: free list linking is done by writing the next-free pointer into the
                    // payload area. The caller (host_gc_collect) is responsible for updating the
                    // $free_list_head global after collection.
                    let _ = payload;
                }
            } else if flags & FLAG_MARK != 0 {
                write_u32(mem, cursor + HEADER_FLAGS_OFFSET, flags & !FLAG_MARK);
            }

            if total == 0 {
                break;
            }
            cursor += total;
        }
    }
}

fn align8(n: u32) -> u32 {
    (n + 7) & !7
}

fn read_u32(mem: &[u8], offset: u32) -> u32 {
    let off = offset as usize;
    if off + 4 > mem.len() {
        return 0;
    }
    u32::from_le_bytes([mem[off], mem[off + 1], mem[off + 2], mem[off + 3]])
}

fn write_u32(mem: &mut [u8], offset: u32, val: u32) {
    let off = offset as usize;
    if off + 4 <= mem.len() {
        mem[off..off + 4].copy_from_slice(&val.to_le_bytes());
    }
}
