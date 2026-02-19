use gno_rs::wasm::compiler::WasmCompiler;
use gno_rs::wasm::runtime::{HostState, UdfRuntime};
use gno_rs::wasm::stdlib::{resolve_import, ImportKind};
use gno_rs::wasm::udf::Manifest;

#[test]
fn test_compile_and_run_simple_add() {
    let source = r#"
package main

func Add(a int, b int) int {
    return a + b
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let add_fn = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Add")
        .expect("Add function not found");

    let result = add_fn.call(&mut store, (3, 5)).expect("call failed");
    assert_eq!(result, 8);
}

#[test]
fn test_compile_and_run_float_math() {
    let source = r#"
package main

func Multiply(a float64, b float64) float64 {
    return a * b
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let mul_fn = instance
        .get_typed_func::<(f64, f64), f64>(&mut store, "Multiply")
        .expect("Multiply function not found");

    let result = mul_fn.call(&mut store, (2.5, 4.0)).expect("call failed");
    assert!((result - 10.0).abs() < f64::EPSILON);
}

#[test]
fn test_compile_with_if_else() {
    let source = r#"
package main

func Max(a int, b int) int {
    if a > b {
        return a
    }
    return b
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let max_fn = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Max")
        .expect("Max function not found");

    assert_eq!(max_fn.call(&mut store, (10, 5)).expect("call failed"), 10);
    assert_eq!(max_fn.call(&mut store, (3, 7)).expect("call failed"), 7);
}

#[test]
fn test_compile_with_for_loop() {
    let source = r#"
package main

func Sum(n int) int {
    result := 0
    i := 0
    for i < n {
        result = result + i
        i++
    }
    return result
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let sum_fn = instance
        .get_typed_func::<i64, i64>(&mut store, "Sum")
        .expect("Sum function not found");

    // Sum(5) = 0 + 1 + 2 + 3 + 4 = 10
    assert_eq!(sum_fn.call(&mut store, 5).expect("call failed"), 10);
}

#[test]
fn test_manifest_generation() {
    let source = r#"
package main

func ComputeScore(age int, score float64) (float64, error) {
    return float64(age) + score, nil
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    assert_eq!(result.manifest.functions.len(), 1);
    let func = &result.manifest.functions[0];
    assert_eq!(func.name, "ComputeScore");
    assert!(func.returns_error);
}

#[test]
fn test_manifest_json_roundtrip() {
    let source = r#"
package main

func Add(a int, b int) int {
    return a + b
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let json = result.manifest.to_json().expect("JSON serialization failed");
    let parsed = Manifest::from_json(&json).expect("JSON parse failed");
    assert_eq!(parsed.functions.len(), result.manifest.functions.len());
}

#[test]
fn test_type_conversion() {
    let source = r#"
package main

func IntToFloat(a int) float64 {
    return float64(a)
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let conv_fn = instance
        .get_typed_func::<i64, f64>(&mut store, "IntToFloat")
        .expect("IntToFloat function not found");

    let result = conv_fn.call(&mut store, 42).expect("call failed");
    assert!((result - 42.0).abs() < f64::EPSILON);
}

#[test]
fn test_stdlib_import_resolution() {
    assert_eq!(resolve_import("math").unwrap(), ImportKind::Stdlib("math".to_string()));
    assert_eq!(resolve_import("strings").unwrap(), ImportKind::Stdlib("strings".to_string()));
    assert_eq!(resolve_import("udf/scoring").unwrap(), ImportKind::Udf("scoring".to_string()));
    assert!(resolve_import("os").is_err());
    assert!(resolve_import("net/http").is_err());
    assert!(resolve_import("github.com/foo/bar").is_err());
}

#[test]
fn test_stdlib_rejection_extended() {
    assert!(resolve_import("os/exec").is_err());
    assert!(resolve_import("os/user").is_err());
    assert!(resolve_import("crypto").is_err());
    assert!(resolve_import("time").is_err());
    assert!(resolve_import("plugin").is_err());
    assert!(resolve_import("sync/atomic").is_err());
}

#[test]
fn test_fuel_limit() {
    let source = r#"
package main

func Infinite() int {
    for {
    }
    return 0
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 100).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let inf_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Infinite")
        .expect("Infinite function not found");

    let err = inf_fn.call(&mut store, ()).expect_err("should have trapped on resource limit");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("fuel") || err_msg.contains("wasm") || err_msg.contains("executing"),
        "error should indicate execution limit: {}", err_msg,
    );
}

#[test]
fn test_context_host_functions() {
    let runtime = UdfRuntime::new().expect("runtime init failed");

    let source = r#"
package main

func Greet() int {
    return 42
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new()
        .with_query_metadata("q-123", "mydb", "public", "alice");

    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let greet_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Greet")
        .expect("Greet function not found");

    assert_eq!(greet_fn.call(&mut store, ()).expect("call failed"), 42);

    assert_eq!(store.data().query_id, "q-123");
    assert_eq!(store.data().database, "mydb");
}

#[test]
fn test_manifest_validation() {
    let source = r#"
package main

func Add(a int, b int) int {
    return a + b
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    // Valid validation
    let ok = result.manifest.validate_function(
        "Add",
        &[("a".to_string(), "int".to_string()), ("b".to_string(), "int".to_string())],
        "int",
    );
    assert!(ok.is_ok(), "validation should pass: {:?}", ok);

    // Wrong type
    let err = result.manifest.validate_function(
        "Add",
        &[("a".to_string(), "float64".to_string()), ("b".to_string(), "int".to_string())],
        "int",
    );
    assert!(err.is_err());

    // Non-existent function
    let err = result.manifest.validate_function("NonExistent", &[], "int");
    assert!(err.is_err());
}

#[test]
fn test_compound_add_assign() {
    let source = r#"
package main

func Accumulate(n int) int {
    total := 0
    i := 0
    for i < n {
        total += i
        i++
    }
    return total
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Accumulate")
        .expect("Accumulate function not found");

    // 0 + 1 + 2 + 3 + 4 = 10
    assert_eq!(func.call(&mut store, 5).expect("call failed"), 10);
}

#[test]
fn test_compound_sub_assign() {
    let source = r#"
package main

func Countdown(n int) int {
    result := 100
    i := 0
    for i < n {
        result -= 10
        i++
    }
    return result
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Countdown")
        .expect("Countdown function not found");

    // 100 - 10*3 = 70
    assert_eq!(func.call(&mut store, 3).expect("call failed"), 70);
}

#[test]
fn test_compound_mul_assign() {
    let source = r#"
package main

func Factorial(n int) int {
    result := 1
    i := 1
    for i <= n {
        result *= i
        i++
    }
    return result
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Factorial")
        .expect("Factorial function not found");

    // 5! = 120
    assert_eq!(func.call(&mut store, 5).expect("call failed"), 120);
}

#[test]
fn test_compound_float_add_assign() {
    let source = r#"
package main

func SumFloats(a float64, b float64, c float64) float64 {
    result := 0.0
    result += a
    result += b
    result += c
    return result
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(f64, f64, f64), f64>(&mut store, "SumFloats")
        .expect("SumFloats function not found");

    let result = func.call(&mut store, (1.5, 2.5, 3.0)).expect("call failed");
    assert!((result - 7.0).abs() < f64::EPSILON);
}

#[test]
fn test_multiple_returns() {
    let source = r#"
package main

func DivMod(a int, b int) (int, int) {
    return a / b, a % b
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(i64, i64), (i64, i64)>(&mut store, "DivMod")
        .expect("DivMod function not found");

    let (quot, rem) = func.call(&mut store, (17, 5)).expect("call failed");
    assert_eq!(quot, 3);
    assert_eq!(rem, 2);
}

#[test]
fn test_nested_if_else() {
    let source = r#"
package main

func Classify(x int) int {
    if x > 0 {
        if x > 100 {
            return 3
        } else {
            return 2
        }
    } else {
        if x < 0 {
            return 1
        }
    }
    return 0
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Classify")
        .expect("Classify function not found");

    assert_eq!(func.call(&mut store, 200).expect("call failed"), 3);
    assert_eq!(func.call(&mut store, 50).expect("call failed"), 2);
    assert_eq!(func.call(&mut store, -5).expect("call failed"), 1);
    assert_eq!(func.call(&mut store, 0).expect("call failed"), 0);
}

#[test]
fn test_unary_negation() {
    let source = r#"
package main

func Negate(a int) int {
    return -a
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Negate")
        .expect("Negate function not found");

    assert_eq!(func.call(&mut store, 42).expect("call failed"), -42);
    assert_eq!(func.call(&mut store, -7).expect("call failed"), 7);
}

#[test]
fn test_boolean_not() {
    let source = r#"
package main

func IsZero(x int) bool {
    return x == 0
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i32>(&mut store, "IsZero")
        .expect("IsZero function not found");

    assert_eq!(func.call(&mut store, 0).expect("call failed"), 1);
    assert_eq!(func.call(&mut store, 5).expect("call failed"), 0);
}

#[test]
fn test_float_comparison() {
    let source = r#"
package main

func MaxFloat(a float64, b float64) float64 {
    if a > b {
        return a
    }
    return b
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(f64, f64), f64>(&mut store, "MaxFloat")
        .expect("MaxFloat function not found");

    let result = func.call(&mut store, (3.14, 2.71)).expect("call failed");
    assert!((result - 3.14).abs() < f64::EPSILON);

    let result = func.call(&mut store, (1.0, 9.9)).expect("call failed");
    assert!((result - 9.9).abs() < f64::EPSILON);
}

#[test]
fn test_aggregate_manifest_detection() {
    let source = r#"
package main

type SumAgg struct {
    total int
    count int
}

func (s SumAgg) Accumulate(val int) {
}

func (s SumAgg) Finalize() int {
    return s.total
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    assert!(
        !result.manifest.aggregates.is_empty(),
        "should detect at least one aggregate"
    );
    assert_eq!(result.manifest.aggregates[0].name, "SumAgg");
}

#[test]
fn test_aggregate_exports() {
    let source = r#"
package main

type Counter struct {
    n int
}

func (c Counter) Accumulate(val int) {
}

func (c Counter) Finalize() int {
    return 0
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    // Verify the init export exists and returns a non-zero pointer
    let init_fn = instance
        .get_typed_func::<(), i32>(&mut store, "Counter_init")
        .expect("Counter_init function not found");
    let ptr = init_fn.call(&mut store, ()).expect("Counter_init call failed");
    assert!(ptr > 0, "init should return a non-zero heap pointer");
}

#[test]
fn test_manifest_multiple_functions() {
    let source = r#"
package main

func Add(a int, b int) int {
    return a + b
}

func Subtract(a int, b int) int {
    return a - b
}

func helper(x int) int {
    return x
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    // Only uppercase functions appear in manifest
    assert_eq!(result.manifest.functions.len(), 2);
    let names: Vec<&str> = result.manifest.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"Add"));
    assert!(names.contains(&"Subtract"));
}

#[test]
fn test_manifest_returns_error_flag() {
    let source = r#"
package main

func Safe(x int) int {
    return x
}

func Risky(x int) (int, error) {
    return x, nil
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let safe = result.manifest.functions.iter().find(|f| f.name == "Safe").unwrap();
    assert!(!safe.returns_error);

    let risky = result.manifest.functions.iter().find(|f| f.name == "Risky").unwrap();
    assert!(risky.returns_error);
}

#[test]
fn test_panic_traps() {
    let source = r#"
package main

func WillPanic() int {
    panic("oh no")
    return 0
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "WillPanic")
        .expect("WillPanic function not found");

    let err = func.call(&mut store, ()).expect_err("should trap on panic");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("unreachable") || err_msg.contains("wasm"),
        "panic should cause an unreachable trap: {}",
        err_msg
    );
}

#[test]
fn test_bitwise_operations() {
    let source = r#"
package main

func BitwiseAnd(a int, b int) int {
    return a & b
}

func BitwiseOr(a int, b int) int {
    return a | b
}

func BitwiseXor(a int, b int) int {
    return a ^ b
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let and_fn = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "BitwiseAnd")
        .expect("BitwiseAnd not found");
    assert_eq!(and_fn.call(&mut store, (0b1100, 0b1010)).expect("call failed"), 0b1000);

    let or_fn = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "BitwiseOr")
        .expect("BitwiseOr not found");
    assert_eq!(or_fn.call(&mut store, (0b1100, 0b1010)).expect("call failed"), 0b1110);

    let xor_fn = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "BitwiseXor")
        .expect("BitwiseXor not found");
    assert_eq!(xor_fn.call(&mut store, (0b1100, 0b1010)).expect("call failed"), 0b0110);
}

#[test]
fn test_inc_dec_statement() {
    let source = r#"
package main

func IncDec(x int) int {
    x++
    x++
    x--
    return x
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "IncDec")
        .expect("IncDec not found");

    // 10 + 1 + 1 - 1 = 11
    assert_eq!(func.call(&mut store, 10).expect("call failed"), 11);
}

#[test]
fn test_local_variable_scoping() {
    let source = r#"
package main

func Scope(x int) int {
    a := x + 1
    b := a * 2
    c := b - 3
    return c
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Scope")
        .expect("Scope not found");

    // x=5: a=6, b=12, c=9
    assert_eq!(func.call(&mut store, 5).expect("call failed"), 9);
}

#[test]
fn test_complex_arithmetic() {
    let source = r#"
package main

func Complex(a int, b int, c int) int {
    d := a * b + c
    e := d - a
    return e * 2
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(i64, i64, i64), i64>(&mut store, "Complex")
        .expect("Complex not found");

    // a=3, b=4, c=5: d=17, e=14, result=28
    assert_eq!(func.call(&mut store, (3, 4, 5)).expect("call failed"), 28);
}

#[test]
fn test_for_loop_with_break() {
    let source = r#"
package main

func FindFirst(limit int) int {
    i := 0
    for i < limit {
        if i == 5 {
            break
        }
        i++
    }
    return i
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "FindFirst")
        .expect("FindFirst not found");

    assert_eq!(func.call(&mut store, 100).expect("call failed"), 5);
    assert_eq!(func.call(&mut store, 3).expect("call failed"), 3);
}

#[test]
fn test_multiple_function_calls() {
    let source = r#"
package main

func double(x int) int {
    return x * 2
}

func Quadruple(x int) int {
    return double(double(x))
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Quadruple")
        .expect("Quadruple not found");

    assert_eq!(func.call(&mut store, 3).expect("call failed"), 12);
    assert_eq!(func.call(&mut store, 7).expect("call failed"), 28);
}

#[test]
fn test_recursive_function() {
    let source = r#"
package main

func Fib(n int) int {
    if n <= 1 {
        return n
    }
    return Fib(n - 1) + Fib(n - 2)
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Fib")
        .expect("Fib not found");

    assert_eq!(func.call(&mut store, 0).expect("call failed"), 0);
    assert_eq!(func.call(&mut store, 1).expect("call failed"), 1);
    assert_eq!(func.call(&mut store, 10).expect("call failed"), 55);
}

#[test]
fn test_struct_type_declaration_compiles() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func GetX(x int, y int) int {
    return x
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("struct type compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "GetX")
        .expect("GetX not found");

    assert_eq!(func.call(&mut store, (10, 20)).expect("call failed"), 10);
}

#[test]
fn test_alloc_export_exists() {
    let source = r#"
package main

func Identity(x int) int {
    return x
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let alloc_fn = instance
        .get_typed_func::<i32, i32>(&mut store, "alloc")
        .expect("alloc function should be exported");

    let ptr = alloc_fn.call(&mut store, 64).expect("alloc call failed");
    assert!(ptr > 0, "alloc should return non-zero pointer");

    // Allocating again should return a different (higher) pointer
    let ptr2 = alloc_fn.call(&mut store, 32).expect("alloc call failed");
    assert!(ptr2 > ptr, "second alloc should return higher pointer");
}

#[test]
fn test_mixed_int_float_arithmetic() {
    let source = r#"
package main

func MixedCalc(intVal int, floatVal float64) float64 {
    return float64(intVal) * floatVal
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(i64, f64), f64>(&mut store, "MixedCalc")
        .expect("MixedCalc not found");

    let result = func.call(&mut store, (3, 2.5)).expect("call failed");
    assert!((result - 7.5).abs() < f64::EPSILON);
}

#[test]
fn test_logical_operators() {
    let source = r#"
package main

func InRange(x int, lo int, hi int) bool {
    if x >= lo && x <= hi {
        return true
    }
    return false
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(i64, i64, i64), i32>(&mut store, "InRange")
        .expect("InRange not found");

    assert_eq!(func.call(&mut store, (5, 1, 10)).expect("call failed"), 1);
    assert_eq!(func.call(&mut store, (15, 1, 10)).expect("call failed"), 0);
    assert_eq!(func.call(&mut store, (0, 1, 10)).expect("call failed"), 0);
}

#[test]
fn test_early_return() {
    let source = r#"
package main

func AbsVal(x int) int {
    if x < 0 {
        return -x
    }
    return x
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "AbsVal")
        .expect("AbsVal not found");

    assert_eq!(func.call(&mut store, 42).expect("call failed"), 42);
    assert_eq!(func.call(&mut store, -42).expect("call failed"), 42);
    assert_eq!(func.call(&mut store, 0).expect("call failed"), 0);
}

#[test]
fn test_switch_with_default() {
    let source = r#"
package main

func Classify(x int) int {
    result := 0
    switch x {
    case 1:
        result = 10
    case 2:
        result = 20
    default:
        result = 99
    }
    return result
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Classify")
        .expect("Classify not found");

    assert_eq!(func.call(&mut store, 1).expect("call failed"), 10);
    assert_eq!(func.call(&mut store, 2).expect("call failed"), 20);
    assert_eq!(func.call(&mut store, 3).expect("call failed"), 99);
    assert_eq!(func.call(&mut store, 0).expect("call failed"), 99);
}

#[test]
fn test_switch_exclusive_execution() {
    let source = r#"
package main

func Count(x int) int {
    counter := 0
    switch x {
    case 1:
        counter = counter + 10
    case 2:
        counter = counter + 20
    case 3:
        counter = counter + 30
    default:
        counter = counter + 100
    }
    return counter
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Count")
        .expect("Count not found");

    // Only one case body should execute, never multiple
    assert_eq!(func.call(&mut store, 1).expect("call failed"), 10);
    assert_eq!(func.call(&mut store, 2).expect("call failed"), 20);
    assert_eq!(func.call(&mut store, 3).expect("call failed"), 30);
    assert_eq!(func.call(&mut store, 999).expect("call failed"), 100);
}

#[test]
fn test_switch_no_default() {
    let source = r#"
package main

func Check(x int) int {
    result := 0
    switch x {
    case 1:
        result = 10
    case 2:
        result = 20
    }
    return result
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Check")
        .expect("Check not found");

    assert_eq!(func.call(&mut store, 1).expect("call failed"), 10);
    assert_eq!(func.call(&mut store, 2).expect("call failed"), 20);
    // No matching case, no default: result stays 0
    assert_eq!(func.call(&mut store, 3).expect("call failed"), 0);
}

#[test]
fn test_continue_in_loop() {
    let source = r#"
package main

func SumOdd(n int) int {
    total := 0
    i := 0
    for i < n {
        i++
        if i % 2 == 0 {
            continue
        }
        total += i
    }
    return total
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "SumOdd")
        .expect("SumOdd not found");

    // Odd numbers up to 6: 1 + 3 + 5 = 9
    assert_eq!(func.call(&mut store, 6).expect("call failed"), 9);
    // Odd numbers up to 10: 1 + 3 + 5 + 7 + 9 = 25
    assert_eq!(func.call(&mut store, 10).expect("call failed"), 25);
}

#[test]
fn test_nested_loops_break() {
    let source = r#"
package main

func Search(rows int, cols int) int {
    total := 0
    i := 0
    for i < rows {
        j := 0
        for j < cols {
            if j == 2 {
                break
            }
            total++
            j++
        }
        i++
    }
    return total
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Search")
        .expect("Search not found");

    // 3 rows, inner loop breaks at j==2, so 2 iterations per row = 6
    assert_eq!(func.call(&mut store, (3, 5)).expect("call failed"), 6);
    // 4 rows, inner loop breaks at j==2 = 8
    assert_eq!(func.call(&mut store, (4, 10)).expect("call failed"), 8);
}

#[test]
fn test_void_function() {
    let source = r#"
package main

func DoNothing() {
}

func Identity(x int) int {
    return x
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), ()>(&mut store, "DoNothing")
        .expect("DoNothing not found");
    func.call(&mut store, ()).expect("void call failed");

    // Ensure other functions still work in the same module
    let id_fn = instance
        .get_typed_func::<i64, i64>(&mut store, "Identity")
        .expect("Identity not found");
    assert_eq!(id_fn.call(&mut store, 42).expect("call failed"), 42);
}

#[test]
fn test_switch_inside_for_with_break() {
    let source = r#"
package main

func FindSpecial(n int) int {
    i := 0
    for i < n {
        switch i {
        case 5:
            return i
        }
        i++
    }
    return -1
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "FindSpecial")
        .expect("FindSpecial not found");

    assert_eq!(func.call(&mut store, 10).expect("call failed"), 5);
    assert_eq!(func.call(&mut store, 3).expect("call failed"), -1);
}

#[test]
fn test_switch_default_only() {
    let source = r#"
package main

func AlwaysDefault(x int) int {
    result := 0
    switch x {
    default:
        result = 42
    }
    return result
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "AlwaysDefault")
        .expect("AlwaysDefault not found");

    assert_eq!(func.call(&mut store, 0).expect("call failed"), 42);
    assert_eq!(func.call(&mut store, 999).expect("call failed"), 42);
}

#[test]
fn test_string_literal_in_memory() {
    let source = r#"
package main

func Greet() int {
    s := "hello"
    _ = s
    return 42
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let greet_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Greet")
        .expect("Greet not found");

    let result_val = greet_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 42);

    let memory = instance
        .get_memory(&mut store, "memory")
        .expect("memory export not found");

    // The string "hello" should be in memory after offset 1024 (heap start)
    let data = memory.data(&store);
    let heap_start = 1024usize;
    let heap_data = &data[heap_start..];
    let pos = heap_data
        .windows(5)
        .position(|w| w == b"hello")
        .expect("string 'hello' not found in WASM memory");
    assert!(pos < 1024, "string should be near heap start");
}

#[test]
fn test_mixed_type_complex_expressions() {
    let source = r#"
package main

func ComplexMixed(a int, b int) float64 {
    return float64(a + b) * 2.5
}

func IntTimesFloat(x int, y float64) float64 {
    return float64(x) * y + 1.0
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let complex_fn = instance
        .get_typed_func::<(i64, i64), f64>(&mut store, "ComplexMixed")
        .expect("ComplexMixed not found");
    let result_val = complex_fn.call(&mut store, (3, 7)).expect("call failed");
    assert!((result_val - 25.0).abs() < f64::EPSILON, "expected 25.0, got {}", result_val);

    let int_float_fn = instance
        .get_typed_func::<(i64, f64), f64>(&mut store, "IntTimesFloat")
        .expect("IntTimesFloat not found");
    let result_val = int_float_fn.call(&mut store, (4, 3.0)).expect("call failed");
    assert!((result_val - 13.0).abs() < f64::EPSILON, "expected 13.0, got {}", result_val);
}

#[test]
fn test_compound_assign_complex_rhs() {
    let source = r#"
package main

func CompoundComplex(a int, b int) int {
    result := 10
    result += a * b
    return result
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "CompoundComplex")
        .expect("CompoundComplex not found");
    let result_val = func.call(&mut store, (3, 5)).expect("call failed");
    assert_eq!(result_val, 25, "10 + (3*5) should be 25");
}

#[test]
fn test_undefined_variable_error() {
    let source = r#"
package main

func Bad() int {
    return undefinedVar
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("should fail on undefined variable"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("undefined") || err_msg.contains("undefinedVar"),
                "error should mention undefined identifier, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_slice_expression_compiles() {
    let source = r#"
package main

func SliceTest(n int) int {
    total := 0
    return total
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "SliceTest")
        .expect("SliceTest not found");
    let result_val = func.call(&mut store, 5).expect("call failed");
    assert_eq!(result_val, 0);
}

#[test]
fn test_math_stdlib_functions() {
    let source = r#"
package main

import "math"

func TestSqrt(x float64) float64 {
    return math.Sqrt(x)
}

func TestAbs(x float64) float64 {
    return math.Abs(x)
}

func TestFloor(x float64) float64 {
    return math.Floor(x)
}

func TestCeil(x float64) float64 {
    return math.Ceil(x)
}

func TestMin(a float64, b float64) float64 {
    return math.Min(a, b)
}

func TestMax(a float64, b float64) float64 {
    return math.Max(a, b)
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let sqrt_fn = instance
        .get_typed_func::<f64, f64>(&mut store, "TestSqrt")
        .expect("TestSqrt not found");
    let r = sqrt_fn.call(&mut store, 16.0).expect("call failed");
    assert!((r - 4.0).abs() < f64::EPSILON, "sqrt(16) = {}", r);

    let abs_fn = instance
        .get_typed_func::<f64, f64>(&mut store, "TestAbs")
        .expect("TestAbs not found");
    let r = abs_fn.call(&mut store, -3.5).expect("call failed");
    assert!((r - 3.5).abs() < f64::EPSILON, "abs(-3.5) = {}", r);

    let floor_fn = instance
        .get_typed_func::<f64, f64>(&mut store, "TestFloor")
        .expect("TestFloor not found");
    let r = floor_fn.call(&mut store, 3.7).expect("call failed");
    assert!((r - 3.0).abs() < f64::EPSILON, "floor(3.7) = {}", r);

    let ceil_fn = instance
        .get_typed_func::<f64, f64>(&mut store, "TestCeil")
        .expect("TestCeil not found");
    let r = ceil_fn.call(&mut store, 3.2).expect("call failed");
    assert!((r - 4.0).abs() < f64::EPSILON, "ceil(3.2) = {}", r);

    let min_fn = instance
        .get_typed_func::<(f64, f64), f64>(&mut store, "TestMin")
        .expect("TestMin not found");
    let r = min_fn.call(&mut store, (5.0, 3.0)).expect("call failed");
    assert!((r - 3.0).abs() < f64::EPSILON, "min(5,3) = {}", r);

    let max_fn = instance
        .get_typed_func::<(f64, f64), f64>(&mut store, "TestMax")
        .expect("TestMax not found");
    let r = max_fn.call(&mut store, (5.0, 3.0)).expect("call failed");
    assert!((r - 5.0).abs() < f64::EPSILON, "max(5,3) = {}", r);
}

#[test]
fn test_string_escape_sequences() {
    let source = r#"
package main

func EscapeTest() int {
    s := "ab\nc"
    _ = s
    return 42
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "EscapeTest")
        .expect("EscapeTest not found");
    func.call(&mut store, ()).expect("call failed");

    let memory = instance
        .get_memory(&mut store, "memory")
        .expect("memory export not found");
    let data = memory.data(&store);
    let heap = &data[1024..];
    // "ab\nc" should be 4 bytes: 'a', 'b', '\n', 'c'
    let pos = heap
        .windows(4)
        .position(|w| w == b"ab\nc")
        .expect("escaped string not found in memory");
    assert!(pos < 1024, "string should be near heap start");
}

#[test]
fn test_char_escape_sequence() {
    let source = r#"
package main

func NewlineChar() int {
    c := '\n'
    return int(c)
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "NewlineChar")
        .expect("NewlineChar not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 10, "newline char should be 10");
}

#[test]
fn test_undefined_function_error() {
    let source = r#"
package main

func Bad() int {
    return nonExistentFunc(1, 2)
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("should fail on undefined function"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("undefined") || err_msg.contains("nonExistentFunc"),
                "error should mention undefined function, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_defer_compiles_and_runs() {
    let source = r#"
package main

func noop(x int) {
}

func DeferTest() int {
    result := 10
    defer noop(1)
    result = result + 5
    return result
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("defer compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "DeferTest")
        .expect("DeferTest not found");
    let r = func.call(&mut store, ()).expect("call failed");
    assert_eq!(r, 15, "10 + 5 with deferred noop should be 15");
}

#[test]
fn test_struct_field_access() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func MakePoint(x int, y int) int {
    p := Point{X: x, Y: y}
    return p.X + p.Y
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "MakePoint")
        .expect("MakePoint not found");
    let result_val = func.call(&mut store, (3, 7)).expect("call failed");
    assert_eq!(result_val, 10, "3 + 7 should be 10");
}

#[test]
fn test_division_by_zero_traps() {
    let source = r#"
package main

func DivByZero(a int) int {
    return a / 0
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "DivByZero")
        .expect("DivByZero not found");
    let result = func.call(&mut store, 42);
    assert!(result.is_err(), "division by zero should trap");
}

#[test]
fn test_multiple_type_conversions() {
    let source = r#"
package main

func IntToFloat(x int) float64 {
    return float64(x)
}

func FloatToInt(x float64) int {
    return int(x)
}

func ChainConvert(x int) int {
    f := float64(x) * 1.5
    return int(f)
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let itf = instance
        .get_typed_func::<i64, f64>(&mut store, "IntToFloat")
        .expect("IntToFloat not found");
    let r = itf.call(&mut store, 7).expect("call failed");
    assert!((r - 7.0).abs() < f64::EPSILON);

    let fti = instance
        .get_typed_func::<f64, i64>(&mut store, "FloatToInt")
        .expect("FloatToInt not found");
    let r = fti.call(&mut store, 7.9).expect("call failed");
    assert_eq!(r, 7);

    let chain = instance
        .get_typed_func::<i64, i64>(&mut store, "ChainConvert")
        .expect("ChainConvert not found");
    let r = chain.call(&mut store, 10).expect("call failed");
    assert_eq!(r, 15, "int(float64(10) * 1.5) should be 15");
}

#[test]
fn test_short_var_decl_with_function_call() {
    let source = r#"
package main

func helper(x int) int {
    return x * 2
}

func Main(n int) int {
    result := helper(n) + 1
    return result
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Main")
        .expect("Main not found");
    let r = func.call(&mut store, 5).expect("call failed");
    assert_eq!(r, 11, "helper(5) + 1 = 11");
}

// ============================================================
// Regression tests for bug fixes (T1-T17)
// ============================================================

#[test]
fn test_global_var_error() {
    let source = r#"
package main

var counter int

func Add(a int, b int) int {
    return a + b
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("global variables should produce a compile error"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("global") && err_msg.contains("not supported"),
                "error should mention global variables not supported, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_global_const_error() {
    let source = r#"
package main

const maxSize = 100

func Add(a int, b int) int {
    return a + b
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("global constants should produce a compile error"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("global") && err_msg.contains("not supported"),
                "error should mention global constants not supported, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_len_on_string() {
    let source = r#"
package main

func StringLen() int {
    s := "hello"
    return int(len(s))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "StringLen")
        .expect("StringLen not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 5, "len(\"hello\") should be 5");
}

#[test]
fn test_make_slice() {
    let source = r#"
package main

func MakeSlice(n int) int {
    s := make([]int, n)
    return int64(len(s))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "MakeSlice")
        .expect("MakeSlice not found");
    let result_val = func.call(&mut store, 5).expect("call failed");
    assert_eq!(result_val, 5, "len(make([]int, 5)) should be 5");
}

#[test]
fn test_append_basic() {
    let source = r#"
package main

func AppendTest() int {
    s := make([]int, 0)
    s = append(s, 10)
    s = append(s, 20)
    s = append(s, 30)
    return int64(len(s))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "AppendTest")
        .expect("AppendTest not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 3, "appending 3 elements should give len 3");
}

#[test]
fn test_slice_index_after_append() {
    let source = r#"
package main

func IndexTest() int {
    s := make([]int, 0)
    s = append(s, 10)
    s = append(s, 20)
    s = append(s, 30)
    return s[1]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "IndexTest")
        .expect("IndexTest not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 20, "s[1] should be 20 after appending 10, 20, 30");
}

#[test]
fn test_fmt_errorf_errors_until_stdlib() {
    let source = r#"
package main

import "fmt"

func Bad() int {
    _ = fmt.Errorf("something went wrong")
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("fmt.Errorf should produce a compile error until stdlib is wired"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("not yet available") || err_msg.contains("host functions"),
                "error should mention stdlib not available, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_invalid_escape_sequence_error() {
    let source = r#"
package main

func Bad() int {
    c := '\q'
    return int(c)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("invalid escape sequence should produce a compile error"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("escape"),
                "error should mention escape sequence, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_decrement_non_i64() {
    let source = r#"
package main

func DecrementI32() int {
    x := int32(10)
    x--
    return int(x)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "DecrementI32")
        .expect("DecrementI32 not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 9, "int32(10)-- should be 9, not 11");
}

#[test]
fn test_fallthrough_error() {
    let source = r#"
package main

func Bad(x int) int {
    switch x {
    case 1:
        fallthrough
    case 2:
        return 2
    }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("fallthrough should produce a compile error"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("unsupported") || err_msg.contains("branch"),
                "error should mention unsupported branch keyword, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_slice_operations_combined() {
    let source = r#"
package main

func SliceOps() int {
    s := make([]int, 0)
    s = append(s, 100)
    s = append(s, 200)
    s = append(s, 300)
    total := s[0] + s[1] + s[2]
    return total
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "SliceOps")
        .expect("SliceOps not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 600, "100 + 200 + 300 = 600");
}

#[test]
fn test_range_over_slice() {
    let source = r#"
package main

func RangeSum() int {
    s := make([]int, 0)
    s = append(s, 10)
    s = append(s, 20)
    s = append(s, 30)
    total := 0
    for _, v := range s {
        total = total + v
    }
    return total
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "RangeSum")
        .expect("RangeSum not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 60, "10 + 20 + 30 = 60");
}

#[test]
fn test_closure_captures_variable() {
    let source = r#"
package main

func ClosureTest() int {
    x := 10
    add := func() int {
        return x + 5
    }
    return add()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "ClosureTest")
        .expect("ClosureTest function not found");

    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 15, "10 + 5 via closure should be 15");
}

#[test]
fn test_method_call_on_struct() {
    let source = r#"
package main

type Counter struct {
    Value int
}

func (c *Counter) Get() int {
    return c.Value
}

func TestMethod() int {
    c := Counter{Value: 42}
    return c.Get()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "TestMethod")
        .expect("TestMethod function not found");

    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 42, "Counter.Get() should return 42");
}

#[test]
fn test_string_concatenation() {
    let source = r#"
package main

func ConcatLen() int {
    a := "hello"
    b := " world"
    c := a + b
    return int(len(c))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "ConcatLen")
        .expect("ConcatLen not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 11, "len(\"hello\" + \" world\") should be 11");
}

#[test]
fn test_switch_multi_value_case() {
    let source = r#"
package main

func Classify(x int) int {
    switch x {
    case 1, 2:
        return 10
    case 3, 4, 5:
        return 20
    }
    return 0
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Classify")
        .expect("Classify not found");

    assert_eq!(func.call(&mut store, 1).expect("call failed"), 10);
    assert_eq!(func.call(&mut store, 2).expect("call failed"), 10);
    assert_eq!(func.call(&mut store, 3).expect("call failed"), 20);
    assert_eq!(func.call(&mut store, 4).expect("call failed"), 20);
    assert_eq!(func.call(&mut store, 5).expect("call failed"), 20);
    assert_eq!(func.call(&mut store, 6).expect("call failed"), 0);
}

#[test]
fn test_switch_multi_value_with_default() {
    let source = r#"
package main

func Classify(x int) int {
    result := 0
    switch x {
    case 1, 2:
        result = 10
    case 3, 4, 5:
        result = 20
    default:
        result = 99
    }
    return result
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Classify")
        .expect("Classify not found");

    assert_eq!(func.call(&mut store, 1).expect("call failed"), 10);
    assert_eq!(func.call(&mut store, 2).expect("call failed"), 10);
    assert_eq!(func.call(&mut store, 3).expect("call failed"), 20);
    assert_eq!(func.call(&mut store, 5).expect("call failed"), 20);
    assert_eq!(func.call(&mut store, 0).expect("call failed"), 99);
    assert_eq!(func.call(&mut store, 100).expect("call failed"), 99);
}

#[test]
fn test_unsupported_operator_error() {
    let source = r#"
package main

func BitClear(a int, b int) int {
    return a &^ b
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("unsupported operator"),
                "error should mention unsupported operator, got: {}",
                err_msg
            );
        }
        Ok(_) => panic!("expected error for unsupported &^ operator"),
    }
}

#[test]
fn test_unary_negation_f32() {
    let source = r#"
package main

func NegF32(x float32) float32 {
    return -x
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<f32, f32>(&mut store, "NegF32")
        .expect("NegF32 not found");

    let result = func.call(&mut store, 3.5).expect("call failed");
    assert!((result - (-3.5f32)).abs() < f32::EPSILON, "expected -3.5, got {}", result);

    let result = func.call(&mut store, -2.0).expect("call failed");
    assert!((result - 2.0f32).abs() < f32::EPSILON, "expected 2.0, got {}", result);
}

#[test]
fn test_int_to_int_conversion_noop() {
    let source = r#"
package main

func Identity(x int) int {
    return int(x)
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "Identity")
        .expect("Identity not found");

    assert_eq!(func.call(&mut store, 42).expect("call failed"), 42);
    assert_eq!(func.call(&mut store, -7).expect("call failed"), -7);
}

#[test]
fn test_float32_to_int_conversion() {
    let source = r#"
package main

func F32ToInt(x float32) int {
    return int(x)
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<f32, i64>(&mut store, "F32ToInt")
        .expect("F32ToInt not found");

    assert_eq!(func.call(&mut store, 3.7).expect("call failed"), 3);
    assert_eq!(func.call(&mut store, -2.9).expect("call failed"), -2);
}

#[test]
fn test_compound_assign_error_on_non_ident() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func MoveX(p Point) int {
    p.X += 10
    return p.X
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("compound assignment")
                    || err_msg.contains("not supported"),
                "error should mention compound assignment limitation, got: {}",
                err_msg
            );
        }
        Ok(_) => panic!("expected error for compound assignment to struct field"),
    }
}

#[test]
fn test_f32_arithmetic() {
    let source = r#"
package main

func AddF32(a float32, b float32) float32 {
    return a + b
}

func SubF32(a float32, b float32) float32 {
    return a - b
}

func MulF32(a float32, b float32) float32 {
    return a * b
}

func DivF32(a float32, b float32) float32 {
    return a / b
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let add = instance
        .get_typed_func::<(f32, f32), f32>(&mut store, "AddF32")
        .expect("AddF32 not found");
    let result = add.call(&mut store, (1.5, 2.5)).expect("call failed");
    assert!((result - 4.0).abs() < f32::EPSILON, "1.5 + 2.5 = {}, expected 4.0", result);

    let sub = instance
        .get_typed_func::<(f32, f32), f32>(&mut store, "SubF32")
        .expect("SubF32 not found");
    let result = sub.call(&mut store, (5.0, 3.0)).expect("call failed");
    assert!((result - 2.0).abs() < f32::EPSILON, "5.0 - 3.0 = {}, expected 2.0", result);

    let mul = instance
        .get_typed_func::<(f32, f32), f32>(&mut store, "MulF32")
        .expect("MulF32 not found");
    let result = mul.call(&mut store, (2.0, 3.0)).expect("call failed");
    assert!((result - 6.0).abs() < f32::EPSILON, "2.0 * 3.0 = {}, expected 6.0", result);

    let div = instance
        .get_typed_func::<(f32, f32), f32>(&mut store, "DivF32")
        .expect("DivF32 not found");
    let result = div.call(&mut store, (10.0, 4.0)).expect("call failed");
    assert!((result - 2.5).abs() < f32::EPSILON, "10.0 / 4.0 = {}, expected 2.5", result);
}

#[test]
fn test_f32_comparison() {
    let source = r#"
package main

func CompareF32(a float32, b float32) int {
    if a < b {
        return -1
    }
    if a > b {
        return 1
    }
    return 0
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(f32, f32), i64>(&mut store, "CompareF32")
        .expect("CompareF32 not found");

    assert_eq!(func.call(&mut store, (1.0, 2.0)).expect("call failed"), -1);
    assert_eq!(func.call(&mut store, (3.0, 2.0)).expect("call failed"), 1);
    assert_eq!(func.call(&mut store, (2.0, 2.0)).expect("call failed"), 0);
}

#[test]
fn test_logical_and_or_with_comparisons() {
    let source = r#"
package main

func BothPositive(x int, y int) int {
    if x > 0 && y > 0 {
        return 1
    }
    return 0
}

func EitherPositive(x int, y int) int {
    if x > 0 || y > 0 {
        return 1
    }
    return 0
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let both = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "BothPositive")
        .expect("BothPositive not found");

    assert_eq!(both.call(&mut store, (1, 2)).expect("call failed"), 1);
    assert_eq!(both.call(&mut store, (-1, 2)).expect("call failed"), 0);
    assert_eq!(both.call(&mut store, (1, -2)).expect("call failed"), 0);
    assert_eq!(both.call(&mut store, (-1, -2)).expect("call failed"), 0);

    let either = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "EitherPositive")
        .expect("EitherPositive not found");

    assert_eq!(either.call(&mut store, (1, 2)).expect("call failed"), 1);
    assert_eq!(either.call(&mut store, (-1, 2)).expect("call failed"), 1);
    assert_eq!(either.call(&mut store, (1, -2)).expect("call failed"), 1);
    assert_eq!(either.call(&mut store, (-1, -2)).expect("call failed"), 0);
}

#[test]
fn test_bitwise_shift_operations() {
    let source = r#"
package main

func ShiftLeft(x int, n int) int {
    return x << n
}

func ShiftRight(x int, n int) int {
    return x >> n
}
"#;

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let shl = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "ShiftLeft")
        .expect("ShiftLeft not found");

    assert_eq!(shl.call(&mut store, (1, 0)).expect("call failed"), 1);
    assert_eq!(shl.call(&mut store, (1, 3)).expect("call failed"), 8);
    assert_eq!(shl.call(&mut store, (5, 2)).expect("call failed"), 20);

    let shr = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "ShiftRight")
        .expect("ShiftRight not found");

    assert_eq!(shr.call(&mut store, (8, 3)).expect("call failed"), 1);
    assert_eq!(shr.call(&mut store, (20, 2)).expect("call failed"), 5);
    assert_eq!(shr.call(&mut store, (16, 1)).expect("call failed"), 8);
}

// --- Regression tests for bug fixes ---

#[test]
fn test_range_loop_int32_slice() {
    let source = r#"
package main

func SumInt32Slice() int {
    s := make([]int32, 0)
    s = append(s, 10)
    s = append(s, 20)
    s = append(s, 30)
    total := 0
    for _, v := range s {
        total = total + int(v)
    }
    return total
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "SumInt32Slice")
        .expect("SumInt32Slice not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 60, "sum of [10, 20, 30] should be 60");
}

#[test]
fn test_make_append_int32_slice() {
    let source = r#"
package main

func AppendInt32() int {
    s := make([]int32, 0)
    s = append(s, 5)
    s = append(s, 15)
    s = append(s, 25)
    return int(len(s))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "AppendInt32")
        .expect("AppendInt32 not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 3, "len should be 3 after 3 appends");
}

#[test]
fn test_slice_expression() {
    let source = r#"
package main

func SliceExpr() int {
    s := make([]int, 0)
    s = append(s, 10)
    s = append(s, 20)
    s = append(s, 30)
    s = append(s, 40)
    s = append(s, 50)
    return int(len(s[1:4]))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "SliceExpr")
        .expect("SliceExpr not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 3, "s[1:4] should have length 3");
}

#[test]
fn test_switch_on_int32() {
    let source = r#"
package main

func SwitchInt32(x int32) int {
    switch x {
    case 1:
        return 10
    case 2:
        return 20
    case 3:
        return 30
    default:
        return -1
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i32, i64>(&mut store, "SwitchInt32")
        .expect("SwitchInt32 not found");
    assert_eq!(func.call(&mut store, 1).expect("call failed"), 10);
    assert_eq!(func.call(&mut store, 2).expect("call failed"), 20);
    assert_eq!(func.call(&mut store, 3).expect("call failed"), 30);
    assert_eq!(func.call(&mut store, 99).expect("call failed"), -1);
}

#[test]
fn test_invalid_field_access_error() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func BadField() int {
    p := Point{X: 1, Y: 2}
    return p.Z
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("unresolved selector"),
                "error should mention unresolved selector, got: {}",
                err_msg
            );
        }
        Ok(_) => panic!("accessing non-existent field 'Z' should produce a compilation error"),
    }
}

#[test]
fn test_function_as_value_error() {
    let source = r#"
package main

func Helper() int {
    return 42
}

func UseFunc() int {
    f := Helper
    return f
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("first-class function values"),
                "error should mention first-class function values, got: {}",
                err_msg
            );
        }
        Ok(_) => panic!("using function name as value should produce a compilation error"),
    }
}

#[test]
fn test_host_function_ctx_log() {
    let source = r#"
package main

func Process(ctx Context, x int) int {
    ctx.Log("hello from wasm")
    return x * 2
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<(i32, i64), i64>(&mut store, "Process")
        .expect("Process not found");
    let result_val = func.call(&mut store, (0, 5)).expect("call failed");
    assert_eq!(result_val, 10);

    let logs = &store.data().logs;
    assert_eq!(logs.len(), 1, "should have 1 log entry");
    assert_eq!(logs[0], "hello from wasm");
}

#[test]
fn test_host_function_ctx_query_id() {
    let source = r#"
package main

func GetQueryLen(ctx Context) int {
    return int(len(ctx.QueryID()))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new()
        .with_query_metadata("test-query-123", "mydb", "public", "alice");
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i32, i64>(&mut store, "GetQueryLen")
        .expect("GetQueryLen not found");
    let result_val = func.call(&mut store, 0).expect("call failed");
    assert_eq!(result_val, 14, "len('test-query-123') should be 14");
}

#[test]
fn test_host_function_ctx_database() {
    let source = r#"
package main

func GetDBLen(ctx Context) int {
    return int(len(ctx.Database()))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new()
        .with_query_metadata("q1", "analytics_db", "main", "bob");
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i32, i64>(&mut store, "GetDBLen")
        .expect("GetDBLen not found");
    let result_val = func.call(&mut store, 0).expect("call failed");
    assert_eq!(result_val, 12, "len('analytics_db') should be 12");
}

#[test]
fn test_host_function_ctx_schema() {
    let source = r#"
package main

func GetSchemaLen(ctx Context) int {
    return int(len(ctx.Schema()))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new()
        .with_query_metadata("q1", "db", "staging", "user1");
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i32, i64>(&mut store, "GetSchemaLen")
        .expect("GetSchemaLen not found");
    let result_val = func.call(&mut store, 0).expect("call failed");
    assert_eq!(result_val, 7, "len('staging') should be 7");
}

#[test]
fn test_host_function_ctx_user() {
    let source = r#"
package main

func GetUserLen(ctx Context) int {
    return int(len(ctx.User()))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new()
        .with_query_metadata("q1", "db", "public", "admin_user");
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i32, i64>(&mut store, "GetUserLen")
        .expect("GetUserLen not found");
    let result_val = func.call(&mut store, 0).expect("call failed");
    assert_eq!(result_val, 10, "len('admin_user') should be 10");
}

#[test]
fn test_switch_inside_for_with_break_continue() {
    let source = r#"
package main

func SwitchInLoop(n int) int {
    total := 0
    for i := 0; i < n; i++ {
        switch {
        case i == 5:
            total = total + 100
        default:
            total = total + i
        }
    }
    return total
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "SwitchInLoop")
        .expect("SwitchInLoop not found");
    // i=0..5: 0+1+2+3+4+100 = 110, i=6..9: 6+7+8+9 = 30. total = 140
    let result_val = func.call(&mut store, 10).expect("call failed");
    assert_eq!(result_val, 140, "0+1+2+3+4+100+6+7+8+9 should be 140");
}
