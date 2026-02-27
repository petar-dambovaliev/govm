use gno_rs::wasm::compiler::WasmCompiler;
use gno_rs::wasm::runtime::{HostState, UdfRuntime};
use wasmtime::{AsContext, AsContextMut, Func, Store, Val};

fn make_gc_string(
    store: &mut Store<HostState>,
    instance: &wasmtime::Instance,
    s: &str,
) -> Val {
    let alloc_fn = instance
        .get_typed_func::<i32, i32>(store.as_context_mut(), "alloc")
        .expect("alloc not found");
    let ptr = alloc_fn.call(store.as_context_mut(), s.len() as i32).expect("alloc failed");
    let memory = instance.get_memory(store.as_context_mut(), "memory").expect("memory not found");
    memory.write(store.as_context_mut(), ptr as usize, s.as_bytes()).expect("write failed");

    let bridge: Func = instance
        .get_func(store.as_context_mut(), "__make_gc_string")
        .expect("__make_gc_string not found");
    let mut results = vec![Val::I32(0)];
    bridge
        .call(store.as_context_mut(), &[Val::I32(ptr), Val::I32(s.len() as i32)], &mut results)
        .expect("__make_gc_string call failed");
    results.into_iter().next().unwrap()
}

fn call_with_gc_string(
    store: &mut Store<HostState>,
    instance: &wasmtime::Instance,
    func_name: &str,
    gc_str: Val,
    extra_args: &[Val],
) -> Vec<Val> {
    let func = instance
        .get_func(store.as_context_mut(), func_name)
        .expect(&format!("{} not found", func_name));
    let mut args = vec![gc_str];
    args.extend_from_slice(extra_args);
    let func_ty = func.ty(store.as_context());
    let mut results: Vec<Val> = func_ty.results().map(|_| Val::I64(0)).collect();
    func.call(store.as_context_mut(), &args, &mut results)
        .expect(&format!("{} call failed", func_name));
    results
}

#[test]
fn test_strconv_atoi_itoa() {
    let source = r#"
package main

import "strconv"

func TestAtoi(s string) int {
    n, err := strconv.Atoi(s)
    if err != nil {
        return -9999
    }
    return n
}

func TestItoa(n int) string {
    return strconv.Itoa(n)
}

func TestParseBool(s string) int32 {
    b, err := strconv.ParseBool(s)
    if err != nil {
        return -1
    }
    if b {
        return 1
    }
    return 0
}

func TestFormatBool(b bool) string {
    return strconv.FormatBool(b)
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    std::fs::write("/tmp/strconv_debug.wasm", &result.wasm_bytes).expect("write wasm");
    eprintln!("Wrote WASM to /tmp/strconv_debug.wasm ({} bytes)", result.wasm_bytes.len());

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new().with_type_layouts(result.type_layouts);
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let gc_42 = make_gc_string(&mut store, &instance, "42");
    let atoi_results = call_with_gc_string(&mut store, &instance, "TestAtoi", gc_42, &[]);
    let n = match &atoi_results[0] {
        Val::I64(v) => *v,
        Val::I32(v) => *v as i64,
        other => panic!("unexpected return type: {:?}", other),
    };
    assert_eq!(n, 42, "Atoi(\"42\")");

    let gc_neg = make_gc_string(&mut store, &instance, "-123");
    let atoi_results = call_with_gc_string(&mut store, &instance, "TestAtoi", gc_neg, &[]);
    let n = match &atoi_results[0] {
        Val::I64(v) => *v,
        Val::I32(v) => *v as i64,
        other => panic!("unexpected return type: {:?}", other),
    };
    assert_eq!(n, -123, "Atoi(\"-123\")");

    let gc_bad = make_gc_string(&mut store, &instance, "abc");
    let atoi_results = call_with_gc_string(&mut store, &instance, "TestAtoi", gc_bad, &[]);
    let n = match &atoi_results[0] {
        Val::I64(v) => *v,
        Val::I32(v) => *v as i64,
        other => panic!("unexpected return type: {:?}", other),
    };
    assert_eq!(n, -9999, "Atoi(\"abc\") should return error sentinel");
}
