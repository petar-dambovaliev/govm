package runtime

const objHeaderSize = 8
const flagFree = 2
const flagPersistent = 4

const globalHeapBump = 1
const globalFreeListHead = 2
const globalHeapPersBytes = 3
const pageSize = 65536

func RtAlloc(size int) int {
	aligned := (size + 7) &^ 7
	total := aligned + objHeaderSize
	bump := __global_get_i32(globalHeapBump)
	__mem_store_i32(bump, aligned)
	__mem_store_i32(bump+4, 0)
	newBump := bump + total
	memBytes := __memory_size() * pageSize
	if newBump > memBytes {
		needed := (newBump - memBytes + pageSize - 1) / pageSize
		__memory_grow(needed)
	}
	__global_set_i32(globalHeapBump, newBump)
	return bump + objHeaderSize
}

func RtAllocPersistent(size int) int {
	aligned := (size + 7) &^ 7
	total := aligned + objHeaderSize

	persTop := __global_get_i32(globalHeapPersBytes)
	if persTop == 0 {
		persTop = __memory_size() * pageSize
	}

	newTop := persTop - total
	memBytes := __memory_size() * pageSize
	if newTop < memBytes/2 {
		__memory_grow(1)
		persTop = __memory_size() * pageSize
		newTop = persTop - total
	}

	__mem_store_i32(newTop, aligned)
	__mem_store_i32(newTop+4, flagPersistent)
	__global_set_i32(globalHeapPersBytes, newTop)
	return newTop + objHeaderSize
}

func RtFree(ptr int) {
	header := ptr - objHeaderSize
	flags := __mem_load_i32(header + 4)
	__mem_store_i32(header+4, flags|flagFree)
	head := __global_get_i32(globalFreeListHead)
	__mem_store_i32(ptr, head)
	__global_set_i32(globalFreeListHead, ptr)
}

func RtWatermark() int {
	return __global_get_i32(globalHeapBump)
}

func RtScopeReset(wm int) {
	__global_set_i32(globalHeapBump, wm)
}
