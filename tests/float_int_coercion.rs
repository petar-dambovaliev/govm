use gno_rs::wasm::compiler::WasmCompiler;
use gno_rs::wasm::runtime::{HostState, UdfRuntime};

fn compile_and_instantiate(
    source: &str,
) -> (
    wasmtime::Store<HostState>,
    wasmtime::Instance,
) {
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    (store, instance)
}

#[test]
fn test_float_lit_modulo_int64() {
    let source = r#"
package main

func F(d int64) int64 {
    return d % 1e9
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i64>(&mut store, "F")
        .expect("F not found");
    assert_eq!(f.call(&mut store, 2_500_000_000).expect("call failed"), 500_000_000);
    assert_eq!(f.call(&mut store, 1_000_000_000).expect("call failed"), 0);
}

#[test]
fn test_float_lit_division_int64() {
    let source = r#"
package main

func F(d int64) int64 {
    return d / 1e9
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i64>(&mut store, "F")
        .expect("F not found");
    assert_eq!(f.call(&mut store, 3_000_000_000).expect("call failed"), 3);
    assert_eq!(f.call(&mut store, 500_000_000).expect("call failed"), 0);
}

#[test]
fn test_float_lit_multiplication_int64() {
    let source = r#"
package main

func F(s int64) int64 {
    return s * 1e9
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i64>(&mut store, "F")
        .expect("F not found");
    assert_eq!(f.call(&mut store, 5).expect("call failed"), 5_000_000_000);
}

#[test]
fn test_float_lit_comparison_int64() {
    let source = r#"
package main

func F(n int64) bool {
    return n >= 1e9
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i32>(&mut store, "F")
        .expect("F not found");
    assert_eq!(f.call(&mut store, 1_000_000_000).expect("call failed"), 1);
    assert_eq!(f.call(&mut store, 999_999_999).expect("call failed"), 0);
    assert_eq!(f.call(&mut store, 2_000_000_000).expect("call failed"), 1);
}

#[test]
fn test_float_lit_sub_assign() {
    let source = r#"
package main

func F(n int64) int64 {
    n -= 1e9
    return n
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i64>(&mut store, "F")
        .expect("F not found");
    assert_eq!(f.call(&mut store, 2_000_000_000).expect("call failed"), 1_000_000_000);
}

#[test]
fn test_float_lit_add_assign() {
    let source = r#"
package main

func F(n int64) int64 {
    n += 1e9
    return n
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i64>(&mut store, "F")
        .expect("F not found");
    assert_eq!(f.call(&mut store, 500_000_000).expect("call failed"), 1_500_000_000);
}

#[test]
fn test_float_lit_func_arg() {
    let source = r#"
package main

func norm(hi int, lo int, base int) int {
    return base
}

func F() int {
    return norm(0, 0, 1e9)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .expect("F not found");
    assert_eq!(f.call(&mut store, ()).expect("call failed"), 1_000_000_000);
}

#[test]
fn test_float_lit_type_cast_modulo() {
    let source = r#"
package main

func F(d int64) int32 {
    return int32(d % 1e9)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i32>(&mut store, "F")
        .expect("F not found");
    assert_eq!(f.call(&mut store, 2_500_000_000).expect("call failed"), 500_000_000);
}

#[test]
fn test_float_lit_division_uint64() {
    let source = r#"
package main

func F(m uint64) uint64 {
    return m / 1e9
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i64>(&mut store, "F")
        .expect("F not found");
    // 5_000_000_000 as uint64 / 1e9 = 5
    assert_eq!(f.call(&mut store, 5_000_000_000).expect("call failed"), 5);
}

// ==================== Root Cause 1: intSize / unsigned const eval ====================

#[test]
fn test_unsigned_const_shift() {
    let source = r#"
package main

const intSize = 32 << (^uint(0) >> 63)

func F() int {
    return intSize
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .expect("F not found");
    assert_eq!(f.call(&mut store, ()).expect("call failed"), 64);
}

#[test]
fn test_import_math_floor() {
    let source = r#"
package main

import "math"

func F(x float64) float64 {
    return math.Floor(x)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<f64, f64>(&mut store, "F")
        .expect("F not found");
    let result = f.call(&mut store, 3.7).expect("call failed");
    assert!((result - 3.0).abs() < f64::EPSILON);
}

// ==================== Root Cause 2: multi-return method resolution ====================

#[test]
fn test_multi_return_method_on_struct() {
    let source = r#"
package main

type Pair struct {
    a int
    b int
}

func (p Pair) Values() (int, int) {
    return p.a, p.b
}

func F() int {
    p := Pair{a: 10, b: 20}
    a, b := p.Values()
    return a + b
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .expect("F not found");
    assert_eq!(f.call(&mut store, ()).expect("call failed"), 30);
}

#[test]
fn test_multi_return_method_on_ptr_field() {
    let source = r#"
package main

type Inner struct {
    val int
}

func (i *Inner) GetTwo() (int, int) {
    return i.val, i.val * 2
}

type Outer struct {
    inner *Inner
}

func F() int {
    i := Inner{val: 5}
    o := Outer{inner: &i}
    a, b := o.inner.GetTwo()
    return a + b
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .expect("F not found");
    assert_eq!(f.call(&mut store, ()).expect("call failed"), 15);
}

// ==================== Root Cause 3: math.Log ====================

#[test]
fn test_math_log() {
    let source = r#"
package main

import "math"

func F() int {
    return int(math.Log(2.0))
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .expect("F not found");
    assert_eq!(f.call(&mut store, ()).expect("call failed"), 0);
}

// ==================== Minimal reproduction: one function at a time ====================

#[test]
fn test_minimal_float64bits_only() {
    let source = r#"
package main

func Float64bits(f float64) uint64

func F() uint64 {
    return Float64bits(1.5)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .expect("F not found");
    let result = f.call(&mut store, ()).expect("call failed");
    assert_eq!(result as u64, 1.5_f64.to_bits());
}

#[test]
fn test_minimal_float64frombits_call() {
    let source = r#"
package main

func Float64frombits(b uint64) float64

func F() float64 {
    return Float64frombits(0)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), f64>(&mut store, "F")
        .expect("F not found");
    let result = f.call(&mut store, ()).expect("call failed");
    assert!((result - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_minimal_float64bits_roundtrip() {
    let source = r#"
package main

func Float64bits(f float64) uint64
func Float64frombits(b uint64) float64

func F() float64 {
    return Float64frombits(Float64bits(1.5))
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), f64>(&mut store, "F")
        .expect("F not found");
    let result = f.call(&mut store, ()).expect("call failed");
    assert!((result - 1.5).abs() < f64::EPSILON);
}
