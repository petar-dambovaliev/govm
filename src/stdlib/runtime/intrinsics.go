package runtime

func __mem_load_i32(addr int) int

func __mem_store_i32(addr int, val int)

func __mem_load_i64(addr int) int

func __mem_store_i64(addr int, val int)

func __memory_size() int

func __memory_grow(pages int) int

func __global_get_i32(idx int) int

func __global_set_i32(idx int, val int)
