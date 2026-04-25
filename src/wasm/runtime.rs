//! Wasmtime embedding: the guest-side allocator in `runtime/alloc.go` manages
//! the heap region of linear memory (see [`crate::wasm::layout::HEAP_BASE`]).
//! The host provides `print_string`, `println_string`, and `rt_gc_collect` imports.

use crate::wasm::host_heap::GcState;
use crate::wasm::layout::GC_ROOT_TABLE_BASE;
use wasmtime::{Caller, Engine, Instance, Linker, Module, Store};

#[derive(Debug)]
pub struct HostState {
    pub gc: GcState,
    pub output: Vec<u8>,
}

fn host_print_string(mut caller: Caller<'_, HostState>, ptr: i32, len: i32) {
    if len <= 0 {
        return;
    }
    let mem = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("memory export");
    let data = mem.data(&caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end <= data.len() {
        let bytes = data[start..end].to_vec();
        use std::io::Write;
        let _ = std::io::stdout().write_all(&bytes);
        let _ = std::io::stdout().flush();
        caller.data_mut().output.extend_from_slice(&bytes);
    }
}

fn host_println_string(mut caller: Caller<'_, HostState>, ptr: i32, len: i32) {
    let mem = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("memory export");
    let data = mem.data(&caller);
    let mut bytes = Vec::new();
    if len > 0 {
        let start = ptr as usize;
        let end = start + len as usize;
        if end <= data.len() {
            bytes.extend_from_slice(&data[start..end]);
        }
    }
    bytes.push(b'\n');
    use std::io::Write;
    let _ = std::io::stdout().write_all(&bytes);
    let _ = std::io::stdout().flush();
    caller.data_mut().output.extend_from_slice(&bytes);
}

fn host_print_int(mut caller: Caller<'_, HostState>, val: i32) {
    let s = format!("{}", val);
    use std::io::Write;
    let _ = std::io::stdout().write_all(s.as_bytes());
    let _ = std::io::stdout().flush();
    caller.data_mut().output.extend_from_slice(s.as_bytes());
}

fn host_print_bool(mut caller: Caller<'_, HostState>, val: i32) {
    let s = if val != 0 { "true" } else { "false" };
    use std::io::Write;
    let _ = std::io::stdout().write_all(s.as_bytes());
    let _ = std::io::stdout().flush();
    caller.data_mut().output.extend_from_slice(s.as_bytes());
}

fn host_print_newline(mut caller: Caller<'_, HostState>) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(b"\n");
    let _ = std::io::stdout().flush();
    caller.data_mut().output.push(b'\n');
}

fn host_print_space(mut caller: Caller<'_, HostState>) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(b" ");
    let _ = std::io::stdout().flush();
    caller.data_mut().output.push(b' ');
}

fn host_gc_collect(mut caller: Caller<'_, HostState>) {
    let mem = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("memory export");

    let heap_bump_global = caller
        .get_export("__heap_bump")
        .and_then(|e| e.into_global());

    let heap_bump = match heap_bump_global {
        Some(g) => g.get(&mut caller).i32().unwrap_or(0) as u32,
        None => return,
    };

    let sp_global = caller
        .get_export("__sp")
        .and_then(|e| e.into_global());
    let sp = sp_global.map_or(crate::wasm::layout::STACK_TOP, |g| {
        g.get(&mut caller).i32().unwrap_or(crate::wasm::layout::STACK_TOP as i32) as u32
    });

    // Read roots from the GC root table + conservative stack scan.
    let roots = {
        let data = mem.data(&caller);
        let mut roots = Vec::new();

        // Explicit root table at start of linear memory.
        let root_count_off = GC_ROOT_TABLE_BASE as usize;
        if root_count_off + 4 <= data.len() {
            let count = u32::from_le_bytes([
                data[root_count_off],
                data[root_count_off + 1],
                data[root_count_off + 2],
                data[root_count_off + 3],
            ]);
            for i in 0..count.min(crate::wasm::layout::GC_ROOT_TABLE_MAX) {
                let off = (GC_ROOT_TABLE_BASE + 4 + i * 4) as usize;
                if off + 4 <= data.len() {
                    let ptr = u32::from_le_bytes([
                        data[off],
                        data[off + 1],
                        data[off + 2],
                        data[off + 3],
                    ]);
                    if ptr != 0 {
                        roots.push(ptr);
                    }
                }
            }
        }

        // Conservative stack scan: every aligned word in [sp, STACK_TOP] that
        // looks like a heap pointer is treated as a root.
        let stack_lo = sp as usize;
        let stack_hi = crate::wasm::layout::STACK_TOP as usize;
        if stack_lo < stack_hi && stack_hi <= data.len() {
            let heap_base = crate::wasm::layout::HEAP_BASE as u32;
            let mut off = stack_lo;
            while off + 4 <= stack_hi {
                let candidate = u32::from_le_bytes([
                    data[off],
                    data[off + 1],
                    data[off + 2],
                    data[off + 3],
                ]);
                if candidate >= heap_base && candidate < heap_bump {
                    roots.push(candidate);
                }
                off += 4;
            }
        }

        roots
    };

    let (mem_data, store) = mem.data_and_store_mut(&mut caller);
    store.gc.collect(mem_data, heap_bump, &roots);
}

fn setup_linker(linker: &mut Linker<HostState>) -> Result<(), String> {
    linker
        .func_wrap("env", "print_string", host_print_string)
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap("env", "println_string", host_println_string)
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap("env", "print_int", host_print_int)
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap("env", "print_bool", host_print_bool)
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap("env", "print_newline", host_print_newline)
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap("env", "print_space", host_print_space)
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap("env", "rt_gc_collect", host_gc_collect)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Loads [`crate::wasm::emit::build_smoke_module`], runs export `demo`.
pub fn run_smoke_demo() -> Result<i32, String> {
    let engine = Engine::default();
    let module = Module::new(&engine, crate::wasm::emit::build_smoke_module())
        .map_err(|e| e.to_string())?;

    let mut store = Store::new(
        &engine,
        HostState {
            gc: GcState::new(),
            output: Vec::new(),
        },
    );

    let instance = Instance::new(&mut store, &module, &[])
        .map_err(|e| e.to_string())?;

    let demo = instance
        .get_func(&mut store, "demo")
        .ok_or_else(|| "missing export demo".to_string())?;

    let mut results = [wasmtime::Val::I32(0)];
    demo.call(&mut store, &[], &mut results)
        .map_err(|e| e.to_string())?;

    match results[0] {
        wasmtime::Val::I32(ptr) => Ok(ptr),
        _ => Err("demo did not return i32".to_string()),
    }
}

/// Compile Go source code and return WASM bytes.
fn compile_go_to_wasm(go_source: &str) -> Result<Vec<u8>, String> {
    use std::io::Write;

    let unique_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let tid = std::thread::current().id();
    let tmp_dir = std::env::temp_dir().join(format!("govm_test_{:?}_{}", tid, unique_id));
    std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
    let go_file = tmp_dir.join("main.go");
    {
        let mut f = std::fs::File::create(&go_file).map_err(|e| e.to_string())?;
        f.write_all(go_source.as_bytes()).map_err(|e| e.to_string())?;
    }

    let pkgs = crate::vm::module::parse_local_dependencies(&go_file)
        .map_err(|e| format!("parse error: {}", e))?;

    let mut goc = crate::vm::compiler::compiler::Compiler::new();
    let main_path = go_file.clone();
    let project_path = tmp_dir.clone();

    goc.compile(main_path, project_path, pkgs, false)
        .map_err(|e| format!("compile error: {}", e))
}

/// Compile Go source code to WASM and run the `main` function through Wasmtime.
/// Returns the i32 result if main returns one, or Ok(0) for void main.
pub fn compile_and_run_go(go_source: &str) -> Result<i32, String> {
    let wasm_bytes = compile_go_to_wasm(go_source)?;
    run_wasm_main(&wasm_bytes).map(|(code, _)| code)
}

/// Compile Go source code to WASM, run `main`, and return both exit code and captured output.
pub fn compile_and_run_go_with_output(go_source: &str) -> Result<(i32, String), String> {
    let wasm_bytes = compile_go_to_wasm(go_source)?;
    let (code, output) = run_wasm_main(&wasm_bytes)?;
    Ok((code, String::from_utf8_lossy(&output).to_string()))
}

/// Run a compiled WASM module, calling its `main` export.
/// Returns (i32 result, captured output bytes).
pub fn run_wasm_main(wasm_bytes: &[u8]) -> Result<(i32, Vec<u8>), String> {
    let engine = Engine::default();
    let module = Module::new(&engine, wasm_bytes).map_err(|e| e.to_string())?;

    let mut linker = Linker::new(&engine);
    setup_linker(&mut linker)?;

    let mut store = Store::new(
        &engine,
        HostState {
            gc: GcState::new(),
            output: Vec::new(),
        },
    );

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| e.to_string())?;

    let main_fn = instance
        .get_func(&mut store, "main")
        .ok_or_else(|| "missing export 'main'".to_string())?;

    let ty = main_fn.ty(&store);
    let num_results = ty.results().len();

    let code = if num_results == 0 {
        main_fn
            .call(&mut store, &[], &mut [])
            .map_err(|e| e.to_string())?;
        0
    } else {
        let mut results = vec![wasmtime::Val::I32(0); num_results];
        main_fn
            .call(&mut store, &[], &mut results)
            .map_err(|e| e.to_string())?;
        match results[0] {
            wasmtime::Val::I32(v) => v,
            _ => 0,
        }
    };

    let output = store.data().output.clone();
    Ok((code, output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::layout;

    #[test]
    fn smoke_demo_returns_heap_pointer() {
        let ptr = run_smoke_demo().expect("demo");
        assert_eq!(ptr, layout::HEAP_BASE);
    }

    #[test]
    fn go_return_literal() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    return 42
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 42);
    }

    #[test]
    fn go_arithmetic() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    return 2 + 3 * 4
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 14);
    }

    #[test]
    fn go_local_variables() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    x := 10
    y := 20
    return x + y
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 30);
    }

    #[test]
    fn go_if_else() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    x := 5
    if x > 3 {
        return 1
    } else {
        return 0
    }
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 1);
    }

    #[test]
    fn go_for_loop() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    sum := 0
    for i := 0; i < 10; i++ {
        sum = sum + i
    }
    return sum
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 45);
    }

    #[test]
    fn go_function_call() {
        let result = compile_and_run_go(
            r#"
package main

func add(a int, b int) int {
    return a + b
}

func main() int {
    return add(3, 7)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 10);
    }

    #[test]
    fn go_nested_if() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    x := 10
    if x > 5 {
        if x > 8 {
            return 100
        } else {
            return 50
        }
    }
    return 0
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 100);
    }

    #[test]
    fn go_break_continue() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    sum := 0
    for i := 0; i < 20; i++ {
        if i == 10 {
            break
        }
        if i % 2 == 0 {
            continue
        }
        sum = sum + i
    }
    return sum
}
"#,
        )
        .expect("should compile and run");
        // odd numbers less than 10: 1+3+5+7+9 = 25
        assert_eq!(result, 25);
    }

    #[test]
    fn go_unary_negate() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    x := 5
    return -x
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, -5);
    }

    #[test]
    fn go_boolean_logic() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    a := true
    b := false
    if a && !b {
        return 1
    }
    return 0
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 1);
    }

    #[test]
    fn go_fibonacci() {
        let result = compile_and_run_go(
            r#"
package main

func fib(n int) int {
    if n <= 1 {
        return n
    }
    return fib(n-1) + fib(n-2)
}

func main() int {
    return fib(10)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 55);
    }

    #[test]
    fn go_string_literal_len() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    s := "hello"
    return len(s)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 5);
    }

    #[test]
    fn go_string_concat_len() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    s := "hello" + " " + "world"
    return len(s)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 11);
    }

    #[test]
    fn go_string_println() {
        let (code, output) = compile_and_run_go_with_output(
            r#"
package main

func main() {
    println("hello")
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(code, 0);
        assert_eq!(output, "hello\n");
    }

    #[test]
    fn go_string_variable() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    a := "foo"
    b := a
    return len(b)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 3);
    }

    #[test]
    fn go_string_concat_println() {
        let (code, output) = compile_and_run_go_with_output(
            r#"
package main

func main() {
    a := "hello"
    b := " world"
    println(a + b)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(code, 0);
        assert_eq!(output, "hello world\n");
    }

    #[test]
    fn go_struct_literal() {
        let result = compile_and_run_go(
            r#"
package main

type Point struct {
    x int
    y int
}

func main() int {
    p := Point{x: 3, y: 7}
    return p.x + p.y
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 10);
    }

    #[test]
    fn go_struct_field_set() {
        let result = compile_and_run_go(
            r#"
package main

type Point struct {
    x int
    y int
}

func main() int {
    p := Point{x: 1, y: 2}
    p.x = 10
    return p.x + p.y
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 12);
    }

    #[test]
    fn go_struct_method() {
        let result = compile_and_run_go(
            r#"
package main

type Rect struct {
    w int
    h int
}

func (r Rect) Area() int {
    return r.w * r.h
}

func main() int {
    rect := Rect{w: 5, h: 3}
    return rect.Area()
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 15);
    }

    #[test]
    fn go_struct_string_field() {
        let (code, output) = compile_and_run_go_with_output(
            r#"
package main

type Greeting struct {
    msg string
}

func main() {
    g := Greeting{msg: "hi there"}
    println(g.msg)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(code, 0);
        assert_eq!(output, "hi there\n");
    }

    #[test]
    fn go_struct_nested() {
        let result = compile_and_run_go(
            r#"
package main

type Inner struct {
    val int
}

type Outer struct {
    inner Inner
    extra int
}

func main() int {
    o := Outer{inner: Inner{val: 42}, extra: 8}
    return o.inner.val + o.extra
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 50);
    }

    #[test]
    fn go_closure_capture() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    n := 10
    f := func(x int) int {
        return x + n
    }
    return f(5)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 15);
    }

    #[test]
    fn go_closure_multiple_captures() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    a := 3
    b := 7
    f := func(x int) int {
        return x + a + b
    }
    return f(10)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 20);
    }

    #[test]
    fn go_closure_no_captures() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    f := func(x int) int {
        return x * 2
    }
    return f(21)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 42);
    }

    #[test]
    fn go_slice_literal() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    s := []int{10, 20, 30}
    return s[1]
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 20);
    }

    #[test]
    fn go_slice_index_set() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    s := []int{0, 0}
    s[1] = 42
    return s[1]
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 42);
    }

    #[test]
    fn go_slice_len() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    s := []int{1, 2, 3}
    return len(s)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 3);
    }

    #[test]
    fn go_slice_append() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    s := []int{1}
    s = append(s, 2, 3)
    return s[2]
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 3);
    }

    #[test]
    fn go_slice_range() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    s := []int{10, 20, 30}
    sum := 0
    for _, v := range s {
        sum += v
    }
    return sum
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 60);
    }

    #[test]
    fn go_slice_range_index() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    s := []int{10, 20, 30}
    sum := 0
    for i, _ := range s {
        sum += i
    }
    return sum
}
"#,
        )
        .expect("should compile and run");
        // 0 + 1 + 2 = 3
        assert_eq!(result, 3);
    }

    #[test]
    fn go_slice_expression() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    s := []int{1, 2, 3, 4}
    t := s[1:3]
    return t[0] + t[1]
}
"#,
        )
        .expect("should compile and run");
        // t = [2, 3], so 2 + 3 = 5
        assert_eq!(result, 5);
    }

    #[test]
    fn go_make_slice() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    s := make([]int, 3)
    s[0] = 7
    return s[0]
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 7);
    }

    #[test]
    fn go_slice_string_elems() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    s := []string{"a", "b"}
    return len(s)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 2);
    }

    #[test]
    fn go_array_literal() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    a := [3]int{10, 20, 30}
    return a[1]
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 20);
    }

    #[test]
    fn go_array_index_set() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    a := [3]int{0, 0, 0}
    a[1] = 42
    return a[1]
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 42);
    }

    #[test]
    fn go_array_len() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    a := [3]int{1, 2, 3}
    return len(a)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 3);
    }

    #[test]
    fn go_array_range() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    a := [3]int{10, 20, 30}
    sum := 0
    for _, v := range a {
        sum += v
    }
    return sum
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 60);
    }

    #[test]
    fn go_array_range_index() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    a := [3]int{10, 20, 30}
    sum := 0
    for i, _ := range a {
        sum += i
    }
    return sum
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 3);
    }

    #[test]
    fn go_array_in_loop() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    sum := 0
    for i := 0; i < 100; i++ {
        a := [3]int{i, i + 1, i + 2}
        sum += a[0]
    }
    return sum
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 4950);
    }

    #[test]
    fn go_compound_assign_ops() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    x := 100
    x += 10
    x -= 20
    x *= 3
    x /= 9
    x %= 7
    return x
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 2);
    }

    #[test]
    fn go_incdec_ident() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    x := 10
    x++
    x++
    x--
    return x
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 11);
    }

    #[test]
    fn go_incdec_index() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    a := [3]int{10, 20, 30}
    a[1]++
    a[1]++
    a[2]--
    return a[0] + a[1] + a[2]
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 10 + 22 + 29);
    }

    #[test]
    fn go_incdec_selector() {
        let result = compile_and_run_go(
            r#"
package main

type Point struct {
    x int
    y int
}

func main() int {
    p := Point{x: 5, y: 10}
    p.x++
    p.y--
    return p.x + p.y
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 6 + 9);
    }

    #[test]
    fn go_short_circuit_and() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    counter := 0
    x := false && counter > 0
    _ = x
    return counter
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 0);
    }

    #[test]
    fn go_short_circuit_or() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    a := 1
    b := 0
    if a > 0 || b > 0 {
        return 1
    }
    return 0
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 1);
    }

    #[test]
    fn go_nil_comparison() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    if nil == nil {
        return 1
    }
    return 0
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 1);
    }

    #[test]
    fn go_nil_literal_value() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    x := 0
    if nil == nil {
        x = 1
    }
    return x
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 1);
    }

    #[test]
    fn go_short_circuit_nil_guard() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    if false && nil == nil {
        return 0
    }
    return 1
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 1);
    }

    #[test]
    fn go_pointer_basic() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    x := 42
    p := &x
    return *p
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 42);
    }

    #[test]
    fn go_pointer_write() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    x := 1
    p := &x
    *p = 2
    return x
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 2);
    }

    #[test]
    fn go_pointer_swap() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    a := 10
    b := 20
    pa := &a
    pb := &b
    *pa = *pb
    return a
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 20);
    }

    #[test]
    fn go_pointer_nil_deref() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    var p *int
    return *p
}
"#,
        );
        assert!(result.is_err(), "dereferencing nil pointer should trap");
    }

    #[test]
    fn go_pointer_escape_return() {
        let result = compile_and_run_go(
            r#"
package main

func newInt(v int) *int {
    x := v
    return &x
}

func main() int {
    p := newInt(99)
    return *p
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 99);
    }

    #[test]
    fn go_pointer_pass_to_function() {
        let result = compile_and_run_go(
            r#"
package main

func setVal(p *int, v int) {
    *p = v
}

func main() int {
    x := 0
    setVal(&x, 77)
    return x
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 77);
    }

    #[test]
    fn go_pointer_multiple_independent() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    a := 1
    b := 2
    c := 3
    pa := &a
    pb := &b
    pc := &c
    *pa = *pa + *pb + *pc
    return a
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 6);
    }

    #[test]
    fn go_pointer_in_loop() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    sum := 0
    p := &sum
    for i := 0; i < 5; i++ {
        *p = *p + i
    }
    return sum
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 10);
    }

    #[test]
    fn go_pointer_reassign() {
        let result = compile_and_run_go(
            r#"
package main

func main() int {
    a := 10
    b := 20
    p := &a
    *p = 100
    p = &b
    *p = 200
    return a + b
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 300);
    }

    #[test]
    fn go_multi_return_basic() {
        let result = compile_and_run_go(
            r#"
package main

func swap(a int, b int) (int, int) {
    return b, a
}

func main() int {
    x, y := swap(1, 2)
    return x*10 + y
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 21);
    }

    #[test]
    fn go_multi_return_discard() {
        let result = compile_and_run_go(
            r#"
package main

func pair() (int, int) {
    return 7, 42
}

func main() int {
    _, b := pair()
    return b
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 42);
    }

    #[test]
    fn go_multi_return_assign() {
        let result = compile_and_run_go(
            r#"
package main

func pair() (int, int) {
    return 3, 5
}

func main() int {
    a := 0
    b := 0
    a, b = pair()
    return a*10 + b
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 35);
    }

    #[test]
    fn go_multi_return_three() {
        let result = compile_and_run_go(
            r#"
package main

func triple(x int) (int, int, int) {
    return x, x*2, x*3
}

func main() int {
    a, b, c := triple(10)
    return a + b + c
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 60);
    }

    #[test]
    fn go_multi_return_discard_first() {
        let result = compile_and_run_go(
            r#"
package main

func pair() (int, int) {
    return 99, 11
}

func main() int {
    a, _ := pair()
    return a
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 99);
    }

    // ── Interface tests ──

    #[test]
    fn go_interface_method_call() {
        let result = compile_and_run_go(
            r#"
package main

type Valuer interface {
    Value() int
}

type Num struct {
    n int
}

func (x Num) Value() int {
    return x.n
}

func main() int {
    var v Valuer = Num{n: 42}
    return v.Value()
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 42);
    }

    #[test]
    fn go_interface_multiple_types() {
        let result = compile_and_run_go(
            r#"
package main

type Valuer interface {
    Value() int
}

type A struct { x int }
type B struct { y int }

func (a A) Value() int { return a.x }
func (b B) Value() int { return b.y * 10 }

func get(v Valuer) int {
    return v.Value()
}

func main() int {
    var v1 Valuer = A{x: 3}
    var v2 Valuer = B{y: 4}
    return get(v1) + get(v2)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 43);
    }

    #[test]
    fn go_interface_type_assert() {
        let result = compile_and_run_go(
            r#"
package main

type Valuer interface {
    Value() int
}

type Num struct { n int }

func (x Num) Value() int { return x.n }

func main() int {
    var v Valuer = Num{n: 7}
    n := v.(Num)
    return n.n
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 7);
    }

    #[test]
    fn go_interface_type_assert_comma_ok() {
        let result = compile_and_run_go(
            r#"
package main

type Valuer interface {
    Value() int
}

type Num struct { n int }
type Other struct { x int }

func (x Num) Value() int { return x.n }
func (x Other) Value() int { return x.x }

func main() int {
    var v Valuer = Num{n: 5}
    _, ok := v.(Num)
    if ok {
        return 1
    }
    return 0
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 1);
    }

    #[test]
    fn go_interface_type_assert_comma_ok_fail() {
        let result = compile_and_run_go(
            r#"
package main

type Valuer interface {
    Value() int
}

type Num struct { n int }
type Other struct { x int }

func (x Num) Value() int { return x.n }
func (x Other) Value() int { return x.x }

func main() int {
    var v Valuer = Other{x: 9}
    _, ok := v.(Num)
    if ok {
        return 1
    }
    return 0
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 0);
    }

    #[test]
    fn go_interface_nil() {
        let result = compile_and_run_go(
            r#"
package main

type Valuer interface {
    Value() int
}

func main() int {
    var v Valuer
    if v == nil {
        return 1
    }
    return 0
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 1);
    }

    #[test]
    fn go_interface_type_switch() {
        let result = compile_and_run_go(
            r#"
package main

type Valuer interface {
    Value() int
}

type A struct { x int }
type B struct { y int }

func (a A) Value() int { return a.x }
func (b B) Value() int { return b.y }

func classify(v Valuer) int {
    switch v.(type) {
    case A:
        return 1
    case B:
        return 2
    }
    return 0
}

func main() int {
    var v1 Valuer = A{x: 10}
    var v2 Valuer = B{y: 20}
    return classify(v1)*10 + classify(v2)
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 12);
    }

    // ──── stdlib import tests ────

    #[test]
    fn go_stdlib_const_access() {
        let result = compile_and_run_go(
            r#"
package main

import "runtime"

func main() int {
    return runtime.GOOS
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 1);
    }

    // ──── GC and scope-based freeing tests ────

    #[test]
    fn go_scope_free_basic() {
        // Non-escaping allocations inside a called function should be freed when
        // it returns (watermark reset). The main function's own allocations should
        // still be valid.
        let result = compile_and_run_go(
            r#"
package main

func allocator() int {
    s := "temporary"
    return len(s)
}

func main() int {
    a := allocator()
    b := allocator()
    return a + b
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 18);
    }

    #[test]
    fn go_scope_free_loop() {
        // Calling an allocating function in a loop: each call's ephemeral
        // allocations are freed on return, preventing unbounded heap growth.
        let result = compile_and_run_go(
            r#"
package main

func work() int {
    s := "hello"
    return len(s)
}

func main() int {
    total := 0
    i := 0
    for i < 100 {
        total = total + work()
        i = i + 1
    }
    return total
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 500);
    }

    #[test]
    fn go_persistent_escape() {
        // A pointer that escapes via return should use persistent allocation
        // and survive the callee's scope reset.
        let result = compile_and_run_go(
            r#"
package main

func make_val() *int {
    x := 42
    return &x
}

func main() int {
    p := make_val()
    return *p
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 42);
    }

    #[test]
    fn go_gc_collect_basic() {
        // Allocate many strings in a loop and verify the program doesn't crash
        // from memory exhaustion (scope-based freeing reclaims per-call allocations).
        let result = compile_and_run_go(
            r#"
package main

func make_string() int {
    s := "abcdefghijklmnop"
    return len(s)
}

func main() int {
    total := 0
    i := 0
    for i < 1000 {
        total = total + make_string()
        i = i + 1
    }
    return total
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 16000);
    }

    #[test]
    fn go_gc_preserve_live_data() {
        // Verify that persistent allocations (escaped pointers) remain valid
        // even after many ephemeral allocations.
        let result = compile_and_run_go(
            r#"
package main

func noise() int {
    s := "noise"
    return len(s)
}

func make_ptr() *int {
    x := 99
    return &x
}

func main() int {
    p := make_ptr()
    i := 0
    for i < 50 {
        noise()
        i = i + 1
    }
    return *p
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 99);
    }

    #[test]
    fn go_gc_stress() {
        // Stress test: many loop iterations, each calling a function that does
        // both ephemeral and persistent allocations.
        let result = compile_and_run_go(
            r#"
package main

func alloc_temp() int {
    s := "temp"
    return len(s)
}

func main() int {
    sum := 0
    i := 0
    for i < 5000 {
        sum = sum + alloc_temp()
        i = i + 1
    }
    return sum
}
"#,
        )
        .expect("should compile and run");
        assert_eq!(result, 20000);
    }
}
