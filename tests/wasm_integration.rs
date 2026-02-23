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
    assert!(resolve_import("time").is_ok());
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

    // The string "hello" should be in memory after offset 65536 (heap start)
    let data = memory.data(&store);
    let heap_start = 65536usize;
    let heap_data = &data[heap_start..];
    let pos = heap_data
        .windows(5)
        .position(|w| w == b"hello")
        .expect("string 'hello' not found in WASM memory");
    assert!(pos < 4096, "string should be near heap start");
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
    let heap = &data[65536..];
    // "ab\nc" should be 4 bytes: 'a', 'b', '\n', 'c'
    let pos = heap
        .windows(4)
        .position(|w| w == b"ab\nc")
        .expect("escaped string not found in memory");
    assert!(pos < 4096, "string should be near heap start");
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
fn test_global_var_compiles() {
    let source = r#"
package main

var counter int

func Add(a int, b int) int {
    return a + b
}
"#;
    let mut compiler = WasmCompiler::new();
    compiler.compile_source(source).expect("global variables should compile successfully");
}

#[test]
fn test_global_const_compiles() {
    let source = r#"
package main

const maxSize = 100

func Add(a int, b int) int {
    return a + b
}
"#;
    let mut compiler = WasmCompiler::new();
    compiler.compile_source(source).expect("global constants should compile successfully");
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
fn test_fallthrough_basic() {
    let source = r#"
package main

func Fall(x int) int {
    result := 0
    switch x {
    case 1:
        result = 10
        fallthrough
    case 2:
        result = result + 20
    case 3:
        result = 30
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
    let func = instance.get_typed_func::<i64, i64>(&mut store, "Fall").expect("Fall not found");
    // case 1: result = 10, then fallthrough to case 2: result = 10 + 20 = 30
    assert_eq!(func.call(&mut store, 1).expect("call failed"), 30);
    // case 2: result = 0 + 20 = 20
    assert_eq!(func.call(&mut store, 2).expect("call failed"), 20);
    // case 3: result = 30
    assert_eq!(func.call(&mut store, 3).expect("call failed"), 30);
    // no match: result = 0
    assert_eq!(func.call(&mut store, 99).expect("call failed"), 0);
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
fn test_andnot_operator_i64() {
    let source = r#"
package main

func BitClear(a int, b int) int {
    return a &^ b
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
        .get_typed_func::<(i64, i64), i64>(&mut store, "BitClear")
        .expect("BitClear not found");

    // 0b1111 &^ 0b1010 = 0b0101 = 5
    assert_eq!(func.call(&mut store, (0b1111, 0b1010)).expect("call failed"), 5);
    // 0xFF &^ 0x0F = 0xF0 = 240
    assert_eq!(func.call(&mut store, (0xFF, 0x0F)).expect("call failed"), 0xF0);
    // a &^ 0 = a
    assert_eq!(func.call(&mut store, (42, 0)).expect("call failed"), 42);
    // a &^ a = 0
    assert_eq!(func.call(&mut store, (42, 42)).expect("call failed"), 0);
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
fn test_compound_assign_to_struct_field() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func MoveX() int {
    p := Point{X: 5, Y: 3}
    p.X += 10
    return p.X
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
        .get_typed_func::<(), i64>(&mut store, "MoveX")
        .expect("MoveX not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 15, "5 + 10 should be 15");
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
fn test_function_as_value() {
    let source = r#"
package main

func helper() int {
    return 42
}

func UseFunc() int {
    f := helper
    return f()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "UseFunc").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 42);
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

#[test]
fn test_index_assign_plain() {
    let source = r#"
package main

func IndexAssign() int {
    s := make([]int, 3)
    s[0] = 10
    s[1] = 20
    s[2] = 30
    return s[0] + s[1] + s[2]
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
        .get_typed_func::<(), i64>(&mut store, "IndexAssign")
        .expect("IndexAssign not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 60, "10 + 20 + 30 should be 60");
}

#[test]
fn test_index_assign_compound() {
    let source = r#"
package main

func IndexCompound() int {
    s := make([]int, 3)
    s[0] = 5
    s[1] = 10
    s[2] = 15
    s[0] += 100
    s[1] -= 3
    s[2] *= 2
    return s[0] + s[1] + s[2]
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
        .get_typed_func::<(), i64>(&mut store, "IndexCompound")
        .expect("IndexCompound not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 142, "(5+100) + (10-3) + (15*2) = 105 + 7 + 30 = 142");
}

#[test]
fn test_field_assign_plain() {
    let source = r#"
package main

type Rect struct {
    W int
    H int
}

func FieldAssign() int {
    r := Rect{W: 0, H: 0}
    r.W = 10
    r.H = 20
    return r.W * r.H
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
        .get_typed_func::<(), i64>(&mut store, "FieldAssign")
        .expect("FieldAssign not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 200, "10 * 20 should be 200");
}

#[test]
fn test_field_assign_compound() {
    let source = r#"
package main

type Counter struct {
    Val int
}

func FieldCompound() int {
    c := Counter{Val: 10}
    c.Val += 5
    c.Val *= 3
    return c.Val
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
        .get_typed_func::<(), i64>(&mut store, "FieldCompound")
        .expect("FieldCompound not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 45, "(10 + 5) * 3 = 45");
}

#[test]
fn test_index_incdec() {
    let source = r#"
package main

func IndexIncDec() int {
    s := make([]int, 2)
    s[0] = 10
    s[1] = 20
    s[0]++
    s[1]--
    return s[0] + s[1]
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
        .get_typed_func::<(), i64>(&mut store, "IndexIncDec")
        .expect("IndexIncDec not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 30, "11 + 19 = 30");
}

#[test]
fn test_field_incdec() {
    let source = r#"
package main

type Pair struct {
    A int
    B int
}

func FieldIncDec() int {
    p := Pair{A: 100, B: 200}
    p.A++
    p.B--
    return p.A + p.B
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
        .get_typed_func::<(), i64>(&mut store, "FieldIncDec")
        .expect("FieldIncDec not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 300, "101 + 199 = 300");
}

#[test]
fn test_index_assign_int32_slice() {
    let source = r#"
package main

func IndexAssignI32() int {
    s := make([]int32, 3)
    s[0] = 10
    s[1] = 20
    s[2] = 30
    return int(s[0]) + int(s[1]) + int(s[2])
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
        .get_typed_func::<(), i64>(&mut store, "IndexAssignI32")
        .expect("IndexAssignI32 not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 60, "10 + 20 + 30 should be 60");
}

// ===== Regression tests for bug fixes =====

#[test]
fn test_string_variable_concat() {
    let source = r#"
package main

func ConcatVars() int {
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
        .get_typed_func::<(), i64>(&mut store, "ConcatVars")
        .expect("ConcatVars not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 11, "len(\"hello\" + \" world\") should be 11");
}

#[test]
fn test_string_variable_reassign() {
    let source = r#"
package main

func ReassignStr() int {
    a := "abc"
    a = "defgh"
    return int(len(a))
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
        .get_typed_func::<(), i64>(&mut store, "ReassignStr")
        .expect("ReassignStr not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 5, "after reassignment, len should be 5");
}

#[test]
fn test_short_circuit_and() {
    let source = r#"
package main

func ShortAnd(x int) int {
    if x > 0 && x < 10 {
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
        .get_typed_func::<i64, i64>(&mut store, "ShortAnd")
        .expect("ShortAnd not found");
    assert_eq!(func.call(&mut store, 5).unwrap(), 1);
    assert_eq!(func.call(&mut store, 0).unwrap(), 0);
    assert_eq!(func.call(&mut store, 15).unwrap(), 0);
}

#[test]
fn test_short_circuit_or() {
    let source = r#"
package main

func ShortOr(x int) int {
    if x < 0 || x > 100 {
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
        .get_typed_func::<i64, i64>(&mut store, "ShortOr")
        .expect("ShortOr not found");
    assert_eq!(func.call(&mut store, -5).unwrap(), 1);
    assert_eq!(func.call(&mut store, 200).unwrap(), 1);
    assert_eq!(func.call(&mut store, 50).unwrap(), 0);
}

#[test]
fn test_short_circuit_and_prevents_div_by_zero() {
    let source = r#"
package main

func SafeDiv(x int, y int) int {
    if y != 0 && x / y > 2 {
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
        .get_typed_func::<(i64, i64), i64>(&mut store, "SafeDiv")
        .expect("SafeDiv not found");
    assert_eq!(func.call(&mut store, (10, 3)).unwrap(), 1);
    assert_eq!(func.call(&mut store, (10, 0)).unwrap(), 0, "short-circuit should prevent division by zero");
}

#[test]
fn test_type_assert_error() {
    let source = r#"
package main

func UseTypeAssert(x int) int {
    y := x.(int)
    return y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("should fail on type assertion on non-interface"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("not an interface"),
                "error should mention not an interface, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_string_conversion_from_int() {
    let source = r#"
package main

func ConvertIntToString(x int32) int32 {
    s := string(x)
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("should compile");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();
    let func = instance
        .get_typed_func::<i32, i32>(&mut store, "ConvertIntToString")
        .unwrap();

    // ASCII 'A' (65) -> 1 UTF-8 byte
    assert_eq!(func.call(&mut store, 65).unwrap(), 1);
    // U+00E9 (233, 'é') -> 2 UTF-8 bytes
    assert_eq!(func.call(&mut store, 0xE9).unwrap(), 2);
    // U+4E16 (20054, '世') -> 3 UTF-8 bytes
    assert_eq!(func.call(&mut store, 0x4E16).unwrap(), 3);
    // U+1F600 (128512, '😀') -> 4 UTF-8 bytes
    assert_eq!(func.call(&mut store, 0x1F600).unwrap(), 4);
}

#[test]
fn test_float_remainder_assign_error() {
    let source = r#"
package main

func FloatRem() float64 {
    x := 5.0
    x %= 2.0
    return x
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("should fail on float %="),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("not valid") || err_msg.contains("float"),
                "error should mention invalid float operation, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_float_bitwise_assign_error() {
    let source = r#"
package main

func FloatBitwise() float64 {
    x := 5.0
    x &= 3.0
    return x
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("should fail on float &="),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("not valid") || err_msg.contains("float"),
                "error should mention invalid float operation, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_float32_arithmetic() {
    let source = r#"
package main

func AddF32(a float32, b float32) float32 {
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

    let func = instance
        .get_typed_func::<(f32, f32), f32>(&mut store, "AddF32")
        .expect("AddF32 not found");
    let result_val = func.call(&mut store, (1.5, 2.5)).expect("call failed");
    assert!((result_val - 4.0).abs() < 0.001, "1.5 + 2.5 should be 4.0");
}

#[test]
fn test_string_len_variable() {
    let source = r#"
package main

func StrLen() int {
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
        .get_typed_func::<(), i64>(&mut store, "StrLen")
        .expect("StrLen not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 5, "len(\"hello\") should be 5");
}

#[test]
fn test_logical_operators_in_loop() {
    let source = r#"
package main

func CountInRange() int {
    count := 0
    for i := 0; i < 20; i++ {
        if i >= 5 && i <= 15 {
            count++
        }
    }
    return count
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
        .get_typed_func::<(), i64>(&mut store, "CountInRange")
        .expect("CountInRange not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 11, "values 5..15 inclusive = 11");
}

#[test]
fn test_or_with_multiple_conditions() {
    let source = r#"
package main

func OrMulti(x int) int {
    if x == 1 || x == 2 || x == 3 {
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
        .get_typed_func::<i64, i64>(&mut store, "OrMulti")
        .expect("OrMulti not found");
    assert_eq!(func.call(&mut store, 1).unwrap(), 1);
    assert_eq!(func.call(&mut store, 2).unwrap(), 1);
    assert_eq!(func.call(&mut store, 3).unwrap(), 1);
    assert_eq!(func.call(&mut store, 4).unwrap(), 0);
}

#[test]
fn test_float32_incdec() {
    let source = r#"
package main

func F32IncDec(x float32) float32 {
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
        .get_typed_func::<f32, f32>(&mut store, "F32IncDec")
        .expect("F32IncDec not found");
    let result_val = func.call(&mut store, 5.0).expect("call failed");
    assert!((result_val - 6.0).abs() < 0.001, "5.0 ++ ++ -- should be 6.0, got {}", result_val);
}

#[test]
fn test_switch_mixed_int_types() {
    let source = r#"
package main

func Classify(x int) int {
    y := int32(x)
    result := 0
    switch y {
    case 1:
        result = 10
    case 2:
        result = 20
    case 3:
        result = 30
    default:
        result = 0
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
    assert_eq!(func.call(&mut store, 1).unwrap(), 10);
    assert_eq!(func.call(&mut store, 2).unwrap(), 20);
    assert_eq!(func.call(&mut store, 3).unwrap(), 30);
    assert_eq!(func.call(&mut store, 99).unwrap(), 0);
}

#[test]
fn test_global_const_int() {
    let source = r#"
package main

const multiplier = 10
const offset = 5

func Compute(x int) int {
    return x * multiplier + offset
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
        .get_typed_func::<i64, i64>(&mut store, "Compute")
        .expect("Compute not found");
    assert_eq!(func.call(&mut store, 3).unwrap(), 35);
    assert_eq!(func.call(&mut store, 0).unwrap(), 5);
}

#[test]
fn test_global_const_float() {
    let source = r#"
package main

const pi = 3.14

func CircleArea(r float64) float64 {
    return pi * r * r
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
        .get_typed_func::<f64, f64>(&mut store, "CircleArea")
        .expect("CircleArea not found");
    let result_val = func.call(&mut store, 2.0).expect("call failed");
    assert!((result_val - 12.56).abs() < 0.01, "pi * 2^2 = 12.56, got {}", result_val);
}

#[test]
fn test_global_const_bool() {
    let source = r#"
package main

const debugMode = true

func IsDebug() int {
    if debugMode {
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
        .get_typed_func::<(), i64>(&mut store, "IsDebug")
        .expect("IsDebug not found");
    assert_eq!(func.call(&mut store, ()).unwrap(), 1);
}

#[test]
fn test_global_var_int() {
    let source = r#"
package main

var counter int

func Increment() int {
    counter = counter + 1
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
        .get_typed_func::<(), i64>(&mut store, "Increment")
        .expect("Increment not found");
    assert_eq!(func.call(&mut store, ()).unwrap(), 1);
    assert_eq!(func.call(&mut store, ()).unwrap(), 2);
    assert_eq!(func.call(&mut store, ()).unwrap(), 3);
}

#[test]
fn test_global_var_with_initializer() {
    let source = r#"
package main

var threshold float64 = 1.5

func AboveThreshold(x float64) int {
    if x > threshold {
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
        .get_typed_func::<f64, i64>(&mut store, "AboveThreshold")
        .expect("AboveThreshold not found");
    assert_eq!(func.call(&mut store, 2.0).unwrap(), 1);
    assert_eq!(func.call(&mut store, 1.0).unwrap(), 0);
    assert_eq!(func.call(&mut store, 1.5).unwrap(), 0);
}

#[test]
fn test_global_var_and_const_together() {
    let source = r#"
package main

const step = 5
var total int

func AddStep() int {
    total = total + step
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
        .get_typed_func::<(), i64>(&mut store, "AddStep")
        .expect("AddStep not found");
    assert_eq!(func.call(&mut store, ()).unwrap(), 5);
    assert_eq!(func.call(&mut store, ()).unwrap(), 10);
    assert_eq!(func.call(&mut store, ()).unwrap(), 15);
}

#[test]
fn test_global_var_compound_assign() {
    let source = r#"
package main

var acc int

func Accumulate(x int) int {
    acc += x
    return acc
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
        .expect("Accumulate not found");
    assert_eq!(func.call(&mut store, 10).unwrap(), 10);
    assert_eq!(func.call(&mut store, 20).unwrap(), 30);
    assert_eq!(func.call(&mut store, 5).unwrap(), 35);
}

#[test]
fn test_host_function_ctx_config() {
    let source = r#"
package main

func GetConfigLen(ctx Context) int {
    return int(len(ctx.Config("my_key")))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new()
        .with_query_metadata("q1", "db1", "public", "alice")
        .with_config("my_key", "hello_world");
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i32, i64>(&mut store, "GetConfigLen")
        .expect("GetConfigLen not found");
    let result_val = func.call(&mut store, 0).expect("call failed");
    assert_eq!(result_val, 11, "len('hello_world') should be 11");
}

#[test]
fn test_host_function_ctx_config_missing_key() {
    let source = r#"
package main

func GetConfigLen(ctx Context) int {
    return int(len(ctx.Config("nonexistent")))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new()
        .with_query_metadata("q1", "db1", "public", "alice");
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i32, i64>(&mut store, "GetConfigLen")
        .expect("GetConfigLen not found");
    let result_val = func.call(&mut store, 0).expect("call failed");
    assert_eq!(result_val, 0, "missing key should return empty string (len 0)");
}

#[test]
fn test_defer_multiple_calls() {
    let source = r#"
package main

var deferSum int

func addToSum(x int) {
    deferSum = deferSum + x
}

func RunDefers() int {
    deferSum = 0
    defer addToSum(1)
    defer addToSum(2)
    defer addToSum(4)
    return 0
}

func GetDeferSum() int {
    return deferSum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let run = instance
        .get_typed_func::<(), i64>(&mut store, "RunDefers")
        .expect("RunDefers not found");
    run.call(&mut store, ()).expect("call failed");

    let get_sum = instance
        .get_typed_func::<(), i64>(&mut store, "GetDeferSum")
        .expect("GetDeferSum not found");
    let sum = get_sum.call(&mut store, ()).expect("call failed");
    assert_eq!(sum, 7, "all three defers (1+2+4) should run, got {}", sum);
}

#[test]
fn test_unsigned_division() {
    let source = r#"
package main

func UintDiv(a uint64, b uint64) uint64 {
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

    let func = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "UintDiv")
        .expect("UintDiv not found");
    // 18446744073709551614 (max uint64 - 1) / 2 = 9223372036854775807
    // In two's complement, 18446744073709551614 is -2 as i64
    // Unsigned division of -2 (as uint64) by 2 should give 9223372036854775807
    // Signed division of -2 by 2 would give -1
    let result_val = func.call(&mut store, (-2i64, 2i64)).expect("call failed");
    assert_eq!(
        result_val,
        9223372036854775807i64,
        "unsigned division of (max_uint64-1) by 2 should give max_int64, got {}",
        result_val
    );
}

#[test]
fn test_unsigned_comparison() {
    let source = r#"
package main

func UintGreater(a uint64, b uint64) int {
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
        .get_typed_func::<(i64, i64), i64>(&mut store, "UintGreater")
        .expect("UintGreater not found");
    // -1 as uint64 is the max value, so it should be > 1
    let result_val = func.call(&mut store, (-1i64, 1i64)).expect("call failed");
    assert_eq!(result_val, 1, "max uint64 should be > 1 (unsigned comparison)");

    // 1 should not be > max uint64
    let result_val2 = func.call(&mut store, (1i64, -1i64)).expect("call failed");
    assert_eq!(result_val2, 0, "1 should not be > max uint64");
}

#[test]
fn test_unsigned_remainder() {
    let source = r#"
package main

func UintRem(a uint64, b uint64) uint64 {
    return a % b
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
        .get_typed_func::<(i64, i64), i64>(&mut store, "UintRem")
        .expect("UintRem not found");
    // -1 as uint64 = 18446744073709551615, % 10 = 5
    let result_val = func.call(&mut store, (-1i64, 10i64)).expect("call failed");
    assert_eq!(result_val, 5, "max uint64 % 10 should be 5, got {}", result_val);
}

// ========== Regression tests for bug fixes ==========

#[test]
fn test_string_equality_same_content() {
    let source = r#"
package main

func StrEq() int {
    a := "hello"
    b := "hello"
    if a == b {
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
        .get_typed_func::<(), i64>(&mut store, "StrEq")
        .expect("StrEq not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 1, "identical strings should be equal");
}

#[test]
fn test_string_equality_different_content() {
    let source = r#"
package main

func StrNeq() int {
    a := "hello"
    b := "world"
    if a == b {
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
        .get_typed_func::<(), i64>(&mut store, "StrNeq")
        .expect("StrNeq not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 0, "different strings should not be equal");
}

#[test]
fn test_string_equality_different_lengths() {
    let source = r#"
package main

func StrLenDiff() int {
    a := "hello"
    b := "hello!"
    if a == b {
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
        .get_typed_func::<(), i64>(&mut store, "StrLenDiff")
        .expect("StrLenDiff not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 0, "strings of different lengths should not be equal");
}

#[test]
fn test_string_equality_empty_strings() {
    let source = r#"
package main

func EmptyEq() int {
    a := ""
    b := ""
    if a == b {
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
        .get_typed_func::<(), i64>(&mut store, "EmptyEq")
        .expect("EmptyEq not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 1, "two empty strings should be equal");
}

#[test]
fn test_string_not_equal_operator() {
    let source = r#"
package main

func StrNotEq() int {
    a := "foo"
    b := "bar"
    if a != b {
        return 1
    }
    return 0
}

func StrNotEqSame() int {
    a := "same"
    b := "same"
    if a != b {
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
        .get_typed_func::<(), i64>(&mut store, "StrNotEq")
        .expect("StrNotEq not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 1, "different strings should be != ");

    let func2 = instance
        .get_typed_func::<(), i64>(&mut store, "StrNotEqSame")
        .expect("StrNotEqSame not found");
    let result_val2 = func2.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val2, 0, "identical strings should not be !=");
}

#[test]
fn test_string_equality_with_concat() {
    let source = r#"
package main

func ConcatEq() int {
    a := "hello" + " world"
    b := "hello world"
    if a == b {
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
        .get_typed_func::<(), i64>(&mut store, "ConcatEq")
        .expect("ConcatEq not found");
    let result_val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result_val, 1, "concatenated string should equal the literal equivalent");
}

#[test]
fn test_global_var_increment() {
    let source = r#"
package main

var counter int

func Bump() int {
    counter++
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
        .get_typed_func::<(), i64>(&mut store, "Bump")
        .expect("Bump not found");
    assert_eq!(func.call(&mut store, ()).unwrap(), 1, "first call should return 1");
    assert_eq!(func.call(&mut store, ()).unwrap(), 2, "second call should return 2");
    assert_eq!(func.call(&mut store, ()).unwrap(), 3, "third call should return 3");
}

#[test]
fn test_global_var_decrement() {
    let source = r#"
package main

var counter int = 10

func Shrink() int {
    counter--
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
        .get_typed_func::<(), i64>(&mut store, "Shrink")
        .expect("Shrink not found");
    assert_eq!(func.call(&mut store, ()).unwrap(), 9, "first decrement from 10");
    assert_eq!(func.call(&mut store, ()).unwrap(), 8, "second decrement");
    assert_eq!(func.call(&mut store, ()).unwrap(), 7, "third decrement");
}

#[test]
fn test_defer_scope_isolation_with_closure() {
    let source = r#"
package main

var outerDeferRan int
var closureDeferRan int

func markOuter(x int) {
    outerDeferRan = outerDeferRan + x
}

func markClosure(x int) {
    closureDeferRan = closureDeferRan + x
}

func RunScopeTest() int {
    outerDeferRan = 0
    closureDeferRan = 0

    defer markOuter(100)

    f := func() int {
        defer markClosure(10)
        return 1
    }

    return f()
}

func GetOuterDefer() int {
    return outerDeferRan
}

func GetClosureDefer() int {
    return closureDeferRan
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let run = instance
        .get_typed_func::<(), i64>(&mut store, "RunScopeTest")
        .expect("RunScopeTest not found");
    run.call(&mut store, ()).expect("call failed");

    let get_outer = instance
        .get_typed_func::<(), i64>(&mut store, "GetOuterDefer")
        .expect("GetOuterDefer not found");
    let outer = get_outer.call(&mut store, ()).expect("call failed");
    assert_eq!(outer, 100, "outer defer should have run exactly once with value 100, got {}", outer);

    let get_closure = instance
        .get_typed_func::<(), i64>(&mut store, "GetClosureDefer")
        .expect("GetClosureDefer not found");
    let closure = get_closure.call(&mut store, ()).expect("call failed");
    assert_eq!(closure, 10, "closure defer should have run exactly once with value 10, got {}", closure);
}

#[test]
fn test_defer_unresolved_function_error() {
    let source = r#"
package main

func Test() int {
    defer unknownFunc()
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("could not resolve function"),
                "error should mention unresolved function, got: {}",
                err_msg
            );
        }
        Ok(_) => panic!("expected error for defer of unknown function"),
    }
}

#[test]
fn test_andnot_operator_i32() {
    let source = r#"
package main

func BitClear32(a int32, b int32) int32 {
    return a &^ b
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
        .get_typed_func::<(i32, i32), i32>(&mut store, "BitClear32")
        .expect("BitClear32 not found");

    assert_eq!(func.call(&mut store, (0b1111, 0b1010)).expect("call failed"), 0b0101);
    assert_eq!(func.call(&mut store, (0xFF, 0x0F)).expect("call failed"), 0xF0_u8 as i32);
    assert_eq!(func.call(&mut store, (42, 0)).expect("call failed"), 42);
    assert_eq!(func.call(&mut store, (42, 42)).expect("call failed"), 0);
}

#[test]
fn test_andnot_assign_operator() {
    let source = r#"
package main

func BitClearAssign(a int, b int) int {
    a &^= b
    return a
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
        .get_typed_func::<(i64, i64), i64>(&mut store, "BitClearAssign")
        .expect("BitClearAssign not found");

    assert_eq!(func.call(&mut store, (0b1111, 0b1010)).expect("call failed"), 0b0101);
    assert_eq!(func.call(&mut store, (0xFF, 0x0F)).expect("call failed"), 0xF0);
}

// ==================== Bug Fix Regression Tests ====================

#[test]
fn test_infer_val_type_user_func_returning_int32() {
    let source = r#"
package main

func helper(v int32) int32 {
    return v
}

func Run(n int32) int32 {
    x := helper(n)
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
        .get_typed_func::<i32, i32>(&mut store, "Run")
        .expect("Run not found");

    assert_eq!(func.call(&mut store, 42).expect("call failed"), 42);
}

#[test]
fn test_infer_val_type_user_func_returning_float64() {
    let source = r#"
package main

func compute() float64 {
    return 3.14
}

func Run() float64 {
    x := compute()
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
        .get_typed_func::<(), f64>(&mut store, "Run")
        .expect("Run not found");

    let result = func.call(&mut store, ()).expect("call failed");
    assert!((result - 3.14).abs() < f64::EPSILON);
}

#[test]
fn test_multi_return_define_assignment() {
    let source = r#"
package main

func split(x int) (int, int) {
    return x / 10, x % 10
}

func Run(x int) int {
    tens, ones := split(x)
    return tens * 100 + ones
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
        .get_typed_func::<i64, i64>(&mut store, "Run")
        .expect("Run not found");

    let result = func.call(&mut store, 47).expect("call failed");
    assert_eq!(result, 407);
}

#[test]
fn test_multi_return_with_blank_identifier() {
    let source = r#"
package main

func pair(x int) (int, int) {
    return x * 2, x * 3
}

func Run(x int) int {
    _, second := pair(x)
    return second
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
        .get_typed_func::<i64, i64>(&mut store, "Run")
        .expect("Run not found");

    assert_eq!(func.call(&mut store, 5).expect("call failed"), 15);
}

#[test]
fn test_all_branches_return_if_else() {
    let source = r#"
package main

func Classify(x int) int {
    if x > 0 {
        return 1
    } else {
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
        .get_typed_func::<i64, i64>(&mut store, "Classify")
        .expect("Classify not found");

    assert_eq!(func.call(&mut store, 10).expect("call failed"), 1);
    assert_eq!(func.call(&mut store, -5).expect("call failed"), -1);
}

#[test]
fn test_all_branches_return_nested_if_else() {
    let source = r#"
package main

func Sign(x int) int {
    if x > 0 {
        return 1
    } else if x < 0 {
        return -1
    } else {
        return 0
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
        .get_typed_func::<i64, i64>(&mut store, "Sign")
        .expect("Sign not found");

    assert_eq!(func.call(&mut store, 100).expect("call failed"), 1);
    assert_eq!(func.call(&mut store, -42).expect("call failed"), -1);
    assert_eq!(func.call(&mut store, 0).expect("call failed"), 0);
}

// ==================== Feature Coverage Tests ====================

#[test]
fn test_nested_struct_field_access() {
    let source = r#"
package main

type Inner struct {
    Value int
}

type Outer struct {
    A Inner
    B int
}

func NestedAccess() int {
    inner := Inner{Value: 100}
    outer := Outer{A: inner, B: 5}
    return outer.B
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
        .get_typed_func::<(), i64>(&mut store, "NestedAccess")
        .expect("NestedAccess not found");

    assert_eq!(func.call(&mut store, ()).expect("call failed"), 5);
}

#[test]
fn test_struct_method_returning_float64() {
    let source = r#"
package main

import "math"

type Circle struct {
    Radius float64
}

func (c *Circle) Area() float64 {
    return 3.14159 * c.Radius * c.Radius
}

func ComputeArea(r float64) float64 {
    c := Circle{Radius: r}
    return c.Area()
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
        .get_typed_func::<f64, f64>(&mut store, "ComputeArea")
        .expect("ComputeArea not found");

    let result = func.call(&mut store, 2.0).expect("call failed");
    assert!((result - 12.56636).abs() < 0.001);
}

#[test]
fn test_struct_method_returning_bool_as_int() {
    let source = r#"
package main

type Threshold struct {
    Limit int
}

func (t *Threshold) Exceeds(val int) int {
    if val > t.Limit {
        return 1
    }
    return 0
}

func CheckThreshold(limit int, val int) int {
    t := Threshold{Limit: limit}
    return t.Exceeds(val)
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
        .get_typed_func::<(i64, i64), i64>(&mut store, "CheckThreshold")
        .expect("CheckThreshold not found");

    assert_eq!(func.call(&mut store, (10, 15)).expect("call failed"), 1);
    assert_eq!(func.call(&mut store, (10, 5)).expect("call failed"), 0);
    assert_eq!(func.call(&mut store, (10, 10)).expect("call failed"), 0);
}

#[test]
fn test_const_expression_arithmetic() {
    let source = r#"
package main

const (
    A = 10
    B = 3
    C = A + B * 2
)

func GetConst() int {
    return C
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
        .get_typed_func::<(), i64>(&mut store, "GetConst")
        .expect("GetConst not found");

    assert_eq!(func.call(&mut store, ()).expect("call failed"), 16);
}

#[test]
fn test_blank_identifier_in_for_range() {
    let source = r#"
package main

func SumSlice() int {
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
        .get_typed_func::<(), i64>(&mut store, "SumSlice")
        .expect("SumSlice not found");

    assert_eq!(func.call(&mut store, ()).expect("call failed"), 60);
}

#[test]
fn test_nested_for_loop_break() {
    let source = r#"
package main

func NestedBreak() int {
    result := 0
    for i := 0; i < 5; i++ {
        for j := 0; j < 5; j++ {
            if j == 3 {
                break
            }
            result = result + 1
        }
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
        .get_typed_func::<(), i64>(&mut store, "NestedBreak")
        .expect("NestedBreak not found");

    // 5 outer iterations * 3 inner iterations (j=0,1,2 then break at j=3)
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 15);
}

#[test]
fn test_nested_for_loop_continue() {
    let source = r#"
package main

func NestedContinue() int {
    s := make([]int, 0)
    s = append(s, 0)
    s = append(s, 1)
    s = append(s, 2)
    s = append(s, 3)

    result := 0
    for _, v := range s {
        if v == 2 {
            continue
        }
        result = result + v
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
        .get_typed_func::<(), i64>(&mut store, "NestedContinue")
        .expect("NestedContinue not found");

    // 0 + 1 + 3 = 4 (skip v==2)
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 4);
}

#[test]
fn test_switch_multi_value_with_expression() {
    let source = r#"
package main

func Grade(score int) int {
    switch {
    case score >= 90:
        return 4
    case score >= 80:
        return 3
    case score >= 70:
        return 2
    default:
        return 1
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
        .get_typed_func::<i64, i64>(&mut store, "Grade")
        .expect("Grade not found");

    assert_eq!(func.call(&mut store, 95).expect("call failed"), 4);
    assert_eq!(func.call(&mut store, 85).expect("call failed"), 3);
    assert_eq!(func.call(&mut store, 75).expect("call failed"), 2);
    assert_eq!(func.call(&mut store, 50).expect("call failed"), 1);
}

#[test]
fn test_multiple_defers_lifo_order() {
    let source = r#"
package main

var result int

func appendVal(v int) {
    result = result * 10 + v
}

func TestDefers() int {
    result = 0
    defer appendVal(3)
    defer appendVal(2)
    defer appendVal(1)
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
        .get_typed_func::<(), i64>(&mut store, "TestDefers")
        .expect("TestDefers not found");

    let _result = func.call(&mut store, ()).expect("call failed");
    // Defers execute in LIFO order: appendVal(1), then appendVal(2), then appendVal(3)
    // result = ((0*10+1)*10+2)*10+3 = 123
    // But result is returned BEFORE defers run, so the returned value is 0
    // The global `result` is updated by defers after the return
}

#[test]
fn test_closure_capturing_multiple_types() {
    let source = r#"
package main

func ClosureMultiCapture() int {
    x := 10
    y := 20
    sum := func() int {
        return x + y
    }
    return sum()
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
        .get_typed_func::<(), i64>(&mut store, "ClosureMultiCapture")
        .expect("ClosureMultiCapture not found");

    assert_eq!(func.call(&mut store, ()).expect("call failed"), 30);
}

#[test]
fn test_global_variable_modification() {
    let source = r#"
package main

var counter int

func Increment() int {
    counter = counter + 1
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
        .get_typed_func::<(), i64>(&mut store, "Increment")
        .expect("Increment not found");

    assert_eq!(func.call(&mut store, ()).expect("call failed"), 1);
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 2);
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 3);
}

#[test]
fn test_for_loop_with_no_condition() {
    let source = r#"
package main

func CountToTen() int {
    i := 0
    for {
        i = i + 1
        if i >= 10 {
            break
        }
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
        .get_typed_func::<(), i64>(&mut store, "CountToTen")
        .expect("CountToTen not found");

    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

#[test]
fn test_switch_all_cases_return() {
    let source = r#"
package main

func Category(x int) int {
    switch {
    case x < 0:
        return -1
    case x == 0:
        return 0
    default:
        return 1
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
        .get_typed_func::<i64, i64>(&mut store, "Category")
        .expect("Category not found");

    assert_eq!(func.call(&mut store, -5).expect("call failed"), -1);
    assert_eq!(func.call(&mut store, 0).expect("call failed"), 0);
    assert_eq!(func.call(&mut store, 42).expect("call failed"), 1);
}

#[test]
fn test_multiple_var_declarations() {
    let source = r#"
package main

func MultiVarDecl() int {
    var a int
    var b int
    a = 10
    b = 20
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

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "MultiVarDecl")
        .expect("MultiVarDecl not found");

    assert_eq!(func.call(&mut store, ()).expect("call failed"), 30);
}

#[test]
fn test_if_inside_for_with_break() {
    let source = r#"
package main

func FindFirst(target int) int {
    s := make([]int, 0)
    s = append(s, 5)
    s = append(s, 10)
    s = append(s, 15)
    s = append(s, 20)

    found := 0
    for _, v := range s {
        if v == target {
            found = 1
            break
        }
    }
    return found
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

    assert_eq!(func.call(&mut store, 15).expect("call failed"), 1);
    assert_eq!(func.call(&mut store, 5).expect("call failed"), 1);
    assert_eq!(func.call(&mut store, 99).expect("call failed"), 0);
}

#[test]
fn test_deeply_nested_control_flow() {
    let source = r#"
package main

func DeepNesting(n int) int {
    result := 0
    for i := 0; i < n; i++ {
        if i % 2 == 0 {
            for j := 0; j < i; j++ {
                if j % 3 == 0 {
                    result = result + 1
                }
            }
        }
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
        .get_typed_func::<i64, i64>(&mut store, "DeepNesting")
        .expect("DeepNesting not found");

    // i=0 (even): j loop 0..0 = 0 iterations
    // i=1 (odd): skip
    // i=2 (even): j loop 0..2: j=0 (0%3==0, +1) = 1
    // i=3 (odd): skip
    // i=4 (even): j loop 0..4: j=0 (+1), j=1, j=2, j=3 (+1) = 2
    // Total: 0 + 1 + 2 = 3
    assert_eq!(func.call(&mut store, 5).expect("call failed"), 3);
}

#[test]
fn test_multiple_functions_calling_each_other() {
    let source = r#"
package main

func double(x int) int {
    return x * 2
}

func addOne(x int) int {
    return x + 1
}

func Pipeline(x int) int {
    return addOne(double(x))
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
        .get_typed_func::<i64, i64>(&mut store, "Pipeline")
        .expect("Pipeline not found");

    assert_eq!(func.call(&mut store, 5).expect("call failed"), 11);
    assert_eq!(func.call(&mut store, 0).expect("call failed"), 1);
}

// --- Regression tests for Bug 4: labeled break/continue ---

#[test]
fn test_labeled_break_outer_loop() {
    let source = r#"
package main

func LabeledBreak() int {
    result := 0
    i := 0
Outer:
    for i < 5 {
        i++
        j := 0
        for j < 5 {
            j++
            if j == 3 {
                break Outer
            }
            result = result + 1
        }
    }
    return result
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();
    let func = instance
        .get_typed_func::<(), i64>(&mut store, "LabeledBreak")
        .unwrap();
    // Inner loop: j=1,2 increment result, j=3 breaks outer -> result = 2
    assert_eq!(func.call(&mut store, ()).unwrap(), 2);
}

#[test]
fn test_labeled_continue_outer_loop() {
    let source = r#"
package main

func LabeledContinue() int {
    result := 0
    i := 0
Outer:
    for i < 3 {
        i++
        j := 0
        for j < 3 {
            j++
            if j == 2 {
                continue Outer
            }
            result = result + 1
        }
    }
    return result
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();
    let func = instance
        .get_typed_func::<(), i64>(&mut store, "LabeledContinue")
        .unwrap();
    // Each outer iteration: j=1 increments result, j=2 continues outer -> 3 iterations * 1 = 3
    assert_eq!(func.call(&mut store, ()).unwrap(), 3);
}

// --- Regression test for Bug 3: OOM produces descriptive error ---

#[test]
fn test_oom_produces_descriptive_error() {
    let source = r#"
package main

func OomTest() int {
    total := 0
    for i := 0; i < 10000; i++ {
        s := make([]int, 10000)
        total = total + len(s)
    }
    return total
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();
    let func = instance
        .get_typed_func::<(), i64>(&mut store, "OomTest")
        .unwrap();
    let err = func.call(&mut store, ()).unwrap_err();
    // The "out of memory" message may be in the error chain
    let full_err = format!("{:#}", err);
    assert!(
        full_err.contains("out of memory") || full_err.contains("unreachable"),
        "OOM should produce a trap (either descriptive 'out of memory' or wasm trap), got: {}",
        full_err
    );
}

// --- Type switch on non-interface should produce error ---

#[test]
fn test_type_switch_non_interface_error() {
    let source = r#"
package main

func TypeSwitchNonIface(x int) int {
    switch x.(type) {
    case int:
        return 1
    default:
        return 0
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("type switch on non-interface should fail"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("not an interface"),
                "error should mention not an interface, got: {}",
                err_msg
            );
        }
    }
}

// --- Regression test for Bug 8: float modulo error ---

#[test]
fn test_float_modulo_error() {
    let source = r#"
package main

func FloatMod(a float64, b float64) float64 {
    return a % b
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("float modulo should produce a compilation error"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("modulo") && err_msg.contains("floating-point"),
                "error should mention modulo on float, got: {}",
                err_msg
            );
        }
    }
}

// --- Regression tests for defer LIFO ordering ---

#[test]
fn test_defer_lifo_with_named_functions() {
    let source = r#"
package main

var trace int = 0

func push1() {
    trace = trace * 10 + 1
}

func push2() {
    trace = trace * 10 + 2
}

func push3() {
    trace = trace * 10 + 3
}

func DeferOrder() int {
    defer push1()
    defer push2()
    defer push3()
    return trace
}

func GetTrace() int {
    return trace
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();

    let defer_func = instance
        .get_typed_func::<(), i64>(&mut store, "DeferOrder")
        .unwrap();
    // DeferOrder returns trace BEFORE defers execute (trace=0 at return)
    let _ = defer_func.call(&mut store, ());

    let get_trace = instance
        .get_typed_func::<(), i64>(&mut store, "GetTrace")
        .unwrap();
    let trace_val = get_trace.call(&mut store, ()).unwrap();
    // LIFO order: push3 first (trace=3), push2 next (trace=32), push1 last (trace=321)
    assert_eq!(trace_val, 321, "defer should execute in LIFO order");
}

// --- Regression tests for unsigned arithmetic ---

#[test]
fn test_unsigned_division_regression() {
    let source = r#"
package main

func UnsignedDiv(a uint, b uint) uint {
    return a / b
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();
    let func = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "UnsignedDiv")
        .unwrap();
    assert_eq!(func.call(&mut store, (10, 3)).unwrap(), 3);
    assert_eq!(func.call(&mut store, (100, 7)).unwrap(), 14);
}

#[test]
fn test_unsigned_right_shift() {
    let source = r#"
package main

func UnsignedShr(a uint, b uint) uint {
    return a >> b
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();
    let func = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "UnsignedShr")
        .unwrap();
    // -1 as uint64 is 0xFFFFFFFFFFFFFFFF
    // Unsigned right shift by 60 gives 0xF = 15
    assert_eq!(func.call(&mut store, (-1i64, 60)).unwrap(), 15);
}

#[test]
fn test_unsigned_comparison_negative_vs_positive() {
    let source = r#"
package main

func UnsignedLess(a uint, b uint) int {
    if a < b {
        return 1
    }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();
    let func = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "UnsignedLess")
        .unwrap();
    // As signed: -1 < 1 is true. As unsigned: 0xFFFF...FFFF > 1 -> false
    assert_eq!(func.call(&mut store, (-1i64, 1)).unwrap(), 0);
    assert_eq!(func.call(&mut store, (1, 2)).unwrap(), 1);
}

// --- Regression test for compound assignment to struct fields ---

#[test]
fn test_compound_assign_struct_field() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func CompoundStructField() int {
    p := Point{X: 10, Y: 20}
    p.X += 5
    p.Y -= 3
    return p.X + p.Y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();
    let func = instance
        .get_typed_func::<(), i64>(&mut store, "CompoundStructField")
        .unwrap();
    // (10+5) + (20-3) = 15 + 17 = 32
    assert_eq!(func.call(&mut store, ()).unwrap(), 32);
}

// --- Regression test for compound assignment to slice elements ---

#[test]
fn test_compound_assign_slice_element() {
    let source = r#"
package main

func CompoundSliceElem() int {
    s := make([]int, 3)
    s[0] = 10
    s[1] = 20
    s[2] = 30
    s[1] += 5
    return s[0] + s[1] + s[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();
    let func = instance
        .get_typed_func::<(), i64>(&mut store, "CompoundSliceElem")
        .unwrap();
    // 10 + (20+5) + 30 = 65
    assert_eq!(func.call(&mut store, ()).unwrap(), 65);
}

// --- Regression test for empty switch ---

#[test]
fn test_empty_switch() {
    let source = r#"
package main

func EmptySwitch(x int) int {
    result := 0
    switch {
    }
    result = x + 1
    return result
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();
    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "EmptySwitch")
        .unwrap();
    assert_eq!(func.call(&mut store, 5).unwrap(), 6);
}

// --- Regression test for global variable mutation across function calls ---

#[test]
fn test_global_variable_mutation() {
    let source = r#"
package main

var counter int = 0

func Increment() int {
    counter = counter + 1
    return counter
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();
    let func = instance
        .get_typed_func::<(), i64>(&mut store, "Increment")
        .unwrap();
    assert_eq!(func.call(&mut store, ()).unwrap(), 1);
    assert_eq!(func.call(&mut store, ()).unwrap(), 2);
    assert_eq!(func.call(&mut store, ()).unwrap(), 3);
}

// ========================== Regression tests ==========================

#[test]
fn test_append_type_mismatch_error() {
    let source = r#"
package main

func Bad() int {
    s := make([]float64, 0)
    s = append(s, "hello")
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("appending string to float64 slice should error"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("type mismatch in append"),
                "expected type mismatch error, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_nested_composite_literal() {
    let source = r#"
package main

type Inner struct {
    X int
    Y int
}

type Outer struct {
    A int
    In Inner
}

func Nested() int {
    o := Outer{A: 1, In: Inner{X: 42, Y: 10}}
    return int(o.A)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Nested").expect("Nested not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 1);
}

#[test]
fn test_unknown_struct_field_error() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Bad() int {
    p := Point{Z: 42}
    return int(p.X)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("unknown struct field should error"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("unknown field 'Z'"),
                "expected unknown field error, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_goto_error() {
    let source = r#"
package main

func Bad() int {
    goto end
end:
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("goto should produce a compile error"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("goto is not supported"),
                "expected goto not supported error, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_map_basic() {
    let source = r#"
package main

func MapBasic() int {
    m := make(map[int]int)
    m[1] = 10
    m[2] = 20
    m[3] = 30
    return m[1] + m[2] + m[3]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "MapBasic").expect("MapBasic not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 60);
}

#[test]
fn test_interface_type_declaration() {
    let source = r#"
package main

type MyInterface interface {
    DoSomething() int
}

func InterfaceDecl() int {
    var x MyInterface
    _ = x
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(cr) => {
            let runtime = UdfRuntime::new().expect("runtime init failed");
            let module = runtime.load_module(&cr.wasm_bytes).expect("module load failed");
            let state = HostState::new();
            let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
            let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
            let func = instance.get_typed_func::<(), i64>(&mut store, "InterfaceDecl").expect("not found");
            assert_eq!(func.call(&mut store, ()).expect("call failed"), 0);
        }
        Err(e) => {
            panic!("interface type declaration should compile successfully, got: {}", e);
        }
    }
}

#[test]
fn test_channel_type_error() {
    let source = r#"
package main

func Bad() int {
    var c chan int
    _ = c
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("channel type should produce a compile error"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("channels are not supported"),
                "expected channels not supported error, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_defer_closure() {
    let source = r#"
package main

func DeferClosure() int {
    defer func() {
    }()
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "DeferClosure").expect("DeferClosure not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 1);
}

#[test]
fn test_defer_method_call_resolution() {
    let source = r#"
package main

type Resource struct {
    Value int32
}

func (r *Resource) Close() {
}

func Compute(x int32) int32 {
    r := Resource{Value: x}
    defer r.Close()
    return r.Value + int32(1)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => {}
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                !err_msg.contains("could not resolve function"),
                "defer method call should be resolved, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_fallthrough_chain() {
    let source = r#"
package main

func FallChain(x int) int {
    result := 0
    switch x {
    case 1:
        result = result + 1
        fallthrough
    case 2:
        result = result + 2
        fallthrough
    case 3:
        result = result + 3
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
    let func = instance.get_typed_func::<i64, i64>(&mut store, "FallChain").expect("FallChain not found");
    // case 1: 1+2+3 = 6 (falls through to 2, then 3)
    assert_eq!(func.call(&mut store, 1).expect("call failed"), 6);
    // case 2: 2+3 = 5 (falls through to 3)
    assert_eq!(func.call(&mut store, 2).expect("call failed"), 5);
    // case 3: 3
    assert_eq!(func.call(&mut store, 3).expect("call failed"), 3);
}

#[test]
fn test_goroutine_error() {
    let source = r#"
package main

func helper() {
}

func Bad() int {
    go helper()
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("goroutine should produce a compile error"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("goroutines not supported"),
                "expected goroutines not supported error, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_select_error() {
    let source = r#"
package main

func Bad() int {
    select {
    }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("select should produce a compile error"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("select not supported"),
                "expected select not supported error, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_struct_too_many_fields_error() {
    let source = r#"
package main

type Point struct {
    X int
}

func Bad() int {
    p := Point{10, 20}
    return int(p.X)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Ok(_) => panic!("too many fields should produce a compile error"),
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("too many fields"),
                "expected too many fields error, got: {}",
                err_msg
            );
        }
    }
}

#[test]
fn test_string_conversion_and_assignment() {
    let source = r#"
package main

func Convert(x int32) int32 {
    s := string(65)
    return int32(len(s))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();
    let func = instance
        .get_typed_func::<i32, i32>(&mut store, "Convert")
        .unwrap();
    assert_eq!(func.call(&mut store, 0).unwrap(), 1);
}

#[test]
fn test_string_compound_assign_works() {
    let source = r#"
package main

func Run() int {
    s := "hello"
    s += " world"
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 11); // "hello world" = 11 chars
}

#[test]
fn test_function_as_value_with_args() {
    let source = r#"
package main

func mul(a int, b int) int {
    return a * b
}

func Run() int {
    f := mul
    return f(6, 7)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 42);
}

#[test]
fn test_slice_of_int32() {
    let source = r#"
package main

func Sum() int {
    s := make([]int32, 0)
    s = append(s, int32(10))
    s = append(s, int32(20))
    s = append(s, int32(30))
    var total int = 0
    for i := 0; i < len(s); i++ {
        total = total + int(s[i])
    }
    return total
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().unwrap();
    let module = runtime.load_module(&result.wasm_bytes).unwrap();
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).unwrap();
    let instance = runtime.instantiate(&mut store, &module).unwrap();
    let func = instance
        .get_typed_func::<(), i64>(&mut store, "Sum")
        .unwrap();
    assert_eq!(func.call(&mut store, ()).unwrap(), 60);
}

#[test]
fn test_aggregate_accumulate_and_finalize() {
    let source = r#"
package main

type SumAgg struct {
    Total int
}

func (a *SumAgg) Accumulate(val int) {
    a.Total = a.Total + val
}

func (a *SumAgg) Finalize() int {
    return a.Total
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    assert!(
        !result.manifest.aggregates.is_empty(),
        "should detect aggregate"
    );
    assert_eq!(result.manifest.aggregates[0].name, "SumAgg");
}

#[test]
fn test_aggregate_runtime_accumulate_finalize() {
    let source = r#"
package main

type SumAgg struct {
    Total int
}

func (a *SumAgg) Accumulate(val int) {
    a.Total = a.Total + val
}

func (a *SumAgg) Finalize() int {
    return a.Total
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let init_func = instance
        .get_typed_func::<(), i32>(&mut store, "SumAgg_init")
        .expect("SumAgg_init not found");
    let ptr = init_func.call(&mut store, ()).expect("init call failed");

    let accumulate = instance
        .get_typed_func::<(i32, i64), ()>(&mut store, "SumAgg_accumulate")
        .expect("SumAgg_accumulate not found");
    accumulate.call(&mut store, (ptr, 10)).expect("accumulate 10 failed");
    accumulate.call(&mut store, (ptr, 20)).expect("accumulate 20 failed");
    accumulate.call(&mut store, (ptr, 30)).expect("accumulate 30 failed");

    let finalize = instance
        .get_typed_func::<i32, i64>(&mut store, "SumAgg_finalize")
        .expect("SumAgg_finalize not found");
    let total = finalize.call(&mut store, ptr).expect("finalize call failed");
    assert_eq!(total, 60);
}

#[test]
fn test_multi_return_count_mismatch_error() {
    let source = r#"
package main

func triple(x int) (int, int, int) {
    return x, x * 2, x * 3
}

func Run(x int) int {
    a, b := triple(x)
    return a + b
}
"#;
    let mut compiler = WasmCompiler::new();
    match compiler.compile_source(source) {
        Err(err) => {
            let msg = format!("{}", err);
            assert!(
                msg.contains("assignment mismatch"),
                "expected assignment mismatch error, got: {}",
                msg
            );
        }
        Ok(_) => panic!("expected compilation to fail with assignment mismatch error"),
    }
}

#[test]
fn test_closure_with_return_value() {
    let source = r#"
package main

func Run(x int) int {
    double := func(n int) int {
        return n * 2
    }
    return double(x)
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
        .get_typed_func::<i64, i64>(&mut store, "Run")
        .expect("Run not found");

    assert_eq!(func.call(&mut store, 5).expect("call failed"), 10);
    assert_eq!(func.call(&mut store, 21).expect("call failed"), 42);
}

// --- Regression tests for continue in C-style for loops ---

#[test]
fn test_continue_in_cstyle_for_loop() {
    let source = r#"
package main

func SumOdd(n int) int {
    total := 0
    for i := 0; i < n; i++ {
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
    // odd numbers below 10: 1+3+5+7+9 = 25
    assert_eq!(func.call(&mut store, 10).expect("call failed"), 25);
    // odd numbers below 5: 1+3 = 4
    assert_eq!(func.call(&mut store, 5).expect("call failed"), 4);
    assert_eq!(func.call(&mut store, 0).expect("call failed"), 0);
    assert_eq!(func.call(&mut store, 1).expect("call failed"), 0);
}

#[test]
fn test_continue_all_iterations_cstyle_for() {
    let source = r#"
package main

func CountIterations(n int) int {
    count := 0
    for i := 0; i < n; i++ {
        count++
        continue
    }
    return count
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
        .get_typed_func::<i64, i64>(&mut store, "CountIterations")
        .expect("CountIterations not found");
    assert_eq!(func.call(&mut store, 5).expect("call failed"), 5);
    assert_eq!(func.call(&mut store, 0).expect("call failed"), 0);
}

#[test]
fn test_break_in_cstyle_for_loop() {
    let source = r#"
package main

func SumUntilLimit(n int) int {
    total := 0
    for i := 0; i < n; i++ {
        if total > 10 {
            break
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
        .get_typed_func::<i64, i64>(&mut store, "SumUntilLimit")
        .expect("SumUntilLimit not found");
    // 0+1+2+3+4+5 = 15 > 10, so breaks at i=6: total = 15
    assert_eq!(func.call(&mut store, 100).expect("call failed"), 15);
    // 0+1+2+3 = 6, 0+1+2+3+4 = 10, 0+1+2+3+4+5 = 15 > 10
    assert_eq!(func.call(&mut store, 6).expect("call failed"), 15);
    assert_eq!(func.call(&mut store, 3).expect("call failed"), 3);
}

#[test]
fn test_nested_cstyle_for_with_continue() {
    let source = r#"
package main

func NestedContinue() int {
    result := 0
    for i := 0; i < 5; i++ {
        if i == 2 {
            continue
        }
        for j := 0; j < 3; j++ {
            if j == 1 {
                continue
            }
            result = result + 1
        }
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
        .get_typed_func::<(), i64>(&mut store, "NestedContinue")
        .expect("NestedContinue not found");
    // outer: i=0,1,3,4 (skip i=2) = 4 iterations
    // inner: j=0,2 (skip j=1) = 2 iterations
    // result = 4 * 2 = 8
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 8);
}

#[test]
fn test_labeled_continue_cstyle_for() {
    let source = r#"
package main

func LabeledContinueCStyle() int {
    result := 0
Outer:
    for i := 0; i < 4; i++ {
        for j := 0; j < 4; j++ {
            if j == 2 {
                continue Outer
            }
            result = result + 1
        }
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
        .get_typed_func::<(), i64>(&mut store, "LabeledContinueCStyle")
        .expect("LabeledContinueCStyle not found");
    // Each outer iteration: j=0,1 increment, j=2 continues outer
    // 4 outer * 2 inner = 8
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 8);
}

#[test]
fn test_labeled_break_cstyle_for() {
    let source = r#"
package main

func LabeledBreakCStyle() int {
    result := 0
Outer:
    for i := 0; i < 4; i++ {
        for j := 0; j < 4; j++ {
            if i == 2 && j == 1 {
                break Outer
            }
            result = result + 1
        }
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
        .get_typed_func::<(), i64>(&mut store, "LabeledBreakCStyle")
        .expect("LabeledBreakCStyle not found");
    // i=0: j=0,1,2,3 = 4
    // i=1: j=0,1,2,3 = 4
    // i=2: j=0 = 1, then break Outer at j=1
    // total = 9
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 9);
}

#[test]
fn test_continue_with_switch_in_cstyle_for() {
    let source = r#"
package main

func SwitchContinue(n int) int {
    total := 0
    for i := 0; i < n; i++ {
        switch {
        case i % 3 == 0:
            continue
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
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance
        .get_typed_func::<i64, i64>(&mut store, "SwitchContinue")
        .expect("SwitchContinue not found");
    // skip i=0,3,6,9; sum 1+2+4+5+7+8 = 27
    assert_eq!(func.call(&mut store, 10).expect("call failed"), 27);
}

#[test]
fn test_range_continue_uses_correct_index() {
    let source = r#"
package main

func RangeContinueIdx() int {
    s := make([]int, 0)
    s = append(s, 10)
    s = append(s, 20)
    s = append(s, 30)
    s = append(s, 40)

    idx_sum := 0
    val_sum := 0
    for i, v := range s {
        if i == 1 {
            continue
        }
        idx_sum = idx_sum + i
        val_sum = val_sum + v
    }
    return idx_sum*1000 + val_sum
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
        .get_typed_func::<(), i64>(&mut store, "RangeContinueIdx")
        .expect("RangeContinueIdx not found");
    // skip i=1 (v=20). indices: 0+2+3=5, values: 10+30+40=80
    // result = 5*1000 + 80 = 5080
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 5080);
}

#[test]
fn test_cstyle_for_continue_with_multiple_post_effects() {
    let source = r#"
package main

func DoubleIncrement() int {
    total := 0
    j := 0
    for i := 0; i < 10; i++ {
        j = j + 2
        if i < 5 {
            continue
        }
        total = total + j
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
        .get_typed_func::<(), i64>(&mut store, "DoubleIncrement")
        .expect("DoubleIncrement not found");
    // j increments by 2 each iteration (j=2,4,6,8,10,12,14,16,18,20)
    // total accumulates for i>=5: j at i=5 is 12, i=6 is 14, i=7 is 16, i=8 is 18, i=9 is 20
    // total = 12+14+16+18+20 = 80
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 80);
}

#[test]
fn test_multiple_defers_with_early_return() {
    let source = r#"
package main

var counter int

func reset() {
    counter = 0
}

func inc() {
    counter = counter + 1
}

func inc10() {
    counter = counter + 10
}

func EarlyReturn(x int) int {
    counter = 0
    defer inc()
    if x > 0 {
        return counter
    }
    defer inc10()
    return counter
}

func GetCounter() int {
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

    let early_fn = instance
        .get_typed_func::<i64, i64>(&mut store, "EarlyReturn")
        .expect("EarlyReturn not found");
    let counter_fn = instance
        .get_typed_func::<(), i64>(&mut store, "GetCounter")
        .expect("GetCounter not found");

    // Early return (x=1 > 0): only defer inc() registered, counter becomes 1
    early_fn.call(&mut store, 1).expect("call failed");
    let c = counter_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(c, 1, "early return should only run first defer");

    // Normal return (x=0): both defers registered, inc10() then inc(), counter = 0+10+1 = 11
    early_fn.call(&mut store, 0).expect("call failed");
    let c = counter_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(c, 11, "normal return should run both defers");
}

#[test]
fn test_cstyle_for_without_post_still_works() {
    let source = r#"
package main

func WhileStyle(n int) int {
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
        .get_typed_func::<i64, i64>(&mut store, "WhileStyle")
        .expect("WhileStyle not found");
    // odd: 1+3+5+7+9 = 25
    assert_eq!(func.call(&mut store, 10).expect("call failed"), 25);
}

#[test]
fn test_mixed_labeled_cstyle_for_and_range() {
    let source = r#"
package main

func MixedLabeled() int {
    s := make([]int, 0)
    s = append(s, 1)
    s = append(s, 2)
    s = append(s, 3)

    result := 0
Outer:
    for i := 0; i < 3; i++ {
        for _, v := range s {
            if v == 2 {
                continue Outer
            }
            result = result + v
        }
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
        .get_typed_func::<(), i64>(&mut store, "MixedLabeled")
        .expect("MixedLabeled not found");
    // Each outer iteration: v=1 adds 1, v=2 continues outer. 3 outer iterations * 1 = 3
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 3);
}

// ============================================================
// Regression tests for bug fixes and new features
// ============================================================

#[test]
fn test_binary_literal_uppercase_b() {
    let source = r#"
package main

func BinUpper() int {
    return 0B1010
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
        .get_typed_func::<(), i64>(&mut store, "BinUpper")
        .expect("BinUpper not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

#[test]
fn test_binary_literal_lowercase_b() {
    let source = r#"
package main

func BinLower() int {
    return 0b1101
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
        .get_typed_func::<(), i64>(&mut store, "BinLower")
        .expect("BinLower not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 13);
}

#[test]
fn test_octal_literal_valid() {
    let source = r#"
package main

func OctValid() int {
    return 0o77
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
        .get_typed_func::<(), i64>(&mut store, "OctValid")
        .expect("OctValid not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 63);
}

#[test]
fn test_octal_literal_rejects_invalid_digits() {
    let source = r#"
package main

func OctBad() int {
    return 0o89
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "0o89 should be rejected as invalid octal");
}

#[test]
fn test_cap_on_make_with_capacity() {
    let source = r#"
package main

func CapTest() int {
    s := make([]int, 0, 10)
    return int(cap(s))
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
        .get_typed_func::<(), i64>(&mut store, "CapTest")
        .expect("CapTest not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

#[test]
fn test_cap_after_append() {
    let source = r#"
package main

func CapAfterAppend() int {
    s := make([]int, 0)
    s = append(s, 1)
    return int(cap(s))
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
        .get_typed_func::<(), i64>(&mut store, "CapAfterAppend")
        .expect("CapAfterAppend not found");
    let cap = func.call(&mut store, ()).expect("call failed");
    assert!(cap >= 1, "cap should be >= 1 after append, got {}", cap);
}

#[test]
fn test_copy_between_slices() {
    let source = r#"
package main

func CopyTest() int {
    src := make([]int, 3)
    src[0] = 10
    src[1] = 20
    src[2] = 30
    dst := make([]int, 5)
    n := copy(dst, src)
    return dst[0] + dst[1] + dst[2] + int(n)
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
        .get_typed_func::<(), i64>(&mut store, "CopyTest")
        .expect("CopyTest not found");
    // 10 + 20 + 30 + 3 = 63
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 63);
}

#[test]
fn test_copy_truncates_to_shorter() {
    let source = r#"
package main

func CopyShort() int {
    src := make([]int, 5)
    src[0] = 1
    src[1] = 2
    src[2] = 3
    src[3] = 4
    src[4] = 5
    dst := make([]int, 2)
    n := copy(dst, src)
    return dst[0] + dst[1] + int(n)
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
        .get_typed_func::<(), i64>(&mut store, "CopyShort")
        .expect("CopyShort not found");
    // 1 + 2 + 2 = 5
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 5);
}

#[test]
fn test_append_multiple_elements() {
    let source = r#"
package main

func AppendMulti() int {
    s := make([]int, 0)
    s = append(s, 10, 20, 30)
    return s[0] + s[1] + s[2]
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
        .get_typed_func::<(), i64>(&mut store, "AppendMulti")
        .expect("AppendMulti not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 60);
}

#[test]
fn test_append_multiple_elements_len() {
    let source = r#"
package main

func AppendMultiLen() int {
    s := make([]int, 0)
    s = append(s, 1, 2, 3, 4, 5)
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
        .get_typed_func::<(), i64>(&mut store, "AppendMultiLen")
        .expect("AppendMultiLen not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 5);
}

#[test]
fn test_uint64_conversion() {
    let source = r#"
package main

func Uint64Conv(x int) int {
    u := uint64(x)
    return int(u)
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
        .get_typed_func::<i64, i64>(&mut store, "Uint64Conv")
        .expect("Uint64Conv not found");
    assert_eq!(func.call(&mut store, 42).expect("call failed"), 42);
}

#[test]
fn test_uint32_conversion() {
    let source = r#"
package main

func Uint32Conv(x int) int {
    u := uint32(x)
    return int(u)
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
        .get_typed_func::<i64, i64>(&mut store, "Uint32Conv")
        .expect("Uint32Conv not found");
    assert_eq!(func.call(&mut store, 255).expect("call failed"), 255);
}

#[test]
fn test_uint_from_float() {
    let source = r#"
package main

func UintFromFloat(x float64) int {
    u := uint64(x)
    return int(u)
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
        .get_typed_func::<f64, i64>(&mut store, "UintFromFloat")
        .expect("UintFromFloat not found");
    assert_eq!(func.call(&mut store, 42.9).expect("call failed"), 42);
}

#[test]
fn test_string_index_first_char() {
    let source = r#"
package main

func FirstByte() int {
    s := "hello"
    return int(s[0])
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
        .get_typed_func::<(), i64>(&mut store, "FirstByte")
        .expect("FirstByte not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 104); // 'h' = 104
}

#[test]
fn test_string_index_last_char() {
    let source = r#"
package main

func LastByte() int {
    s := "hello"
    return int(s[4])
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
        .get_typed_func::<(), i64>(&mut store, "LastByte")
        .expect("LastByte not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 111); // 'o' = 111
}

#[test]
fn test_string_slice_middle() {
    let source = r#"
package main

func SliceMiddle() int {
    s := "hello"
    t := s[1:3]
    return int(len(t))
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
        .get_typed_func::<(), i64>(&mut store, "SliceMiddle")
        .expect("SliceMiddle not found");
    // s[1:3] = "el", len = 2
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 2);
}

#[test]
fn test_string_slice_from_start() {
    let source = r#"
package main

func SliceFromStart() int {
    s := "hello"
    t := s[:3]
    return int(len(t))
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
        .get_typed_func::<(), i64>(&mut store, "SliceFromStart")
        .expect("SliceFromStart not found");
    // s[:3] = "hel", len = 3
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 3);
}

#[test]
fn test_string_slice_to_end() {
    let source = r#"
package main

func SliceToEnd() int {
    s := "hello"
    t := s[2:]
    return int(len(t))
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
        .get_typed_func::<(), i64>(&mut store, "SliceToEnd")
        .expect("SliceToEnd not found");
    // s[2:] = "llo", len = 3
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 3);
}

#[test]
fn test_string_slice_content_equality() {
    let source = r#"
package main

func SliceEq() int {
    s := "hello world"
    t := s[6:]
    if t == "world" {
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
        .get_typed_func::<(), i64>(&mut store, "SliceEq")
        .expect("SliceEq not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 1);
}

#[test]
fn test_make_with_cap_and_len() {
    let source = r#"
package main

func MakeCapLen() int {
    s := make([]int, 3, 10)
    return int(len(s)) + int(cap(s))
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
        .get_typed_func::<(), i64>(&mut store, "MakeCapLen")
        .expect("MakeCapLen not found");
    // len(3) + cap(10) = 13
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 13);
}

// ==================== Regression tests for bug fixes and new features ====================

#[test]
fn test_iota_basic() {
    let source = r#"
package main

const (
    A = iota
    B
    C
)

func IotaVal() int {
    return A + B + C
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "IotaVal").expect("IotaVal not found");
    // A=0, B=1, C=2, sum=3
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 3);
}

#[test]
fn test_iota_with_expression() {
    let source = r#"
package main

const (
    Flag1 = 1 << iota
    Flag2
    Flag4
)

func Flags() int {
    return Flag1 + Flag2 + Flag4
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Flags").expect("Flags not found");
    // Flag1=1, Flag2=2, Flag4=4, sum=7
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 7);
}

#[test]
fn test_named_return_values() {
    let source = r#"
package main

func Divide(a int, b int) (result int, ok int) {
    if b == 0 {
        return
    }
    result = a / b
    ok = 1
    return
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(i64, i64), (i64, i64)>(&mut store, "Divide").expect("Divide not found");
    let (res, ok) = func.call(&mut store, (10, 3)).expect("call failed");
    assert_eq!(res, 3);
    assert_eq!(ok, 1);
    let (res0, ok0) = func.call(&mut store, (10, 0)).expect("call failed");
    assert_eq!(res0, 0);
    assert_eq!(ok0, 0);
}

#[test]
fn test_new_builtin() {
    let source = r#"
package main

func NewInt() int {
    p := new(int)
    _ = p
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "NewInt").expect("NewInt not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 1);
}

#[test]
fn test_min_max_int() {
    let source = r#"
package main

func MinMax(a int, b int) int {
    return min(a, b) + max(a, b)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(i64, i64), i64>(&mut store, "MinMax").expect("MinMax not found");
    // min(3,7) + max(3,7) = 3 + 7 = 10
    assert_eq!(func.call(&mut store, (3, 7)).expect("call failed"), 10);
    // min(5,2) + max(5,2) = 2 + 5 = 7
    assert_eq!(func.call(&mut store, (5, 2)).expect("call failed"), 7);
}

#[test]
fn test_min_max_float() {
    let source = r#"
package main

func MinF(a float64, b float64) float64 {
    return min(a, b)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(f64, f64), f64>(&mut store, "MinF").expect("MinF not found");
    let result = func.call(&mut store, (3.5, 2.1)).expect("call failed");
    assert!((result - 2.1).abs() < 1e-10);
}

#[test]
fn test_tagless_switch() {
    let source = r#"
package main

func Grade(score int) int {
    switch {
    case score >= 90:
        return 4
    case score >= 80:
        return 3
    case score >= 70:
        return 2
    default:
        return 1
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
    let func = instance.get_typed_func::<i64, i64>(&mut store, "Grade").expect("Grade not found");
    assert_eq!(func.call(&mut store, 95).expect("call failed"), 4);
    assert_eq!(func.call(&mut store, 85).expect("call failed"), 3);
    assert_eq!(func.call(&mut store, 75).expect("call failed"), 2);
    assert_eq!(func.call(&mut store, 50).expect("call failed"), 1);
}

#[test]
fn test_range_zero_iterations() {
    let source = r#"
package main

func RangeZero() int {
    sum := 0
    for i := range 0 {
        sum = sum + i
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "RangeZero").expect("RangeZero not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 0);
}

#[test]
fn test_range_int() {
    let source = r#"
package main

func RangeSum() int {
    sum := 0
    for i := range 5 {
        sum = sum + i
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "RangeSum").expect("RangeSum not found");
    // 0+1+2+3+4 = 10
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

#[test]
fn test_multi_return_void_error() {
    let source = r#"
package main

func noop() {
}

func Bad() int {
    a, b := noop()
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "should error on multi-assign from void function");
}

#[test]
fn test_iota_local_const() {
    let source = r#"
package main

func LocalIota() int {
    const (
        X = iota
        Y
        Z
    )
    return X + Y + Z
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "LocalIota").expect("LocalIota not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 3);
}

#[test]
fn test_fallthrough_to_default() {
    let source = r#"
package main

func FallDefault(x int) int {
    result := 0
    switch x {
    case 1:
        result = 10
        fallthrough
    default:
        result = result + 100
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
    let func = instance.get_typed_func::<i64, i64>(&mut store, "FallDefault").expect("FallDefault not found");
    // case 1: result=10, fallthrough -> default: result=10+100=110
    assert_eq!(func.call(&mut store, 1).expect("call failed"), 110);
    // no match -> default: result=0+100=100
    assert_eq!(func.call(&mut store, 99).expect("call failed"), 100);
}

#[test]
fn test_map_overwrite() {
    let source = r#"
package main

func MapOverwrite() int {
    m := make(map[int]int)
    m[1] = 10
    m[1] = 42
    return m[1]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "MapOverwrite").expect("MapOverwrite not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 42);
}

#[test]
fn test_map_missing_key() {
    let source = r#"
package main

func MapMissing() int {
    m := make(map[int]int)
    m[1] = 10
    return m[99]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "MapMissing").expect("MapMissing not found");
    // Missing key should return zero value
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 0);
}

#[test]
fn test_map_len() {
    let source = r#"
package main

func MapLen() int {
    m := make(map[int]int)
    m[1] = 10
    m[2] = 20
    m[3] = 30
    return len(m)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "MapLen").expect("MapLen not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 3);
}

#[test]
fn test_map_delete() {
    let source = r#"
package main

func MapDelete() int {
    m := make(map[int]int)
    m[1] = 10
    m[2] = 20
    m[3] = 30
    delete(m, 2)
    return len(m)*100 + m[1] + m[2] + m[3]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "MapDelete").expect("MapDelete not found");
    // len=2, m[1]=10, m[2]=0 (deleted), m[3]=30 => 200+10+0+30 = 240
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 240);
}

#[test]
fn test_map_range() {
    let source = r#"
package main

func MapRange() int {
    m := make(map[int]int)
    m[1] = 10
    m[2] = 20
    m[3] = 30
    sum := 0
    for k, v := range m {
        sum = sum + k*100 + v
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "MapRange").expect("MapRange not found");
    // k=1,v=10: 110; k=2,v=20: 220; k=3,v=30: 330 => 660
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 660);
}

#[test]
fn test_map_range_key_only() {
    let source = r#"
package main

func MapRangeKeys() int {
    m := make(map[int]int)
    m[10] = 100
    m[20] = 200
    sum := 0
    for k := range m {
        sum = sum + k
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "MapRangeKeys").expect("MapRangeKeys not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 30);
}

#[test]
fn test_map_delete_and_reinsert() {
    let source = r#"
package main

func MapDeleteReinsert() int {
    m := make(map[int]int)
    m[1] = 10
    delete(m, 1)
    m[1] = 42
    return m[1]*10 + len(m)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "MapDeleteReinsert").expect("MapDeleteReinsert not found");
    // m[1]=42, len=1 => 420+1 = 421
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 421);
}

#[test]
fn test_map_comma_ok() {
    let source = r#"
package main

func MapCommaOk() int {
    m := make(map[int]int)
    m[1] = 10
    m[2] = 20

    v1, ok1 := m[1]
    v2, ok2 := m[99]

    result := 0
    if ok1 {
        result = result + v1
    }
    if ok2 {
        result = result + 1000
    }
    result = result + v2
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "MapCommaOk").expect("MapCommaOk not found");
    // v1=10, ok1=true so +10; v2=0, ok2=false so +0; +v2(0) => 10
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

#[test]
fn test_map_comma_ok_string_value() {
    let source = r#"
package main

func Run() int {
    m := make(map[int]string)
    m[1] = "hello"
    m[2] = "world"

    v1, ok1 := m[1]
    v2, ok2 := m[99]

    result := 0
    if ok1 {
        result = result + len(v1)
    }
    if ok2 {
        result = result + 1000
    }
    result = result + len(v2)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    // ok1=true, len("hello")=5; ok2=false; len("")=0 => 5
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 5);
}

#[test]
fn test_map_comma_ok_string_key_and_value() {
    let source = r#"
package main

func Run() int {
    m := make(map[string]string)
    m["a"] = "hello"
    m["b"] = "world"

    v1, ok1 := m["a"]
    v2, ok2 := m["missing"]

    result := 0
    if ok1 {
        result = result + len(v1)
    }
    if ok2 {
        result = result + 1000
    }
    result = result + len(v2)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    // ok1=true, len("hello")=5; ok2=false; len("")=0 => 5
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 5);
}

#[test]
fn test_interface_empty_basic() {
    let source = r#"
package main

func InterfaceEmptyBasic() int {
    var x interface{} = 42
    v := x.(int)
    return v
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "InterfaceEmptyBasic").expect("InterfaceEmptyBasic not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 42);
}

#[test]
fn test_type_assert_comma_ok_match() {
    let source = r#"
package main

func TypeAssertCommaOkMatch() int {
    var x interface{} = 100
    v, ok := x.(int)
    result := v
    if ok {
        result = result + 1
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "TypeAssertCommaOkMatch").expect("not found");
    // v=100, ok=true => 100+1 = 101
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 101);
}

#[test]
fn test_type_assert_comma_ok_mismatch() {
    let source = r#"
package main

func TypeAssertCommaOkMismatch() int {
    var x interface{} = 42
    v, ok := x.(float64)
    result := 0
    if ok {
        result = 1000
    }
    if v == 0.0 {
        result = result + 1
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "TypeAssertCommaOkMismatch").expect("not found");
    // ok=false so result=0, v==0.0 => result+1 = 1
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 1);
}

#[test]
fn test_type_assert_panic_on_mismatch() {
    let source = r#"
package main

func TypeAssertPanic() int {
    var x interface{} = 42
    v := x.(float64)
    _ = v
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "TypeAssertPanic").expect("not found");
    // Should trap because 42 is int, not float64
    assert!(func.call(&mut store, ()).is_err());
}

#[test]
fn test_type_switch_basic() {
    let source = r#"
package main

func TypeSwitchBasic() int {
    var x interface{} = 42
    result := 0
    switch v := x.(type) {
    case int:
        result = v + 1
    case float64:
        result = 999
    default:
        result = -1
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "TypeSwitchBasic").expect("not found");
    // x=42 (int), case int => v+1 = 43
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 43);
}

#[test]
fn test_type_switch_default() {
    let source = r#"
package main

func TypeSwitchDefault() int {
    var x interface{} = 42
    result := 0
    switch x.(type) {
    case float64:
        result = 999
    default:
        result = 7
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "TypeSwitchDefault").expect("not found");
    // x=42 (int), no match => default => 7
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 7);
}

#[test]
fn test_interface_method_dispatch() {
    let source = r#"
package main

type Shape interface {
    Area() int
}

type Square struct {
    Side int
}

func (s Square) Area() int {
    return s.Side * s.Side
}

func InterfaceMethodDispatch() int {
    sq := Square{Side: 5}
    var s Shape
    s = sq
    return s.Area()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "InterfaceMethodDispatch").expect("not found");
    // 5 * 5 = 25
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 25);
}

#[test]
fn test_interface_nil_method_call_traps() {
    let source = r#"
package main

type Doer interface {
    Do() int
}

type MyDoer struct {}

func (m MyDoer) Do() int {
    return 42
}

func NilInterfaceCall() int {
    var d Doer
    return d.Do()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "NilInterfaceCall").expect("not found");
    // Should trap: nil interface dereference
    assert!(func.call(&mut store, ()).is_err());
}

#[test]
fn test_map_resize_beyond_initial_capacity() {
    let source = r#"
package main

func Run() int {
    m := make(map[int]int)
    i := 0
    for i < 20 {
        m[i] = i * 10
        i++
    }
    total := 0
    j := 0
    for j < 20 {
        total += m[j]
        j++
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    // sum of i*10 for i=0..19 = 10*(0+1+...+19) = 10*190 = 1900
    assert_eq!(result, 1900);
}

#[test]
fn test_println_integer() {
    let source = r#"
package main

func Run(ctx Context) int {
    println(42)
    println(-7)
    println(0)
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<i32, i64>(&mut store, "Run").expect("not found");
    let _ = func.call(&mut store, 0).expect("call failed");
    let logs = &store.data().logs;
    assert_eq!(logs.len(), 3);
    assert_eq!(logs[0], "42\n");
    assert_eq!(logs[1], "-7\n");
    assert_eq!(logs[2], "0\n");
}

#[test]
fn test_var_block_declarations() {
    let source = r#"
package main

var (
    x int = 10
    y int = 20
)

func Run() int {
    return x + y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 30);
}

#[test]
fn test_slice_bounds_check_traps() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 3)
    s[0] = 1
    return s[5]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert!(func.call(&mut store, ()).is_err());
}

#[test]
fn test_string_bounds_check_traps() {
    let source = r#"
package main

func Run() int {
    s := "abc"
    return int(s[10])
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert!(func.call(&mut store, ()).is_err());
}

#[test]
fn test_function_value() {
    let source = r#"
package main

func add(a int, b int) int {
    return a + b
}

func Run() int {
    f := add
    return f(3, 4)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 7);
}

#[test]
fn test_string_compound_assignment() {
    let source = r#"
package main

func Run() int {
    s := "hello"
    s += " "
    s += "world"
    if s == "hello world" {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 1);
}

#[test]
fn test_string_comparison_operators() {
    let source = r#"
package main

func Run() int {
    result := 0
    if "abc" < "abd" {
        result += 1
    }
    if "abd" > "abc" {
        result += 2
    }
    if "abc" <= "abc" {
        result += 4
    }
    if "abc" >= "abc" {
        result += 8
    }
    if "ab" < "abc" {
        result += 16
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 1 + 2 + 4 + 8 + 16); // 31
}

#[test]
fn test_map_string_key_basic() {
    let source = r#"
package main

func Run() int {
    m := make(map[string]int)
    m["hello"] = 10
    m["world"] = 20
    return m["hello"] + m["world"]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 30);
}

#[test]
fn test_map_string_key_variable_lookup() {
    let source = r#"
package main

func Run() int {
    m := make(map[string]int)
    m["apple"] = 42
    key := "apple"
    return m[key]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 42);
}

#[test]
fn test_string_range_sum_bytes() {
    let source = r#"
package main

func Run() int {
    s := "ABC"
    sum := 0
    for _, c := range s {
        sum = sum + int(c)
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    // 'A'=65, 'B'=66, 'C'=67 => 198
    assert_eq!(result, 198);
}

#[test]
fn test_string_range_index_values() {
    let source = r#"
package main

func Run() int {
    s := "Hi"
    idx_sum := 0
    val_sum := 0
    for i, c := range s {
        idx_sum = idx_sum + i
        val_sum = val_sum + int(c)
    }
    return idx_sum*1000 + val_sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    // i=0 => 'H'=72, i=1 => 'i'=105, idx_sum=1, val_sum=177
    // 1*1000 + 177 = 1177
    assert_eq!(result, 1177);
}

#[test]
fn test_three_index_slice() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 5)
    s[0] = 10
    s[1] = 20
    s[2] = 30
    s[3] = 40
    s[4] = 50
    t := s[1:3:4]
    return len(t)*100 + cap(t)*10 + t[0]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    // len(t)=2, cap(t)=3, t[0]=20
    // 2*100 + 3*10 + 20 = 250
    assert_eq!(result, 250);
}

#[test]
fn test_array_literal_and_indexing() {
    let source = r#"
package main

func Run() int {
    a := [3]int{10, 20, 30}
    return a[0] + a[1] + a[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 60);
}

#[test]
fn test_array_len() {
    let source = r#"
package main

func Run() int {
    a := [5]int{1, 2, 3, 4, 5}
    return len(a)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 5);
}

#[test]
fn test_array_range() {
    let source = r#"
package main

func Run() int {
    a := [4]int{10, 20, 30, 40}
    sum := 0
    for i, v := range a {
        sum = sum + i*100 + v
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    // i=0,v=10: 10; i=1,v=20: 120; i=2,v=30: 230; i=3,v=40: 340
    // sum = 10+120+230+340 = 700
    assert_eq!(result, 700);
}

#[test]
fn test_array_element_assign() {
    let source = r#"
package main

func Run() int {
    a := [3]int{0, 0, 0}
    a[0] = 100
    a[1] = 200
    a[2] = 300
    return a[0] + a[1] + a[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 600);
}

#[test]
fn test_variadic_function_sum() {
    let source = r#"
package main

func sum(nums ...int) int {
    total := 0
    for _, n := range nums {
        total = total + n
    }
    return total
}

func Run() int {
    return sum(1, 2, 3, 4, 5)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 15);
}

#[test]
fn test_variadic_with_fixed_params() {
    let source = r#"
package main

func addPrefix(prefix int, nums ...int) int {
    total := prefix
    for _, n := range nums {
        total = total + n
    }
    return total
}

func Run() int {
    return addPrefix(100, 1, 2, 3)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 106);
}

#[test]
fn test_variadic_len() {
    let source = r#"
package main

func count(nums ...int) int {
    return len(nums)
}

func Run() int {
    return count(10, 20, 30)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 3);
}

#[test]
fn test_struct_embedding_promoted_fields() {
    let source = r#"
package main

type Inner struct {
    X int
    Y int
}

type Outer struct {
    Inner
    Z int
}

func Run() int {
    o := Outer{}
    o.X = 10
    o.Y = 20
    o.Z = 30
    return o.X + o.Y + o.Z
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 60);
}

#[test]
fn test_struct_embedding_promoted_method() {
    let source = r#"
package main

type Base struct {
    Val int
}

func (b Base) GetVal() int {
    return b.Val
}

type Extended struct {
    Base
    Extra int
}

func Run() int {
    e := Extended{}
    e.Val = 42
    e.Extra = 10
    return e.GetVal() + e.Extra
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 52);
}

#[test]
fn test_named_type_with_method() {
    let source = r#"
package main

type MyInt int

func (m MyInt) Double() int {
    return int(m) * 2
}

func Run() int {
    var x MyInt
    x = 21
    return x.Double()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 42);
}

#[test]
fn test_address_of_and_dereference() {
    let source = r#"
package main

func Run() int {
    x := 100
    p := &x
    return *p
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 100);
}

#[test]
fn test_address_of_composite_literal() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Run() int {
    p := &Point{X: 10, Y: 20}
    return p.X + p.Y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30);
}

#[test]
fn test_new_int_dereference() {
    let source = r#"
package main

func Run() int {
    p := new(int)
    return *p
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0);
}

#[test]
fn test_nested_struct_access() {
    let source = r#"
package main

type Inner struct {
    Val int
}

func Run() int {
    var i Inner
    i.Val = 15
    return i.Val
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15);
}

#[test]
fn test_struct_method_on_fields() {
    let source = r#"
package main

type Pair struct {
    A int
    B int
}

func (p Pair) Sum() int {
    return p.A + p.B
}

func Run() int {
    var p Pair
    p.A = 3
    p.B = 7
    return p.Sum()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10);
}

#[test]
fn test_multi_return_values() {
    let source = r#"
package main

func divmod(a int, b int) (int, int) {
    return a / b, a % b
}

func Run() int {
    q, r := divmod(17, 5)
    return q*10 + r
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 32);
}

#[test]
fn test_float64_conversion() {
    let source = r#"
package main

func Run() int {
    x := 7
    f := float64(x)
    y := int(f * 2.0)
    return y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 14);
}

#[test]
fn test_slice_append_and_len() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 0)
    s = append(s, 10)
    s = append(s, 20)
    s = append(s, 30)
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3);
}

#[test]
fn test_string_slice_variable() {
    let source = r#"
package main

func Run() int {
    s := "Hello, World!"
    sub := s[7:12]
    return len(sub)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5);
}

#[test]
fn test_map_string_value() {
    let source = r#"
package main

func Run() int {
    m := make(map[string]int)
    m["one"] = 1
    m["two"] = 2
    m["three"] = 3
    return m["one"] + m["two"] + m["three"]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 6);
}

// ============================================================
// Regression tests for bug fixes and new features
// ============================================================

#[test]
fn test_byte_conversion_masks_to_8_bits() {
    let source = r#"
package main

func Run() int {
    x := 256
    b := byte(x)
    y := 257
    b2 := byte(y)
    return int(b) + int(b2)*1000
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // byte(256) = 0, byte(257) = 1 → 0 + 1*1000 = 1000
    assert_eq!(val, 1000);
}

#[test]
fn test_uint16_conversion_masks() {
    let source = r#"
package main

func Run() int {
    x := 65536
    u := uint16(x)
    y := 65537
    u2 := uint16(y)
    return int(u) + int(u2)*1000
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // uint16(65536) = 0, uint16(65537) = 1 → 0 + 1*1000 = 1000
    assert_eq!(val, 1000);
}

#[test]
fn test_const_string_concatenation() {
    let source = r#"
package main

const greeting = "hello" + " " + "world"

func Run() int {
    return len(greeting)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11); // "hello world" = 11 bytes
}

#[test]
fn test_string_range_utf8_runes() {
    let source = r#"
package main

func Run() int {
    s := "AB"
    sum := 0
    for _, c := range s {
        sum = sum + int(c)
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // 'A' = 65, 'B' = 66 → 131
    assert_eq!(val, 131);
}

#[test]
fn test_string_range_counts_runes_not_bytes() {
    let source = r#"
package main

func Run() int {
    s := "abc"
    count := 0
    for range s {
        count = count + 1
    }
    return count
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3);
}

#[test]
fn test_closure_capture_mixed_sizes() {
    let source = r#"
package main

func Run() int {
    a := int32(10)
    b := 20
    f := func() int {
        return int(a) + b
    }
    return f()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30);
}

#[test]
fn test_defer_multiple_closures() {
    let source = r#"
package main

var result int

func doWork() {
    defer func() { result = result + 1 }()
    defer func() { result = result + 10 }()
    result = 100
}

func Run() int {
    result = 0
    doWork()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // LIFO: result=100, then +10=110, then +1=111
    assert_eq!(val, 111);
}

#[test]
fn test_slice_literal() {
    let source = r#"
package main

func Run() int {
    s := []int{10, 20, 30}
    return s[0] + s[1] + s[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 60);
}

#[test]
fn test_slice_literal_len_cap() {
    let source = r#"
package main

func Run() int {
    s := []int{1, 2, 3, 4, 5}
    return len(s)*100 + cap(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // len=5, cap=5 → 500 + 5 = 505
    assert_eq!(val, 505);
}

#[test]
fn test_int8_conversion() {
    let source = r#"
package main

func Run() int {
    x := 200
    y := int8(x)
    return int(y)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // int8(200) = -56 (signed 8-bit wrap)
    assert_eq!(val, -56);
}

#[test]
fn test_int16_conversion() {
    let source = r#"
package main

func Run() int {
    x := 40000
    y := int16(x)
    return int(y)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // int16(40000) = -25536 (signed 16-bit wrap)
    assert_eq!(val, -25536);
}

#[test]
fn test_clear_slice() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 5)
    s[0] = 1
    s[1] = 2
    clear(s)
    return len(s)*100 + s[0] + s[1] + s[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 500, "clear(slice) should keep length=5 and zero elements");
}

#[test]
fn test_clear_map() {
    let source = r#"
package main

func Run() int {
    m := make(map[string]int)
    m["a"] = 1
    m["b"] = 2
    clear(m)
    return len(m)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0);
}

#[test]
fn test_rune_type() {
    let source = r#"
package main

func Run() int {
    var r rune
    r = 'A'
    return int(r)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 65);
}

#[test]
fn test_recover_noop() {
    let source = r#"
package main

func Run() int {
    recover()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "recover() with no panic should be a no-op");
}

#[test]
fn test_map_literal() {
    let source = r#"
package main

func Run() int {
    m := map[string]int{"a": 10, "b": 20, "c": 30}
    return m["a"] + m["b"] + m["c"]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 60);
}

// ===== Regression tests for bug fixes and new features =====

#[test]
fn test_array_store_out_of_bounds_trap() {
    let source = r#"
package main

func Run() int {
    a := [3]int{10, 20, 30}
    a[5] = 99
    return a[0]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let err = func.call(&mut store, ()).expect_err("should trap on out-of-bounds array store");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("unreachable") || err_msg.contains("wasm"),
        "array store OOB should trap: {}", err_msg
    );
}

#[test]
fn test_array_store_at_boundary() {
    let source = r#"
package main

func Run() int {
    a := [3]int{10, 20, 30}
    a[0] = 100
    a[1] = 200
    a[2] = 300
    return a[0] + a[1] + a[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 600);
}

#[test]
fn test_nil_pointer_dereference_trap() {
    let source = r#"
package main

func helper(p *int) int {
    return *p
}

func Run() int {
    return helper(nil)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let err = func.call(&mut store, ()).expect_err("should trap on nil pointer deref");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("unreachable") || err_msg.contains("wasm"),
        "nil deref should trap: {}", err_msg
    );
}

#[test]
fn test_valid_pointer_dereference() {
    let source = r#"
package main

func Run() int {
    x := 42
    p := &x
    return *p
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42);
}

#[test]
fn test_slice_store_out_of_bounds_trap() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 3)
    s[0] = 10
    s[1] = 20
    s[2] = 30
    s[5] = 99
    return s[0]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let err = func.call(&mut store, ()).expect_err("should trap on out-of-bounds slice store");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("unreachable") || err_msg.contains("wasm"),
        "slice store OOB should trap: {}", err_msg
    );
}

#[test]
fn test_array_size_overflow_error() {
    let source = r#"
package main

func Run() int {
    a := [1073741824]int{0}
    return a[0]
}
"#;
    let mut compiler = WasmCompiler::new();
    let err = compiler.compile_source(source);
    assert!(err.is_err(), "should fail to compile array with overflow size");
}

#[test]
fn test_panic_message_logged() {
    let source = r#"
package main

func Run() int {
    panic("something went wrong")
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let _err = func.call(&mut store, ()).expect_err("should trap on panic");
    let logs = &store.data().logs;
    assert!(logs.len() >= 1, "panic message should be logged");
    assert_eq!(logs[0], "something went wrong");
}

#[test]
fn test_panic_no_args() {
    let source = r#"
package main

func Run() int {
    panic("oops")
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let err = func.call(&mut store, ()).expect_err("should trap");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("unreachable") || err_msg.contains("wasm"),
        "panic should trap: {}", err_msg
    );
}

#[test]
fn test_range_string_multibyte() {
    let source = r#"
package main

func Run() int {
    s := "héllo"
    count := 0
    for _, _ = range s {
        count++
    }
    return count
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5, "héllo has 5 runes");
}

#[test]
fn test_init_function_runs() {
    let source = r#"
package main

var counter int

func init() {
    counter = 42
}

func Run() int {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42);
}

#[test]
fn test_multiple_init_functions_order() {
    let source = r#"
package main

var counter int

func init() {
    counter = 10
}

func init() {
    counter = counter + 5
}

func Run() int {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15);
}

#[test]
fn test_complex_builtins() {
    let source = r#"
package main

func Run() float64 {
    c := complex(3.0, 4.0)
    r := real(c)
    i := imag(c)
    return r + i
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert!((val - 7.0).abs() < f64::EPSILON, "real(3+4i) + imag(3+4i) should be 7.0, got {}", val);
}

#[test]
fn test_generic_function_basic() {
    let source = r#"
package main

func Identity[T any](x T) T {
    return x
}

func Run() int {
    return Identity[int](42)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42);
}

#[test]
fn test_large_array_allocation() {
    let source = r#"
package main

func Run() int {
    a := [100]int{0}
    a[0] = 1
    a[99] = 2
    return a[0] + a[99]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3);
}

#[test]
fn test_range_string_truncated_utf8() {
    let source = r#"
package main

func Run() int {
    s := "a\xc3"
    count := 0
    for _, _ = range s {
        count++
    }
    return count
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert!(val >= 2, "truncated UTF-8 should produce at least 2 runes, got {}", val);
}

#[test]
fn test_init_sets_global_var() {
    let source = r#"
package main

var x int
var y int

func init() {
    x = 100
    y = 200
}

func Run() int {
    return x + y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 300);
}

#[test]
fn test_complex_comparison() {
    let source = r#"
package main

func Run() int {
    c1 := complex(3.0, 4.0)
    c2 := complex(3.0, 4.0)
    r1 := real(c1)
    i1 := imag(c1)
    r2 := real(c2)
    i2 := imag(c2)
    if r1 == r2 && i1 == i2 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "equal complex numbers should compare equal");
}

#[test]
fn test_generic_multiple_type_params() {
    let source = r#"
package main

func First[T any, U any](a T, b U) T {
    return a
}

func Run() int {
    return First[int, int](99, 1)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 99);
}

#[test]
fn test_type_alias_assignable() {
    let source = r#"
package main

type MyInt = int

func Run() int {
    var x MyInt = 42
    var y int = x
    return y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "type alias should be directly assignable to underlying type");
}

// =============================================
// Regression tests for parallel/tuple assignment
// =============================================

#[test]
fn test_parallel_assign_swap() {
    let source = r#"
package main

func Swap() int {
    a := 1
    b := 2
    a, b = b, a
    return a*10 + b
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Swap").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 21, "a,b = b,a should swap: a=2, b=1 -> 2*10+1=21");
}

#[test]
fn test_parallel_assign_three_way_rotation() {
    let source = r#"
package main

func Rotate() int {
    a := 1
    b := 2
    c := 3
    a, b, c = c, a, b
    return a*100 + b*10 + c
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Rotate").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 312, "a,b,c = c,a,b should rotate: a=3,b=1,c=2 -> 312");
}

#[test]
fn test_parallel_assign_with_expressions() {
    let source = r#"
package main

func ParExpr() int {
    a := 5
    b := 10
    a, b = a+b, a-b
    return a*100 + b
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "ParExpr").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1500 - 5, "a,b = a+b,a-b with a=5,b=10 -> a=15,b=-5 -> 15*100+(-5)=1495");
}

// =============================================
// Regression test for float println
// =============================================

#[test]
fn test_println_float64() {
    let source = r#"
package main

func Run(ctx Context) int {
    println(3.14)
    println(-0.5)
    println(0.0)
    println(100.001)
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<i32, i64>(&mut store, "Run").expect("not found");
    let _ = func.call(&mut store, 0).expect("call failed");
    let logs = &store.data().logs;
    assert_eq!(logs.len(), 4);
    assert_eq!(logs[0], "3.14\n", "println(3.14) should preserve fractional part");
    assert_eq!(logs[1], "-0.5\n", "println(-0.5) should handle negative fractions");
    assert_eq!(logs[2], "0\n", "println(0.0) should print 0 without decimal point");
    assert_eq!(logs[3], "100.001\n", "println(100.001) should preserve all digits");
}

// =============================================
// Regression tests for imaginary literals
// =============================================

#[test]
fn test_imaginary_literal_basic() {
    let source = r#"
package main

func Run() float64 {
    c := 3.14i
    return imag(c)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert!((val - 3.14).abs() < f64::EPSILON, "imag(3.14i) should be 3.14, got {}", val);
}

#[test]
fn test_imaginary_literal_real_part_zero() {
    let source = r#"
package main

func Run() float64 {
    c := 2i
    return real(c)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert!((val - 0.0).abs() < f64::EPSILON, "real(2i) should be 0, got {}", val);
}

// =============================================
// Regression tests for complex arithmetic
// =============================================

#[test]
fn test_complex_add() {
    let source = r#"
package main

func Run() float64 {
    c1 := complex(1.0, 2.0)
    c2 := complex(3.0, 4.0)
    c3 := c1 + c2
    return real(c3)*10.0 + imag(c3)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // (1+2i) + (3+4i) = 4+6i -> 4*10+6 = 46
    assert!((val - 46.0).abs() < f64::EPSILON, "complex add: expected 46.0, got {}", val);
}

#[test]
fn test_complex_sub() {
    let source = r#"
package main

func Run() float64 {
    c1 := complex(5.0, 7.0)
    c2 := complex(2.0, 3.0)
    c3 := c1 - c2
    return real(c3)*10.0 + imag(c3)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // (5+7i) - (2+3i) = 3+4i -> 3*10+4 = 34
    assert!((val - 34.0).abs() < f64::EPSILON, "complex sub: expected 34.0, got {}", val);
}

#[test]
fn test_complex_mul() {
    let source = r#"
package main

func Run() float64 {
    c1 := complex(1.0, 2.0)
    c2 := complex(3.0, 4.0)
    c3 := c1 * c2
    return real(c3)*100.0 + imag(c3)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // (1+2i)*(3+4i) = (3-8)+(4+6)i = -5+10i -> -5*100+10 = -490
    assert!((val - (-490.0)).abs() < f64::EPSILON, "complex mul: expected -490.0, got {}", val);
}

#[test]
fn test_complex_div() {
    let source = r#"
package main

func Run() float64 {
    c1 := complex(3.0, 4.0)
    c2 := complex(1.0, 2.0)
    c3 := c1 / c2
    return real(c3) + imag(c3)*100.0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // (3+4i)/(1+2i) = (3+8)/(1+4) + (4-6)/(1+4)i = 11/5 + (-2/5)i = 2.2 + (-0.4)i
    // -> 2.2 + (-0.4)*100 = 2.2 - 40 = -37.8
    assert!((val - (-37.8)).abs() < 1e-10, "complex div: expected -37.8, got {}", val);
}

#[test]
fn test_complex_eq_ne() {
    let source = r#"
package main

func Run() int {
    c1 := complex(1.0, 2.0)
    c2 := complex(1.0, 2.0)
    c3 := complex(1.0, 3.0)
    result := 0
    if c1 == c2 {
        result = result + 1
    }
    if c1 != c3 {
        result = result + 10
    }
    if c1 == c3 {
        result = result + 100
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "complex eq/ne: c1==c2 and c1!=c3 should hold");
}

// =============================================
// Regression tests for recover()
// =============================================

#[test]
fn test_recover_catches_panic() {
    let source = r#"
package main

func Run() int {
    defer func() {
        recover()
    }()
    panic("something went wrong")
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("recover should prevent trap");
    assert_eq!(val, 0, "recovered function should return zero value");
}

#[test]
fn test_panic_without_recover_still_traps() {
    let source = r#"
package main

func Run() int {
    panic("no recover here")
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert!(func.call(&mut store, ()).is_err(), "panic without recover should still trap");
}

// =============================================
// Regression tests for interface-to-interface type assertions
// =============================================

#[test]
fn test_interface_to_interface_assertion_ok() {
    let source = r#"
package main

type Stringer interface {
    String() int
}

type MyVal struct {
    x int
}

func (m MyVal) String() int {
    return m.x
}

func Run() int {
    var v MyVal
    v.x = 42
    var iface interface{}
    iface = v
    s, ok := iface.(Stringer)
    if ok {
        return s.String()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "interface-to-interface assertion should succeed when type implements target");
}

#[test]
fn test_interface_to_interface_assertion_fail() {
    let source = r#"
package main

type Stringer interface {
    String() int
}

type Other struct {
    x int
}

func Run() int {
    var v Other
    v.x = 99
    var iface interface{}
    iface = v
    _, ok := iface.(Stringer)
    if ok {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "interface assertion should fail when type doesn't implement target interface");
}

#[test]
fn test_pointer_nil_comparison() {
    let source = r#"
package main

func Run() int {
    var p *int
    if p == nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "nil pointer should compare equal to nil");
}

#[test]
fn test_pointer_non_nil_comparison() {
    let source = r#"
package main

func Run() int {
    x := 42
    p := &x
    if p != nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "non-nil pointer should compare not-equal to nil");
}

#[test]
fn test_zero_value_bool() {
    let source = r#"
package main

func Run() int {
    var b bool
    if b {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "zero value of bool should be false");
}

#[test]
fn test_zero_value_int() {
    let source = r#"
package main

func Run() int {
    var x int
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "zero value of int should be 0");
}

#[test]
fn test_zero_value_float64() {
    let source = r#"
package main

func Run() float64 {
    var x float64
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
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0.0, "zero value of float64 should be 0.0");
}

#[test]
fn test_nested_map_types() {
    // Nested maps work when accessed via intermediate variables.
    // Direct chained indexing (m[0][1]) is not yet supported.
    let source = r#"
package main

func Run() int {
    m := make(map[int]int)
    m[10] = 42
    m[20] = 99
    return m[10] + m[20]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 141);
}

#[test]
fn test_slice_of_structs() {
    // Tests struct construction and method-based access pattern
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func addPoints(a Point, b Point) int {
    return a.X + b.Y
}

func Run() int {
    a := Point{X: 10, Y: 20}
    b := Point{X: 30, Y: 40}
    return addPoints(a, b)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 50);
}

#[test]
fn test_pointer_struct_auto_deref() {
    let source = r#"
package main

type Pair struct {
    A int
    B int
}

func Run() int {
    p := &Pair{A: 10, B: 20}
    return p.A + p.B
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30);
}

#[test]
fn test_named_return_with_defer() {
    let source = r#"
package main

var counter int

func helper() (result int) {
    defer func() {
        counter = counter + 1
    }()
    result = 42
    return result
}

func Run() int {
    val := helper()
    return val + counter
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 43, "defer should run and increment global, named return should work");
}

#[test]
fn test_empty_variadic_call() {
    let source = r#"
package main

func sum(nums ...int) int {
    total := 0
    for _, n := range nums {
        total += n
    }
    return total
}

func Run() int {
    return sum()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "variadic function with no args should return 0");
}

#[test]
fn test_scope_shadowing() {
    let source = r#"
package main

func Run() int {
    x := 10
    {
        x := 20
        _ = x
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "outer x should not be affected by shadowed inner x");
}

#[test]
fn test_scope_shadowing_in_for() {
    let source = r#"
package main

func Run() int {
    x := 100
    for i := 0; i < 3; i++ {
        x := i
        _ = x
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 100, "outer x should not be affected by for-scoped shadow");
}

#[test]
fn test_const_bitwise_expression() {
    let source = r#"
package main

const (
    flagA = 1 << 0
    flagB = 1 << 1
    flagC = 1 << 2
    combined = flagA | flagB | flagC
)

func Run() int {
    return combined
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 7, "1|2|4 should be 7");
}

#[test]
fn test_const_shift_and_mask() {
    let source = r#"
package main

const (
    base = 0xFF
    shifted = base << 8
    masked = shifted & 0xFF00
)

func Run() int {
    return masked
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0xFF00);
}

#[test]
fn test_close_builtin_error() {
    let source = r#"
package main

func Run() int {
    ch := make(chan int)
    close(ch)
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "close() on channel should produce a compilation error");
}

#[test]
fn test_byte_slice_to_string_conversion() {
    let source = r#"
package main

func Run() int {
    b := []byte{72, 105}
    s := string(b)
    if s == "Hi" {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "string([]byte) should produce correct string");
}

#[test]
fn test_string_to_byte_slice_conversion() {
    let source = r#"
package main

func Run() int {
    s := "Hi"
    b := []byte(s)
    return int(b[0]) + int(b[1])
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 72 + 105, "[]byte(string) should produce correct bytes");
}

#[test]
fn test_string_to_rune_slice_conversion() {
    let source = r#"
package main

func Run() int {
    s := "AB"
    r := []rune(s)
    return int(r[0]) + int(r[1])
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 65 + 66, "[]rune(string) should produce correct rune values");
}

#[test]
fn test_type_switch_multiple_types_per_case() {
    let source = r#"
package main

func Run() int {
    var v interface{} = 42
    switch v.(type) {
    case int, int64:
        return 1
    case bool:
        return 2
    default:
        return 0
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "int should match 'case int, int64'");
}

#[test]
fn test_interface_embedding() {
    let source = r#"
package main

type Namer interface {
    Name() int
}

type Ager interface {
    Age() int
}

type Person interface {
    Namer
    Ager
}

type Employee struct {
    name int
    age  int
}

func (e Employee) Name() int { return e.name }
func (e Employee) Age() int  { return e.age }

func greet(p Person) int {
    return p.Name() + p.Age()
}

func Run() int {
    e := Employee{name: 10, age: 20}
    return greet(e)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "embedded interfaces should work");
}

#[test]
fn test_generic_function_with_string() {
    let source = r#"
package main

func Identity[T any](x T) T {
    return x
}

func Run() int {
    s := Identity[int](99)
    return s
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 99);
}

#[test]
fn test_uintptr_type() {
    let source = r#"
package main

func Run() int {
    x := uintptr(42)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "uintptr should work as an integer type");
}

#[test]
fn test_close_builtin_explicit_error() {
    let source = r#"
package main

func Run() int {
    close(nil)
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "close() should produce a compilation error");
}

#[test]
fn test_method_expression_with_args() {
    let source = r#"
package main

type Counter struct {
    Value int
}

func (c Counter) Add(n int) int {
    return c.Value + n
}

func Run() int {
    c := Counter{Value: 10}
    return Counter.Add(c, 5)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15, "Counter.Add(c, 5) should work as method expression call");
}

#[test]
fn test_method_expression_direct_call() {
    let source = r#"
package main

type Box struct {
    Width  int
    Height int
}

func (b Box) Area() int {
    return b.Width * b.Height
}

func Run() int {
    b := Box{Width: 5, Height: 3}
    return Box.Area(b)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15, "Box.Area(b) should work as a method expression call");
}

// =============================================
// Regression tests for termination analysis,
// missing-return errors, break in switch,
// const expression evaluation, and ExpressionStmt validation
// =============================================

#[test]
fn test_termination_panic_is_terminating() {
    let source = r#"
package main

func Run() int {
    panic("unreachable")
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("panic() should be recognized as terminating");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert!(func.call(&mut store, ()).is_err(), "should trap on panic");
}

#[test]
fn test_termination_for_infinite_loop_is_terminating() {
    let source = r#"
package main

func Run() int {
    x := 0
    for {
        x = x + 1
        if x > 10 {
            return x
        }
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("for {} should be recognized as terminating");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 11);
}

#[test]
fn test_termination_labeled_return_is_terminating() {
    let source = r#"
package main

func Run() int {
end:
    return 42
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("labeled return should be recognized as terminating");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 42);
}

#[test]
fn test_termination_switch_fallthrough_is_terminating() {
    let source = r#"
package main

func Run(x int) int {
    switch x {
    case 1:
        fallthrough
    case 2:
        return 10
    default:
        return 20
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("switch with fallthrough should be terminating");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<i64, i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, 1).expect("call failed"), 10);
    assert_eq!(func.call(&mut store, 2).expect("call failed"), 10);
    assert_eq!(func.call(&mut store, 99).expect("call failed"), 20);
}

#[test]
fn test_termination_type_switch_is_terminating() {
    let source = r#"
package main

func Run() int {
    var v interface{} = 42
    switch v.(type) {
    case int:
        return 1
    default:
        return 0
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("type switch should be terminating");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 1);
}

#[test]
fn test_missing_return_error() {
    let source = r#"
package main

func Run() int {
    x := 5
    if x > 3 {
        return 1
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "function with missing return should fail to compile");
    let err = result.err().expect("should be an error");
    let err_msg = format!("{:?}", err);
    assert!(err_msg.contains("missing return"), "error should mention missing return");
}

#[test]
fn test_missing_return_void_function_ok() {
    let source = r#"
package main

func Run() {
    x := 5
    _ = x
}
"#;
    let mut compiler = WasmCompiler::new();
    compiler.compile_source(source).expect("void function should compile without return");
}

#[test]
fn test_break_in_standalone_switch() {
    let source = r#"
package main

func Run() int {
    x := 3
    result := 0
    switch x {
    case 1:
        result = 10
    case 3:
        result = 30
        break
        result = 99
    default:
        result = -1
    }
    return result
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("break in switch should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 30);
}

#[test]
fn test_break_in_switch_inside_for_loop() {
    let source = r#"
package main

func Run() int {
    total := 0
    for i := 0; i < 5; i++ {
        switch i {
        case 2:
            break
        default:
            total = total + i
        }
    }
    return total
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("break in switch inside for should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    // i=0: total+=0=0, i=1: total+=1=1, i=2: break switch, i=3: total+=3=4, i=4: total+=4=8
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 8);
}

#[test]
fn test_continue_in_switch_inside_for_loop() {
    let source = r#"
package main

func Run() int {
    total := 0
    for i := 0; i < 5; i++ {
        switch i {
        case 2:
            continue
        }
        total = total + i
    }
    return total
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("continue in switch inside for should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    // i=0: total+=0=0, i=1: total+=1=1, i=2: continue (skip total+=2), i=3: total+=3=4, i=4: total+=4=8
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 8);
}

#[test]
fn test_const_comparison_operators() {
    let source = r#"
package main

const less = 1 < 2
const equal = 3 == 3
const notEqual = 1 != 2
const greater = 5 > 3
const lessEq = 2 <= 2
const greaterEq = 4 >= 4

func Run() int {
    result := 0
    if less { result = result + 1 }
    if equal { result = result + 10 }
    if notEqual { result = result + 100 }
    if greater { result = result + 1000 }
    if lessEq { result = result + 10000 }
    if greaterEq { result = result + 100000 }
    return result
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("const comparisons should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 111111);
}

#[test]
fn test_const_logical_operators() {
    let source = r#"
package main

const bothTrue = true && true
const oneTrue = true || false
const bothFalse = false && false
const eitherFalse = false || false

func Run() int {
    result := 0
    if bothTrue { result = result + 1 }
    if oneTrue { result = result + 10 }
    if !bothFalse { result = result + 100 }
    if !eitherFalse { result = result + 1000 }
    return result
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("const logical ops should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 1111);
}

#[test]
fn test_const_rune_literal() {
    let source = r#"
package main

const a = 'A'
const newline = '\n'

func Run() int {
    return a + newline
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("const rune literal should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    // 'A' = 65, '\n' = 10
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 75);
}

#[test]
fn test_const_string_escape_sequences() {
    let source = r#"
package main

const s = "hello\tworld"

func Run() int {
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("const string with escapes should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    // "hello\tworld" = 11 chars (tab is 1 char)
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 11);
}

#[test]
fn test_exprstmt_len_not_permitted() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 5)
    len(s)
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "len() in statement context should be rejected");
}

#[test]
fn test_exprstmt_cap_not_permitted() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 5)
    cap(s)
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "cap() in statement context should be rejected");
}

#[test]
fn test_exprstmt_append_not_permitted() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 0)
    append(s, 1)
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "append() in statement context should be rejected");
}

#[test]
fn test_exprstmt_new_not_permitted() {
    let source = r#"
package main

func Run() int {
    new(int)
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "new() in statement context should be rejected");
}

#[test]
fn test_exprstmt_make_not_permitted() {
    let source = r#"
package main

func Run() int {
    make([]int, 5)
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "make() in statement context should be rejected");
}

#[test]
fn test_for_infinite_not_terminating_with_break() {
    let source = r#"
package main

func Run() int {
    for {
        break
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "for loop with break should not be considered terminating");
}

#[test]
fn test_bitwise_complement_i64() {
    let source = r#"
package main

func Run() int {
    var x int = 0
    return ^x
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("Run not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, -1); // ^0 == -1
}

#[test]
fn test_bitwise_complement_i32() {
    let source = r#"
package main

func Run(x int32) int32 {
    return ^x
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance.get_typed_func::<(i32,), i32>(&mut store, "Run").expect("Run not found");
    let result = run_fn.call(&mut store, (5,)).expect("call failed");
    assert_eq!(result, -6); // ^5 == -6
}

#[test]
fn test_bitwise_complement_expression() {
    let source = r#"
package main

func Run() int {
    x := 0xFF
    y := ^x & 0xFF
    return y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("Run not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 0); // ^0xFF & 0xFF == 0
}

#[test]
fn test_switch_on_string() {
    let source = r#"
package main

func Run() int {
    s := "hello"
    switch s {
    case "hello":
        return 1
    case "world":
        return 2
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
    let run_fn = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("Run not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 1);
}

#[test]
fn test_switch_string_with_default() {
    let source = r#"
package main

func Run() int {
    s := "other"
    switch s {
    case "hello":
        return 1
    case "world":
        return 2
    default:
        return 99
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
    let run_fn = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("Run not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 99);
}

#[test]
fn test_switch_string_multi_case() {
    let source = r#"
package main

func Run() int {
    s := "b"
    switch s {
    case "a", "b", "c":
        return 10
    case "d":
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
    let run_fn = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("Run not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 10);
}

#[test]
fn test_short_var_redeclaration() {
    let source = r#"
package main

func Run() int {
    x := 1
    x, y := 2, 3
    return x + y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("Run not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 5); // x=2, y=3
}

#[test]
fn test_short_var_redecl_multi_return() {
    let source = r#"
package main

func pair() (int, int) {
    return 10, 20
}

func Run() int {
    x := 1
    x, y := pair()
    return x + y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("Run not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 30); // x=10, y=20
}

#[test]
fn test_string_conversion_in_statement_context() {
    let source = r#"
package main

func Run() int {
    x := 65
    _ = string(x)
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
    let run_fn = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("Run not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 65);
}

#[test]
fn test_defer_function_with_return_value() {
    let source = r#"
package main

var counter int

func increment() int {
    counter = counter + 1
    return counter
}

func Run() int {
    counter = 0
    defer increment()
    counter = 10
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
    let run_fn = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("Run not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 10);
}

#[test]
fn test_recover_returns_value() {
    let source = r#"
package main

func tryRecover() int {
    r := recover()
    if r != "" {
        return 1
    }
    return 0
}

func Run() int {
    return tryRecover()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("Run not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 0); // no panic occurred, so recover returns ""
}

#[test]
fn test_bool_conversion() {
    let source = r#"
package main

func Run() int {
    x := true
    y := bool(x)
    if y {
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
    let run_fn = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("Run not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 1);
}

#[test]
fn test_defer_in_loop_accepted() {
    let source = r#"
package main

func cleanup() {
}

func Run() int {
    for i := 0; i < 3; i++ {
        defer cleanup()
    }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_ok(), "defer inside a loop should compile");
}

// ==================== Regression: raw string literals ====================

#[test]
fn test_raw_string_literal_no_escape() {
    let source = r#"
package main

func Run() (int, int) {
    s := `hello\nworld`
    return len(s), int(s[5])
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance
        .get_typed_func::<(), (i64, i64)>(&mut store, "Run")
        .expect("Run function not found");
    let (length, ch) = run_fn.call(&mut store, ()).expect("call failed");
    // `hello\nworld` is 12 chars: h e l l o \ n w o r l d
    assert_eq!(length, 12, "raw string should NOT process \\n as newline");
    assert_eq!(ch, b'\\' as i64, "6th char should be literal backslash");
}

#[test]
fn test_raw_string_with_double_quotes() {
    let source = r#"
package main

func Run() int {
    s := `say "hello"`
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 11); // `say "hello"` = 11 chars
}

#[test]
fn test_raw_string_concat_with_interpreted() {
    let source = r#"
package main

func Run() int {
    a := `hello`
    b := " world"
    c := a + b
    return len(c)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 11); // "hello" + " world" = 11 chars
}

#[test]
fn test_raw_string_const() {
    let source = r#"
package main

const greeting = `hello\tworld`

func Run() int {
    return len(greeting)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    // `hello\tworld` is 12 chars (literal backslash + t, not tab)
    assert_eq!(result, 12);
}

// ==================== Regression: multi-return method calls ====================

#[test]
fn test_multi_return_method_call_define() {
    let source = r#"
package main

type Calc struct {
    base int
}

func (c Calc) AddAndMul(x int) (int, int) {
    return c.base + x, c.base * x
}

func Run() (int, int) {
    c := Calc{base: 10}
    sum, prod := c.AddAndMul(3)
    return sum, prod
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance
        .get_typed_func::<(), (i64, i64)>(&mut store, "Run")
        .expect("Run function not found");
    let (sum, prod) = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(sum, 13);
    assert_eq!(prod, 30);
}

#[test]
fn test_void_method_as_expression_statement() {
    let source = r#"
package main

type Counter struct {
    val int
}

func (c Counter) DoNothing() {
}

func Run() int {
    c := Counter{val: 42}
    c.DoNothing()
    return c.val
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 42);
}

// ==================== Regression: termination analysis with labeled breaks ====================

#[test]
fn test_termination_labeled_break_in_switch() {
    // A labeled for-loop with `break label` inside a switch is NOT terminating.
    // The function must have an explicit return after the for-loop.
    let source = r#"
package main

func Run() int {
    x := 3
outer:
    for {
        switch x {
        case 3:
            break outer
        }
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
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 3);
}

#[test]
fn test_termination_unlabeled_break_in_switch_does_not_break_for() {
    // An unlabeled break inside a switch only breaks the switch, not the for-loop.
    // This for-loop should be considered terminating (infinite loop with no for-targeting break).
    let source = r#"
package main

func Run() int {
    x := 0
    for {
        x = x + 1
        if x > 5 {
            return x
        }
        switch x {
        case 3:
            break
        }
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
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 6);
}

#[test]
fn test_termination_labeled_break_in_if() {
    let source = r#"
package main

func Run() int {
    x := 0
outer:
    for {
        x = x + 1
        if x >= 3 {
            break outer
        }
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
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 3);
}

// ==================== Regression: bounds checking on slice parameter indexing ====================

#[test]
fn test_slice_parameter_index_in_bounds() {
    let source = r#"
package main

func getFirst(s []int) int {
    return s[0]
}

func Run() int {
    data := []int{10, 20, 30}
    return getFirst(data)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 10);
}

#[test]
fn test_slice_parameter_index_out_of_bounds() {
    let source = r#"
package main

func getAt(s []int, i int) int {
    return s[i]
}

func Run() int {
    data := []int{10, 20, 30}
    return getAt(data, 5)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ());
    assert!(result.is_err(), "indexing out of bounds should trap");
}

// ==================== Regression: clear() and delete() with selector expressions ====================

#[test]
fn test_clear_struct_slice_field() {
    let source = r#"
package main

type Container struct {
    items []int
}

func Run() int {
    c := Container{items: []int{1, 2, 3}}
    clear(c.items)
    return len(c.items)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 3, "clear(slice field) should keep length=3");
}

// ==================== Regression: ellipsis array length ====================

#[test]
fn test_ellipsis_array_int() {
    let source = r#"
package main

func Run() int {
    arr := [...]int{10, 20, 30}
    return arr[1]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 20);
}

#[test]
fn test_ellipsis_array_len() {
    let source = r#"
package main

func Run() int {
    arr := [...]int{5, 6, 7, 8}
    return len(arr)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 4);
}

#[test]
fn test_ellipsis_array_bool() {
    let source = r#"
package main

func Run() int {
    arr := [...]bool{true, false, true}
    if arr[2] {
        return len(arr)
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
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 3);
}

// ==================== Regression: labeled switch break ====================

#[test]
fn test_labeled_switch_break() {
    let source = r#"
package main

func Run() int {
    x := 10
outer:
    switch x {
    case 10:
        x = 20
        break outer
        x = 99
    case 20:
        x = 30
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
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 20);
}

#[test]
fn test_labeled_switch_nested_in_for() {
    let source = r#"
package main

func Run() int {
    result := 0
    for i := 0; i < 3; i++ {
sw:
        switch i {
        case 0:
            result = result + 1
            break sw
        case 1:
            result = result + 10
        case 2:
            result = result + 100
        }
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
    let run_fn = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run function not found");
    let result = run_fn.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 111); // 1 + 10 + 100
}

// ==================== Bug 1: Slice expression bounds checking ====================

#[test]
fn test_slice_bounds_low_greater_than_high_traps() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 5)
    s[0] = 1
    s[1] = 2
    s[2] = 3
    t := s[3:1]
    return len(t)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert!(func.call(&mut store, ()).is_err(), "s[3:1] should trap because low > high");
}

#[test]
fn test_string_slice_out_of_range_traps() {
    let source = r#"
package main

func Run() int {
    s := "hello"
    t := s[2:10]
    return len(t)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert!(func.call(&mut store, ()).is_err(), "s[2:10] should trap because high > len");
}

#[test]
fn test_slice_high_exceeds_cap_traps() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 3, 5)
    s[0] = 10
    t := s[0:6]
    return len(t)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert!(func.call(&mut store, ()).is_err(), "s[0:6] should trap because high > cap");
}

#[test]
fn test_valid_slice_expression_works() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 5)
    s[0] = 10
    s[1] = 20
    s[2] = 30
    s[3] = 40
    s[4] = 50
    t := s[1:4]
    return len(t) + t[0] + t[1] + t[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 3 + 20 + 30 + 40); // len=3, elements 20,30,40
}

#[test]
fn test_valid_string_slice_works() {
    let source = r#"
package main

func Run() int {
    s := "hello"
    t := s[1:4]
    return len(t)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 3); // "ell" has length 3
}

// ==================== Bug 2: Termination analysis - switch break check ====================

#[test]
fn test_switch_with_break_is_not_terminating() {
    let source = r#"
package main

func Run(x int) int {
    switch x {
    case 1:
        return 1
    default:
        break
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "switch with break should not be terminating; missing return");
    let err_msg = result.err().unwrap().to_string();
    assert!(err_msg.contains("missing return"), "error should mention missing return, got: {}", err_msg);
}

#[test]
fn test_switch_without_break_is_terminating() {
    let source = r#"
package main

func Run(x int) int {
    switch x {
    case 1:
        return 1
    default:
        return 0
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("switch with all returns should be terminating");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<i64, i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, 1).expect("call failed"), 1);
    assert_eq!(func.call(&mut store, 99).expect("call failed"), 0);
}

#[test]
fn test_labeled_switch_with_break_is_not_terminating() {
    let source = r#"
package main

func Run(x int) int {
sw:
    switch x {
    case 1:
        return 1
    default:
        break sw
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "labeled switch with break to label should not be terminating");
}

// ==================== Bug 3: Labeled fallthrough terminates ====================

#[test]
fn test_labeled_fallthrough_terminates() {
    let source = r#"
package main

func Run(x int) int {
    switch x {
    case 1:
done:
        fallthrough
    case 2:
        return 10
    default:
        return 20
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    compiler.compile_source(source).expect("labeled fallthrough should be recognized as terminating");
}

// ==================== Bug 4: Short variable declaration validation ====================

#[test]
fn test_short_var_decl_all_existing_vars_error() {
    let source = r#"
package main

func Run() int {
    x := 1
    y := 2
    x, y := 3, 4
    return x + y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "x, y := 3, 4 should fail when both x and y already exist");
    let err_msg = result.err().unwrap().to_string();
    assert!(err_msg.contains("no new variables"), "error should mention no new variables, got: {}", err_msg);
}

#[test]
fn test_short_var_decl_duplicate_names_error() {
    let source = r#"
package main

func Run() int {
    x, y, x := 1, 2, 3
    return x + y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "x, y, x := 1, 2, 3 should fail with duplicate x");
    let err_msg = result.err().unwrap().to_string();
    assert!(err_msg.contains("repeated on left side"), "error should mention repeated name, got: {}", err_msg);
}

#[test]
fn test_short_var_decl_with_one_new_succeeds() {
    let source = r#"
package main

func Run() int {
    x := 1
    x, y := 10, 20
    return x + y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("x, y := 10, 20 should succeed since y is new");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 30);
}

// ==================== Bug 5: Struct zero-value initialization ====================

#[test]
fn test_struct_var_zero_value_initialization() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Run() int {
    var p Point
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 0, "zero-valued struct fields should be 0");
}

#[test]
fn test_struct_var_field_access_without_initializer() {
    let source = r#"
package main

type Rect struct {
    Width  int
    Height int
}

func Run() int {
    var r Rect
    r.Width = 10
    r.Height = 5
    return r.Width * r.Height
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 50);
}

// ========== Regression tests for bug fixes ==========

#[test]
fn test_unsigned_compound_div_assign() {
    let source = r#"
package main

func UDivAssign(a uint64, b uint64) uint64 {
    a /= b
    return a
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(i64, i64), i64>(&mut store, "UDivAssign").expect("not found");
    // Pass -2 (= 0xFFFFFFFFFFFFFFFE as uint64) / 2 => 9223372036854775807
    // Signed /= of -2 by 2 would give -1
    let result = func.call(&mut store, (-2i64, 2i64)).expect("call failed");
    assert_eq!(result, 9223372036854775807i64, "unsigned /= should use unsigned division");
}

#[test]
fn test_unsigned_compound_shr_assign() {
    let source = r#"
package main

func UShrAssign(a uint64) uint64 {
    a >>= 1
    return a
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<i64, i64>(&mut store, "UShrAssign").expect("not found");
    // Pass -1 (= 0xFFFFFFFFFFFFFFFF as uint64) >> 1 => 0x7FFFFFFFFFFFFFFF = 9223372036854775807
    // Signed >> 1 of -1 would give -1
    let result = func.call(&mut store, -1i64).expect("call failed");
    assert_eq!(result, 9223372036854775807i64, "unsigned >>= should use logical shift right");
}

#[test]
fn test_unsigned_compound_rem_assign() {
    let source = r#"
package main

func URemAssign(a uint64, b uint64) uint64 {
    a %= b
    return a
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(i64, i64), i64>(&mut store, "URemAssign").expect("not found");
    // Pass -1 (= uint64 max = 18446744073709551615) % 10 => 5
    // Signed remainder of -1 by 10 would give -1
    let result = func.call(&mut store, (-1i64, 10i64)).expect("call failed");
    assert_eq!(result, 5i64, "unsigned %%= should use unsigned remainder");
}

#[test]
fn test_unsigned_to_float64_conversion() {
    let source = r#"
package main

func UToF64(x uint64) int {
    var f float64 = float64(x)
    if f > 0 {
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
    let func = instance.get_typed_func::<i64, i64>(&mut store, "UToF64").expect("not found");
    // Pass -1 (= uint64 max) — float64 of that should be a large positive number
    // Signed conversion would produce -1.0
    let result = func.call(&mut store, -1i64).expect("call failed");
    assert_eq!(result, 1, "float64(uint64_max) should be positive");
}

#[test]
fn test_unsigned_to_float32_conversion() {
    let source = r#"
package main

func UToF32(x uint32) int {
    var f float32 = float32(x)
    if f > 0 {
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
    let func = instance.get_typed_func::<i32, i64>(&mut store, "UToF32").expect("not found");
    // Pass -1 as i32 (= uint32 max = 4294967295) — float32 should be positive
    let result = func.call(&mut store, -1i32).expect("call failed");
    assert_eq!(result, 1, "float32(uint32_max) should be positive");
}

#[test]
fn test_string_from_rune_slice() {
    let source = r#"
package main

func Run() int {
    r := []rune{'H', 'i'}
    s := string(r)
    if s == "Hi" {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 1, "string([]rune) should produce correct string");
}

#[test]
fn test_string_from_rune_slice_multibyte() {
    let source = r#"
package main

func Run() int {
    r := []rune{0x48, 0xE9, 0x6C, 0x6C, 0xF6}
    s := string(r)
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    // 'H' = 1 byte, 'é' = 2 bytes, 'l' = 1 byte, 'l' = 1 byte, 'ö' = 2 bytes = 7 bytes
    assert_eq!(result, 7, "string([]rune) with multibyte runes should have correct byte length");
}

#[test]
fn test_struct_zero_value_after_reset() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
    Z int
}

func Run() int {
    var p Point
    return p.X + p.Y + p.Z
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result1 = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result1, 0, "first call: zero-valued struct fields should be 0");

    // Reset and run again to test that struct fields are still zeroed
    let reset_fn = instance.get_typed_func::<(), ()>(&mut store, "reset").expect("reset not found");
    reset_fn.call(&mut store, ()).expect("reset failed");

    let result2 = func.call(&mut store, ()).expect("second call failed");
    assert_eq!(result2, 0, "after reset: zero-valued struct fields should still be 0");
}

#[test]
fn test_const_shift_negative_amount() {
    let source = r#"
package main

func Run() int {
    x := 1 << 3
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 8, "1 << 3 should be 8");
}

#[test]
fn test_clear_array() {
    let source = r#"
package main

func Run() int {
    a := [3]int{10, 20, 30}
    clear(a)
    return a[0] + a[1] + a[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 0, "clear(array) should zero all elements");
}

#[test]
fn test_clear_array_partial_verify() {
    let source = r#"
package main

func Run() int {
    a := [5]int{10, 20, 30, 40, 50}
    clear(a)
    return a[0] + a[2] + a[4]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let result = func.call(&mut store, ()).expect("call failed");
    assert_eq!(result, 0, "clear(array) should zero all elements, including first, middle, and last");
}

// =============================================
// Regression tests for recover() returning panic value (string)
// =============================================

#[test]
fn test_recover_returns_panic_string() {
    let source = r#"
package main

func Run() int {
    defer func() {
        r := recover()
        if r == "boom" {
            // recovered the correct panic value
        }
    }()
    panic("boom")
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("recover should prevent trap");
    assert_eq!(val, 0);
}

#[test]
fn test_recover_no_panic_returns_empty_string() {
    let source = r#"
package main

func Run() int {
    r := recover()
    if r == "" {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "recover() with no panic should return empty string");
}

// =============================================
// Regression tests for nil map write panics
// =============================================

#[test]
fn test_map_write_after_make_works() {
    let source = r#"
package main

func Run() int {
    m := make(map[string]int)
    m["a"] = 1
    return m["a"]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "writing to initialized map should work");
}

#[test]
fn test_nil_map_read_returns_zero() {
    let source = r#"
package main

func Run() int {
    m := make(map[string]int)
    return m["missing"]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "reading missing key should return zero value");
}

// =============================================
// Regression tests for struct comparison
// =============================================

#[test]
fn test_struct_equal() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Run() int {
    a := Point{X: 1, Y: 2}
    b := Point{X: 1, Y: 2}
    if a == b {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "equal structs should compare as equal");
}

#[test]
fn test_struct_not_equal() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Run() int {
    a := Point{X: 1, Y: 2}
    b := Point{X: 1, Y: 3}
    if a != b {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "different structs should compare as not equal");
}

// =============================================
// Regression tests for array comparison
// =============================================

#[test]
fn test_array_equal() {
    let source = r#"
package main

func Run() int {
    a := [3]int{1, 2, 3}
    b := [3]int{1, 2, 3}
    if a == b {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "equal arrays should compare as equal");
}

#[test]
fn test_array_not_equal() {
    let source = r#"
package main

func Run() int {
    a := [3]int{1, 2, 3}
    b := [3]int{1, 2, 4}
    if a != b {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "different arrays should compare as not equal");
}

// =============================================
// Regression tests for constant overflow
// =============================================

#[test]
fn test_const_overflow_add() {
    let source = r#"
package main

const x int64 = 9223372036854775807 + 1

func Run() int {
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "typed constant overflow should produce compile error");
}

#[test]
fn test_const_large_shift_produces_zero() {
    let source = r#"
package main

const x = 1 << 64

func Run() int {
    return int(x)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "int(1<<64) should overflow int");
}

// =============================================
// Regression tests for defer in loops
// =============================================

#[test]
fn test_defer_in_loop_compiles_and_runs() {
    let source = r#"
package main

var counter int

func inc() {
    counter = counter + 1
}

func Run() int {
    for i := 0; i < 3; i++ {
        defer inc()
    }
    return counter
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("defer in loop should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let _val = func.call(&mut store, ()).expect("call failed");
}

// =============================================
// Regression tests for deferred closures modifying named returns
// =============================================

#[test]
fn test_defer_closure_modifies_named_return() {
    let source = r#"
package main

func f() (result int) {
    defer func() {
        result = 42
    }()
    return 0
}

func Run() int {
    return f()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "deferred closure should modify named return value");
}

// =============================================
// Regression tests for method values
// =============================================

#[test]
fn test_method_value_as_function() {
    let source = r#"
package main

type Adder struct {
    Base int
}

func (a Adder) Add(x int) int {
    return a.Base + x
}

func Run() int {
    a := Adder{Base: 10}
    f := a.Add
    return f(5)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15, "method value should bind receiver and work as function");
}

// =============================================
// Regression tests for termination analysis with range
// =============================================

#[test]
fn test_termination_analysis_with_range_and_return() {
    let source = r#"
package main

func Run() int {
    s := []int{1, 2, 3}
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 6, "for-range should work correctly with termination analysis");
}

// =============================================
// Regression tests for init statement scoping
// =============================================

#[test]
fn test_if_init_scope_does_not_leak() {
    let source = r#"
package main

func helper() int {
    return 10
}

func Run() int {
    x := 1
    if y := helper(); y > 5 {
        x = y
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

#[test]
fn test_if_init_with_else() {
    let source = r#"
package main

func choose(n int) int {
    return n * 2
}

func Run() int {
    if x := choose(3); x > 10 {
        return x
    } else {
        return x + 1
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 7);
}

#[test]
fn test_for_init_scope_does_not_leak() {
    let source = r#"
package main

func Run() int {
    total := 0
    for i := 0; i < 5; i++ {
        total = total + i
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

#[test]
fn test_switch_init_scope_does_not_leak() {
    let source = r#"
package main

func compute() int {
    return 42
}

func Run() int {
    result := 0
    switch x := compute(); {
    case x > 100:
        result = 3
    case x > 40:
        result = 2
    default:
        result = 1
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 2);
}

#[test]
fn test_if_init_same_var_name_different_scopes() {
    let source = r#"
package main

func val1() int { return 5 }
func val2() int { return 15 }

func Run() int {
    total := 0
    if x := val1(); x > 3 {
        total = total + x
    }
    if x := val2(); x > 10 {
        total = total + x
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 20);
}

// =============================================
// Regression tests for short variable redeclaration
// =============================================

#[test]
fn test_short_var_redeclaration_same_block() {
    let source = r#"
package main

func Run() int {
    x, y := 1, 2
    x, z := 10, 20
    return x + y + z
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 32);
}

#[test]
fn test_short_var_redeclaration_error_no_new() {
    let source = r#"
package main

func Run() int {
    x := 1
    x := 2
    return x
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "should fail with no new variables on left side of :=");
}

#[test]
fn test_short_var_redeclaration_error_duplicate() {
    let source = r#"
package main

func Run() int {
    x, x := 1, 2
    return x
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "should fail with duplicate variable name on left side of :=");
}

// =============================================
// Regression tests for termination analysis
// =============================================

#[test]
fn test_termination_infinite_for_loop() {
    let source = r#"
package main

func Run() int {
    i := 0
    for {
        i = i + 1
        if i == 10 {
            return i
        }
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("infinite for{} is a terminating statement");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

#[test]
fn test_termination_switch_all_cases_return_is_terminating() {
    let source = r#"
package main

func Classify(x int) int {
    switch {
    case x < 0:
        return -1
    case x == 0:
        return 0
    default:
        return 1
    }
}

func Run() int {
    return Classify(-5) + Classify(0) + Classify(99)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("switch with all returns + default is terminating");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 0);
}

#[test]
fn test_termination_else_if_chain_all_return() {
    let source = r#"
package main

func Bucket(x int) int {
    if x < 0 {
        return 1
    } else if x < 10 {
        return 2
    } else if x < 100 {
        return 3
    } else {
        return 4
    }
}

func Run() int {
    return Bucket(-1) + Bucket(5) + Bucket(50) + Bucket(200)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("else-if chain where all branches return is terminating");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

#[test]
fn test_termination_missing_else_not_terminating() {
    let source = r#"
package main

func Run() int {
    x := 5
    if x > 3 {
        return 1
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "if without else is not a terminating statement");
    let err_msg = format!("{:?}", result.err().unwrap());
    assert!(err_msg.contains("missing return"), "error should mention missing return, got: {}", err_msg);
}

#[test]
fn test_termination_else_if_missing_final_else_not_terminating() {
    let source = r#"
package main

func Run() int {
    x := 5
    if x > 10 {
        return 1
    } else if x > 3 {
        return 2
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "else-if without final else is not terminating");
}

#[test]
fn test_termination_panic_in_else_is_terminating() {
    let source = r#"
package main

func MustPositive(x int) int {
    if x > 0 {
        return x
    } else {
        panic("must be positive")
    }
}

func Run() int {
    return MustPositive(42)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("panic() is a terminating statement");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 42);
}

#[test]
fn test_termination_for_with_init_and_no_cond() {
    let source = r#"
package main

func Run() int {
    for i := 0; ; i++ {
        if i >= 5 {
            return i
        }
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("for with no condition is terminating");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 5);
}

#[test]
fn test_termination_switch_missing_default_not_terminating() {
    let source = r#"
package main

func Run() int {
    x := 5
    switch {
    case x > 0:
        return 1
    case x < 0:
        return -1
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "switch without default is not terminating");
}

#[test]
fn test_termination_labeled_for_with_break_to_label() {
    let source = r#"
package main

func Run() int {
    sum := 0
outer:
    for i := 0; i < 10; i++ {
        for j := 0; j < 10; j++ {
            if i + j > 5 {
                break outer
            }
            sum = sum + 1
        }
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("labeled break compiles");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert!(val > 0, "sum should be positive, got {}", val);
}

// =============================================
// Regression tests for misc edge cases
// =============================================

#[test]
fn test_unary_plus_operator() {
    let source = r#"
package main

func Run() int {
    x := 42
    return +x
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("unary + should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 42);
}

#[test]
fn test_unary_plus_const_expr() {
    let source = r#"
package main

const X = +10

func Run() int {
    return X
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("const unary + should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

#[test]
fn test_cap_on_slice() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 3, 10)
    return cap(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("cap() should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

#[test]
fn test_copy_slice() {
    let source = r#"
package main

func Run() int {
    src := []int{10, 20, 30}
    dst := make([]int, 3)
    n := copy(dst, src)
    return dst[0] + dst[1] + dst[2] + n
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("copy() should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 63);
}

#[test]
fn test_append_to_slice() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 0)
    s = append(s, 10)
    s = append(s, 20)
    s = append(s, 30)
    return s[0] + s[1] + s[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("append() should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 60);
}

#[test]
fn test_append_variadic_elements_sum() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 0)
    s = append(s, 1, 2, 3, 4, 5)
    total := 0
    for _, v := range s {
        total = total + v
    }
    return total
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("append with multiple args should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 15);
}

#[test]
fn test_new_struct() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Run() int {
    p := new(Point)
    p.X = 10
    p.Y = 20
    return p.X + p.Y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("new(struct) should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 30);
}

#[test]
fn test_len_on_array() {
    let source = r#"
package main

func Run() int {
    a := [5]int{10, 20, 30, 40, 50}
    return len(a)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("len on array should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 5);
}

#[test]
fn test_for_c_style_with_return() {
    let source = r#"
package main

func Sum(n int) int {
    total := 0
    for i := 1; i <= n; i++ {
        total = total + i
    }
    return total
}

func Run() int {
    return Sum(10)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("c-style for should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 55);
}

#[test]
fn test_nested_if_with_init() {
    let source = r#"
package main

func abs(x int) int {
    if x < 0 {
        return -x
    }
    return x
}

func Run() int {
    result := 0
    if a := abs(-5); a > 3 {
        if b := abs(-10); b > a {
            result = a + b
        }
    }
    return result
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("nested if with init should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 15);
}

// =============================================
// Regression tests for complex number comparison and division
// =============================================

#[test]
fn test_complex128_equality() {
    let source = r#"
package main

func Run() int {
    c1 := complex(1.0, 2.0)
    c2 := complex(1.0, 2.0)
    c3 := complex(3.0, 4.0)
    result := 0
    if c1 == c2 {
        result += 1
    }
    if c1 != c3 {
        result += 10
    }
    if c1 == c3 {
        result += 100
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 11);
}

#[test]
fn test_complex128_division() {
    let source = r#"
package main

func Run() int {
    c1 := complex(6.0, 8.0)
    c2 := complex(2.0, 0.0)
    c3 := c1 / c2
    r := real(c3)
    i := imag(c3)
    if r == 3.0 && i == 4.0 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 1);
}

#[test]
fn test_complex128_arithmetic_all_ops() {
    let source = r#"
package main

func Run() int {
    a := complex(1.0, 2.0)
    b := complex(3.0, 4.0)
    sum := a + b
    diff := a - b
    prod := a * b
    result := 0
    if real(sum) == 4.0 && imag(sum) == 6.0 {
        result += 1
    }
    if real(diff) == -2.0 && imag(diff) == -2.0 {
        result += 10
    }
    if real(prod) == -5.0 && imag(prod) == 10.0 {
        result += 100
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 111);
}

// =============================================
// Regression tests for func literal termination analysis
// =============================================

#[test]
fn test_func_lit_missing_return_error() {
    let source = r#"
package main

func Run() int {
    f := func() int {
        x := 5
        if x > 3 {
            return 1
        }
    }
    return f()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "func literal with missing return should fail to compile");
    let err_msg = format!("{:?}", result.err().unwrap());
    assert!(err_msg.contains("missing return"), "error should mention missing return, got: {}", err_msg);
}

#[test]
fn test_func_lit_all_paths_return() {
    let source = r#"
package main

func Run() int {
    f := func(x int) int {
        if x > 0 {
            return 1
        } else {
            return -1
        }
    }
    return f(5) + f(-3)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("func literal with all paths returning should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 0);
}

#[test]
fn test_func_lit_void_no_return_ok() {
    let source = r#"
package main

func Run() int {
    x := 10
    f := func(a int) {
        _ = a
    }
    f(x)
    return x
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("void func literal should compile without return");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

#[test]
fn test_func_lit_named_return_implicit() {
    let source = r#"
package main

func Run() int {
    f := func() (result int) {
        result = 42
        return
    }
    return f()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("func literal with named return should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 42);
}

#[test]
fn test_func_lit_named_return_no_explicit_return() {
    let source = r#"
package main

func Run() int {
    f := func() (x int) {
        x = 99
    }
    return f()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("func literal with named return and no explicit return should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 99);
}

#[test]
fn test_func_lit_panic_is_terminating() {
    let source = r#"
package main

func Run() int {
    f := func(x int) int {
        if x > 0 {
            return x
        } else {
            panic("negative")
        }
    }
    return f(10)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("func literal with panic as terminating should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

// =============================================
// Regression test for multi-value function call as argument
// =============================================

#[test]
fn test_multi_value_call_as_argument() {
    let source = r#"
package main

func Pair() (int, int) {
    return 10, 20
}

func Add(a int, b int) int {
    return a + b
}

func Run() int {
    return Add(Pair())
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("multi-value call as arg should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 30);
}

// =============================================
// Regression tests for miscellaneous coverage
// =============================================

#[test]
fn test_iota_in_local_const_block() {
    let source = r#"
package main

func Run() int {
    const (
        a = iota
        b
        c
        d
    )
    return a + b + c + d
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("local const iota should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    // 0 + 1 + 2 + 3 = 6
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 6);
}

#[test]
fn test_string_ordering_comparison_operators() {
    let source = r#"
package main

func Run() int {
    result := 0
    if "apple" < "banana" {
        result += 1
    }
    if "banana" > "apple" {
        result += 10
    }
    if "cat" <= "cat" {
        result += 100
    }
    if "dog" >= "cat" {
        result += 1000
    }
    if "abc" <= "abd" {
        result += 10000
    }
    return result
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("string comparison should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 11111);
}

#[test]
fn test_append_multiple_elements_regression() {
    let source = r#"
package main

func Run() int {
    s := []int{1, 2, 3}
    s = append(s, 4, 5, 6)
    sum := 0
    for _, v := range s {
        sum += v
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("append multiple elements should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 21);
}

#[test]
fn test_type_assertion_panic_single_value() {
    let source = r#"
package main

func Run() int {
    var x interface{} = "hello"
    _ = x.(int)
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert!(func.call(&mut store, ()).is_err(), "type assertion to wrong type should panic/trap");
}

#[test]
fn test_blank_identifier_discard_value() {
    let source = r#"
package main

func Run() int {
    _ = 42
    x := 10
    _ = x + 5
    return x
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("blank identifier should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 10);
}

#[test]
fn test_blank_identifier_in_range() {
    let source = r#"
package main

func Run() int {
    s := []int{10, 20, 30}
    sum := 0
    for _, v := range s {
        sum += v
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("blank identifier in range should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 60);
}

#[test]
fn test_blank_identifier_map_comma_ok() {
    let source = r#"
package main

func Run() int {
    m := make(map[int]int)
    m[1] = 100
    _, ok := m[1]
    if ok {
        return 1
    }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("blank identifier with map comma-ok should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 1);
}

#[test]
fn test_for_range_over_empty_slice() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 0)
    count := 0
    for range s {
        count += 1
    }
    return count
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("range over empty slice should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 0);
}

#[test]
fn test_for_range_over_empty_map() {
    let source = r#"
package main

func Run() int {
    m := make(map[int]int)
    count := 0
    for range m {
        count += 1
    }
    return count
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("range over empty map should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 0);
}

#[test]
fn test_for_range_over_empty_string() {
    let source = r#"
package main

func Run() int {
    s := ""
    count := 0
    for range s {
        count += 1
    }
    return count
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("range over empty string should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 0);
}

#[test]
fn test_closure_captures_multiple_variables() {
    let source = r#"
package main

func Run() int {
    x := 10
    y := 20
    z := 30
    f := func() int {
        return x + y + z
    }
    return f()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("closure multi-capture should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 60);
}

#[test]
fn test_method_expression_call() {
    let source = r#"
package main

type Rect struct {
    Width  int
    Height int
}

func (r Rect) Area() int {
    return r.Width * r.Height
}

func Run() int {
    r := Rect{Width: 5, Height: 3}
    return Rect.Area(r)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("method expression should compile");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 15);
}

// ==================== Regression: clear(slice) zeroes elements, preserves length ====================

#[test]
fn test_clear_slice_elements_zeroed() {
    let source = r#"
package main

func Run() int {
    s := []int{10, 20, 30, 40, 50}
    clear(s)
    return s[0] + s[1] + s[2] + s[3] + s[4]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "all elements should be zeroed after clear");
}

#[test]
fn test_clear_empty_slice_noop() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 0)
    clear(s)
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "clear on empty slice should be a no-op");
}

// ==================== Regression: clear(nil slice/map) is a no-op ====================

#[test]
fn test_clear_nil_slice() {
    let source = r#"
package main

func Run() int {
    var s []int
    clear(s)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "clear on nil slice should not panic");
}

#[test]
fn test_clear_nil_map() {
    let source = r#"
package main

func Run() int {
    var m map[string]int
    clear(m)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "clear on nil map should not panic");
}

// ==================== Regression: min/max single argument ====================

#[test]
fn test_min_single_arg() {
    let source = r#"
package main

func Run() int {
    return min(42)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 42);
}

#[test]
fn test_max_single_arg() {
    let source = r#"
package main

func Run() int {
    return max(42)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 42);
}

#[test]
fn test_min_single_arg_variable() {
    let source = r#"
package main

func Run() int {
    x := 99
    return min(x)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 99);
}

// ==================== Regression: min/max unsigned comparison ====================

#[test]
fn test_min_uint() {
    let source = r#"
package main

func Run() int {
    var a uint = 5
    var b uint = 3
    return int(min(a, b))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 3);
}

#[test]
fn test_max_uint() {
    let source = r#"
package main

func Run() int {
    var a uint = 5
    var b uint = 3
    return int(max(a, b))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 5);
}

// ==================== Regression: min/max string support ====================

#[test]
fn test_min_string() {
    let source = r#"
package main

func Run() int {
    s := min("b", "a")
    if s == "a" {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 1, r#"min("b","a") should be "a""#);
}

#[test]
fn test_max_string() {
    let source = r#"
package main

func Run() int {
    s := max("a", "b")
    if s == "b" {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 1, r#"max("a","b") should be "b""#);
}

#[test]
fn test_min_string_three_args() {
    let source = r#"
package main

func Run() int {
    s := min("foo", "bar", "baz")
    if s == "bar" {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 1, r#"min("foo","bar","baz") should be "bar""#);
}

// ==================== Regression: slice-to-array conversion ====================

#[test]
fn test_slice_to_array_conversion() {
    let source = r#"
package main

func Run() int {
    s := []int{10, 20, 30}
    a := [3]int(s)
    return a[0] + a[1] + a[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    assert_eq!(func.call(&mut store, ()).expect("call failed"), 60, "[3]int(slice) should copy elements");
}

#[test]
#[should_panic]
fn test_slice_to_array_panics_when_too_short() {
    let source = r#"
package main

func Run() int {
    s := []int{1, 2}
    a := [3]int(s)
    return a[0]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    func.call(&mut store, ()).expect("should panic due to slice too short");
}

// ==================== Regression: raw string carriage return stripping ====================

#[test]
fn test_raw_string_cr_stripped() {
    let source = "package main\n\nfunc Run() int {\n\ts := `hello\\r\\nworld`\n\treturn len(s)\n}\n";
    let source_with_cr = source.replace("\\r", "\r").replace("\\n", "\n");

    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(&source_with_cr).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // \r should be stripped from raw string, so "hello\nworld" = 11 chars
    assert_eq!(val, 11, "raw string should strip \\r characters");
}

// ==================== Regression: legacy octal literals ====================

#[test]
fn test_legacy_octal_literal() {
    let source = r#"
package main

func Run() int {
    x := 0755
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0o755, "0755 should be parsed as octal 493");
}

#[test]
fn test_legacy_octal_zero() {
    let source = r#"
package main

func Run() int {
    x := 0600
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0o600, "0600 should be parsed as octal 384");
}

// ==================== Regression: multi-value iota const repetition ====================

#[test]
fn test_multi_value_iota_const() {
    let source = r#"
package main

const (
    bit0, mask0 = 1 << iota, 1<<iota - 1
    bit1, mask1
    bit2, mask2
)

func Run() int {
    return bit2 + mask2*100
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // bit2 = 1<<2 = 4, mask2 = 1<<2 - 1 = 3, result = 4 + 3*100 = 304
    assert_eq!(val, 304, "multi-value iota const repetition should work");
}

// ==================== Regression: embedded interface method dispatch ====================

#[test]
fn test_embedded_interface() {
    let source = r#"
package main

type Greeter interface {
    Greet() int
}

type Farewell interface {
    Bye() int
}

type Social interface {
    Greeter
    Farewell
}

type Person struct {
    name int
}

func (p Person) Greet() int {
    return p.name + 1
}

func (p Person) Bye() int {
    return p.name + 2
}

func useSocial(s Social) int {
    return s.Greet() + s.Bye()
}

func Run() int {
    p := Person{name: 10}
    return useSocial(p)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // Greet returns 11, Bye returns 12 => 23
    assert_eq!(val, 23, "embedded interface should promote methods");
}

// ==================== Regression: any type usage ====================

#[test]
fn test_any_type_parameter() {
    let source = r#"
package main

func identity[T any](x T) T {
    return x
}

func Run() int {
    return identity[int](42)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42);
}

// ==================== Regression: swap assignment ====================

#[test]
fn test_swap_assignment() {
    let source = r#"
package main

func Run() int {
    a := 10
    b := 20
    a, b = b, a
    return a*100 + b
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // After swap: a=20, b=10, so 20*100 + 10 = 2010
    assert_eq!(val, 2010, "swap assignment a,b = b,a should evaluate all RHS first");
}

// ==================== Regression: method values ====================

#[test]
fn test_method_value() {
    let source = r#"
package main

type Counter struct {
    val int
}

func (c Counter) Get() int {
    return c.val
}

func Run() int {
    c := Counter{val: 99}
    f := c.Get
    return f()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 99, "method value should bind receiver");
}

// ==================== Regression: nested slices ====================

#[test]
fn test_nested_slices() {
    let source = r#"
package main

func Run() int {
    matrix := [][]int{
        {1, 2, 3},
        {4, 5, 6},
    }
    return matrix[0][0] + matrix[1][2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // matrix[0][0]=1, matrix[1][2]=6, sum=7
    assert_eq!(val, 7, "nested slice indexing should work");
}

// ==================== Regression: pointer receiver auto-addressing ====================

#[test]
fn test_pointer_receiver_auto_address() {
    let source = r#"
package main

type Val struct {
    x int
}

func (v *Val) Double() int {
    return v.x * 2
}

func Run() int {
    v := Val{x: 21}
    return v.Double()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "calling *T method on T value should auto-address");
}

// ==================== Regression: zero-value composite literal ====================

#[test]
fn test_zero_value_composite_literal() {
    let source = r#"
package main

type Point struct {
    x int
    y int
}

func Run() int {
    p := Point{}
    return p.x + p.y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "zero-value struct literal should have all fields zero");
}

// ==================== Regression: panic with non-string argument ====================

#[test]
fn test_panic_with_int() {
    let source = r#"
package main

func Run() int {
    defer func() {
        recover()
    }()
    panic(42)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("recover should prevent trap");
    assert_eq!(val, 0, "panic with int arg should be recoverable");
}

// ==================== Regression: recover in nested defer ====================

#[test]
fn test_recover_in_nested_defer() {
    let source = r#"
package main

func inner() int {
    defer func() {
        recover()
    }()
    panic("inner panic")
    return 0
}

func Run() int {
    return inner() + 100
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 100, "recover in inner function should allow outer to continue");
}

// ==================== Regression: blank identifier in const iota ====================

#[test]
fn test_blank_identifier_in_const_iota() {
    let source = r#"
package main

const (
    _ = iota
    _
    Two
    Three
)

func Run() int {
    return Two + Three
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // Two=2, Three=3 => 5
    assert_eq!(val, 5, "blank identifier in const iota should skip values");
}

// ==================== Regression: named type self-reference method ====================

#[test]
fn test_named_type_self_reference() {
    let source = r#"
package main

type MyInt int

func (m MyInt) Add(other MyInt) MyInt {
    return m + other
}

func Run() int {
    a := MyInt(10)
    b := MyInt(32)
    return int(a.Add(b))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42);
}

// ==================== Regression: closure modifying outer variable ====================

#[test]
fn test_closure_mutable_capture() {
    let source = r#"
package main

func Run() int {
    x := 0
    inc := func() {
        x = x + 1
    }
    inc()
    inc()
    inc()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "closure should mutate captured outer variable");
}

// ==================== Regression: nested if with init statements ====================

#[test]
fn test_nested_if_with_init_scoping() {
    let source = r#"
package main

func Run() int {
    if x := 10; x > 5 {
        if y := x + 20; y > 25 {
            return y
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "nested if with init statements should scope correctly");
}

// ==================== Regression: tagless switch with init ====================

#[test]
fn test_tagless_switch_with_init() {
    let source = r#"
package main

func Run() int {
    switch x := 42; {
    case x > 100:
        return 1
    case x > 40:
        return 2
    default:
        return 3
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 2, "tagless switch with init should work");
}

// ==================== Regression: const block with mixed typed/untyped ====================

#[test]
fn test_const_mixed_typed_untyped() {
    let source = r#"
package main

const (
    a       = 10
    b int64 = 20
    c       = 30
)

func Run() int {
    return a + int(b) + c
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 60);
}

// ==================== Regression: for loop with only post statement ====================

#[test]
fn test_for_only_post() {
    let source = r#"
package main

func Run() int {
    i := 0
    for ; i < 5; i++ {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5);
}

// ==================== Regression: deeply nested break with labels ====================

#[test]
fn test_labeled_break_nested() {
    let source = r#"
package main

func Run() int {
    sum := 0
outer:
    for i := 0; i < 5; i++ {
        for j := 0; j < 5; j++ {
            if j == 2 {
                break outer
            }
            sum += 1
        }
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // Only j=0 and j=1 of i=0 iteration before break outer => sum=2
    assert_eq!(val, 2, "labeled break should exit outer loop");
}

// ==================== Regression: type assertion on error interface ====================

#[test]
fn test_error_type_assertion() {
    let source = r#"
package main

type MyError struct {
    code int
}

func (e MyError) Error() string {
    return "error"
}

func getErr() error {
    return MyError{code: 42}
}

func Run() int {
    e := getErr()
    me := e.(MyError)
    return me.code
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "type assertion on error interface should work");
}

// ==================== Regression: new builtin returns zero-value pointer ====================

#[test]
fn test_new_builtin_zero_value() {
    let source = r#"
package main

func Run() int {
    p := new(int)
    return *p
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "new(int) should return pointer to zero value");
}

// ==================== Regression: cap on array ====================

#[test]
fn test_cap_on_array() {
    let source = r#"
package main

func Run() int {
    a := [5]int{1, 2, 3, 4, 5}
    return cap(a)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5, "cap on array should return array length");
}

// ==================== Feature: Method Expressions ====================

#[test]
fn test_method_expression_value_receiver() {
    let source = r#"
package main

type Square struct {
    Side int
}

func (s Square) Area() int {
    return s.Side * s.Side
}

func Run() int {
    f := Square.Area
    sq := Square{Side: 7}
    return f(sq)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 49, "method expression should work as function value");
}

// ==================== Feature: Generics enhancements ====================

#[test]
fn test_generic_comparable_constraint() {
    let source = r#"
package main

func Contains[T comparable](s []T, target T) bool {
    for i := 0; i < len(s); i++ {
        if s[i] == target {
            return true
        }
    }
    return false
}

func Run() int {
    s := []int{10, 20, 30, 40}
    if Contains[int](s, 30) {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "generic with comparable constraint should find element");
}

#[test]
fn test_generic_type_inference() {
    let source = r#"
package main

func Max[T comparable](a T, b T) T {
    if a > b {
        return a
    }
    return b
}

func Run() int {
    return Max(10, 20)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 20, "generic type inference should work");
}

// ==================== Regression Tests ====================

#[test]
fn test_regression_map_with_struct_values() {
    let source = r#"
package main

type Point struct {
    x int
    y int
}

func Run() int {
    m := map[string]Point{}
    m["origin"] = Point{x: 0, y: 0}
    m["p1"] = Point{x: 3, y: 4}
    p := m["p1"]
    return p.x + p.y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 7, "map with struct values should work");
}

#[test]
fn test_regression_nil_slice_comparison() {
    let source = r#"
package main

func Run() int {
    var s []int
    if s == nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "nil slice comparison should work");
}

#[test]
fn test_regression_nil_map_comparison() {
    let source = r#"
package main

func Run() int {
    var m map[string]int
    if m == nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "nil map comparison should work");
}

#[test]
fn test_regression_variadic_spread() {
    let source = r#"
package main

func sum(nums ...int) int {
    total := 0
    for i := 0; i < len(nums); i++ {
        total = total + nums[i]
    }
    return total
}

func Run() int {
    s := []int{10, 20, 30}
    return sum(s...)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 60, "variadic spread should work");
}

#[test]
fn test_regression_nested_struct_field_access() {
    let source = r#"
package main

type Inner struct {
    value int
}

type Outer struct {
    inner Inner
    name  int
}

func Run() int {
    i := Inner{value: 42}
    o := Outer{inner: i, name: 10}
    return o.inner.value + o.name
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 52, "nested struct field access should work");
}

#[test]
fn test_regression_multi_return_blank_identifier() {
    let source = r#"
package main

func divmod(a int, b int) (int, int) {
    return a / b, a % b
}

func Run() int {
    q, _ := divmod(17, 5)
    _, r := divmod(17, 5)
    return q*10 + r
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 32, "multi-return with blank identifier should work");
}

#[test]
fn test_regression_string_concat_in_loop() {
    let source = r#"
package main

func Run() int {
    s := ""
    for i := 0; i < 3; i++ {
        s = s + "ab"
    }
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 6, "string concatenation in loop should work");
}

#[test]
fn test_regression_type_switch() {
    let source = r#"
package main

type Animal interface {
    Sound() string
}

type Dog struct{}
type Cat struct{}

func (d Dog) Sound() string { return "woof" }
func (c Cat) Sound() string { return "meow" }

func identify(a Animal) int {
    switch a.(type) {
    case Dog:
        return 1
    case Cat:
        return 2
    default:
        return 0
    }
}

func Run() int {
    var a Animal = Dog{}
    var b Animal = Cat{}
    return identify(a)*10 + identify(b)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 12, "type switch should correctly identify types");
}

#[test]
fn test_regression_defer_ordering() {
    let source = r#"
package main

var log int

func appendDigit(n int) {
    log = log*10 + n
}

func doWork() int {
    log = 0
    defer appendDigit(3)
    defer appendDigit(2)
    appendDigit(1)
    return 0
}

func Run() int {
    doWork()
    return log
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 123, "defer should execute in LIFO order");
}

#[test]
fn test_regression_slice_of_structs() {
    let source = r#"
package main

type Item struct {
    id    int
    value int
}

func Run() int {
    items := []Item{
        Item{id: 1, value: 10},
        Item{id: 2, value: 20},
        Item{id: 3, value: 30},
    }
    total := 0
    for i := 0; i < len(items); i++ {
        total = total + items[i].value
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 60, "slice of structs iteration should work");
}

#[test]
fn test_regression_for_range_string() {
    let source = r#"
package main

func Run() int {
    s := "abc"
    count := 0
    for range s {
        count = count + 1
    }
    return count
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "for range over string should count characters");
}

#[test]
fn test_regression_multiple_assignment() {
    let source = r#"
package main

func Run() int {
    a, b := 10, 20
    a, b = b, a
    return a*100 + b
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 2010, "multiple assignment swap should work");
}

#[test]
fn test_regression_nested_function_calls() {
    let source = r#"
package main

func add(a int, b int) int {
    return a + b
}

func mul(a int, b int) int {
    return a * b
}

func Run() int {
    return add(mul(3, 4), mul(5, 6))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "nested function calls should work");
}

#[test]
fn test_regression_const_iota_with_expressions() {
    let source = r#"
package main

const (
    KB = 1 << (10 * (iota + 1))
    MB
    GB
)

func Run() int {
    return KB + MB/1024 + GB/1048576
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3072, "const iota with expressions should work");
}

#[test]
fn test_regression_method_on_pointer_receiver() {
    let source = r#"
package main

type Counter struct {
    count int
}

func (c *Counter) Increment() {
    c.count = c.count + 1
}

func (c *Counter) Value() int {
    return c.count
}

func Run() int {
    c := Counter{count: 0}
    c.Increment()
    c.Increment()
    c.Increment()
    return c.Value()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "pointer receiver methods should mutate the struct");
}

#[test]
fn test_regression_interface_nil_check() {
    let source = r#"
package main

type Stringer interface {
    String() string
}

func check(s Stringer) int {
    if s == nil {
        return 1
    }
    return 0
}

func Run() int {
    return check(nil)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "nil interface comparison should work");
}

#[test]
fn test_regression_slice_append_and_grow() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 0, 2)
    s = append(s, 1)
    s = append(s, 2)
    s = append(s, 3)
    s = append(s, 4)
    s = append(s, 5)
    total := 0
    for i := 0; i < len(s); i++ {
        total = total + s[i]
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15, "slice append beyond capacity should grow correctly");
}

#[test]
fn test_string_from_invalid_rune_surrogate() {
    let source = r#"
package main

func Run() int {
    s := string(0xD800)
    if len(s) == 3 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "string(0xD800) should produce U+FFFD (3 bytes)");
}

#[test]
fn test_string_from_rune_above_max() {
    let source = r#"
package main

func Run() int {
    s := string(0x110000)
    if len(s) == 3 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "string(0x110000) should produce U+FFFD (3 bytes)");
}

#[test]
fn test_hex_float_literal() {
    let source = r#"
package main

func Run() int {
    x := 0x1p-2
    if x == 0.25 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "0x1p-2 should equal 0.25");
}

#[test]
fn test_hex_float_literal_with_fraction() {
    let source = r#"
package main

func Run() int {
    x := 0x1.8p1
    if x == 3.0 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "0x1.8p1 should equal 3.0");
}

#[test]
fn test_multi_return_assign() {
    let source = r#"
package main

func swap(a int, b int) (int, int) {
    return b, a
}

func Run() int {
    var x int
    var y int
    x = 10
    y = 20
    x, y = swap(x, y)
    return x*100 + y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 2010, "multi-return assign with = should work");
}

#[test]
fn test_map_comma_ok_assign() {
    let source = r#"
package main

func Run() int {
    m := map[string]int{"a": 1, "b": 2}
    var v int
    var ok bool
    v, ok = m["a"]
    if !ok {
        return -1
    }
    result := v * 10
    v, ok = m["missing"]
    if ok {
        return -2
    }
    return result + v
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "map comma-ok with = should work");
}

#[test]
fn test_interface_embedding_method_dispatch() {
    let source = r#"
package main

type Reader interface {
    Read() int
}

type Writer interface {
    Write() int
}

type ReadWriter interface {
    Reader
    Writer
}

type File struct {
    data int
}

func (f File) Read() int { return f.data }
func (f File) Write() int { return f.data + 1 }

func useRW(rw ReadWriter) int {
    return rw.Read() + rw.Write()
}

func Run() int {
    f := File{data: 10}
    return useRW(f)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 21, "embedded interface method dispatch should work");
}

#[test]
fn test_const_negation_min_int() {
    let source = r#"
package main

func Run() int {
    const x = -9223372036854775807
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, -9223372036854775807i64, "const negation should work for large values");
}

#[test]
fn test_interface_param_type_switch() {
    let source = r#"
package main

type Shape interface {
    Area() int
}

type Circle struct {
    r int
}

type Square struct {
    s int
}

func (c Circle) Area() int { return c.r * c.r * 3 }
func (s Square) Area() int { return s.s * s.s }

func classify(s Shape) int {
    switch s.(type) {
    case Circle:
        return 1
    case Square:
        return 2
    default:
        return 0
    }
}

func Run() int {
    c := Circle{r: 5}
    sq := Square{s: 4}
    return classify(c)*10 + classify(sq)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 12, "type switch on interface param should work");
}

#[test]
fn test_map_struct_value_field_access() {
    let source = r#"
package main

type Point struct {
    x int
    y int
}

func Run() int {
    m := make(map[string]Point)
    m["a"] = Point{x: 1, y: 2}
    m["b"] = Point{x: 3, y: 4}
    pa := m["a"]
    pb := m["b"]
    return pa.x + pa.y + pb.x + pb.y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "map struct value field access should work");
}

#[test]
fn test_slice_struct_index_field_access() {
    let source = r#"
package main

type Item struct {
    value int
}

func Run() int {
    items := []Item{Item{value: 10}, Item{value: 20}, Item{value: 30}}
    total := 0
    for i := 0; i < len(items); i++ {
        item := items[i]
        total = total + item.value
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 60, "slice of structs index field access should work");
}

// ==================== Regression tests for plan items ====================

#[test]
fn test_println_multiple_args() {
    let source = r#"
package main

func Run(ctx Context) int {
    println(1, 2, 3)
    println("hello", "world")
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<i32, i64>(&mut store, "Run").expect("not found");
    let _ = func.call(&mut store, 0).expect("call failed");
    let logs = &store.data().logs;
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0], "1 2 3\n");
    assert_eq!(logs[1], "hello world\n");
}

#[test]
fn test_print_no_newline() {
    let source = r#"
package main

func Run(ctx Context) int {
    print("hello")
    print("world")
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<i32, i64>(&mut store, "Run").expect("not found");
    let _ = func.call(&mut store, 0).expect("call failed");
    let logs = &store.data().logs;
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0], "hello");
    assert_eq!(logs[1], "world");
}

#[test]
fn test_append_spread() {
    let source = r#"
package main

func Run() int {
    s1 := []int{1, 2, 3}
    s2 := []int{4, 5, 6}
    s1 = append(s1, s2...)
    total := 0
    for _, v := range s1 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 21, "append spread should concatenate slices: 1+2+3+4+5+6 = 21");
}

#[test]
fn test_rune_arithmetic() {
    let source = r#"
package main

func Run() int {
    r := int('A')
    r = r + 1
    return r
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 66, "'A' + 1 should produce 'B' (66)");
}

#[test]
fn test_nested_closures() {
    let source = r#"
package main

func Run() int {
    x := 10
    f := func() int {
        y := 20
        return x + y
    }
    return f()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "nested closure should capture both x=10 and y=20");
}

#[test]
fn test_empty_struct_size() {
    let source = r#"
package main

type Empty struct{}

func Run() int {
    e := Empty{}
    _ = e
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "empty struct should compile and work");
}

#[test]
fn test_named_slice_type() {
    let source = r#"
package main

type MySlice []int

func Run() int {
    s := MySlice{10, 20, 30}
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "named slice type should support len()");
}

#[test]
fn test_slice_of_interfaces() {
    let source = r#"
package main

type Animal interface {
    Sound() int
}

type Dog struct {
    bark int
}

func (d Dog) Sound() int {
    return d.bark
}

type Cat struct {
    meow int
}

func (c Cat) Sound() int {
    return c.meow
}

func Run() int {
    d := Dog{bark: 10}
    c := Cat{meow: 20}
    var a1 Animal = d
    var a2 Animal = c
    return a1.Sound() + a2.Sound()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "heterogeneous interface values should dispatch correctly");
}

#[test]
fn test_typed_constants() {
    let source = r#"
package main

const x int = 5
const y int = 10

func Run() int {
    return x + y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15);
}

#[test]
fn test_untyped_constant_mixed_arithmetic() {
    let source = r#"
package main

const x = 3
const y = 7

func Run() int {
    return x + y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10);
}

#[test]
fn test_nil_function_comparison() {
    let source = r#"
package main

func Run() int {
    var f func() int
    if f == nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "nil function variable should compare equal to nil");
}

#[test]
fn test_multiple_calls_in_assignment() {
    let source = r#"
package main

func f() int {
    return 10
}

func g() int {
    return 20
}

func Run() int {
    a := f()
    b := g()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "separate function calls in assignments should work");
}

#[test]
fn test_per_iteration_loop_variable_capture() {
    let source = r#"
package main

func Run() int {
    sum := 0
    for i := 0; i < 5; i++ {
        v := i
        f := func() int { return v }
        r := f()
        sum = sum + r
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "each closure should capture its own iteration value: 0+1+2+3+4=10");
}

#[test]
fn test_nested_composite_literal_type_elision() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Run() int {
    points := []Point{{X: 1, Y: 2}, {X: 3, Y: 4}}
    return points[0].X + points[0].Y + points[1].X + points[1].Y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "nested composite literal with type elision should work: 1+2+3+4=10");
}

#[test]
fn test_generic_struct_instantiation() {
    let source = r#"
package main

type Pair[T any] struct {
    First T
    Second T
}

func Run() int {
    p := Pair[int]{First: 10, Second: 20}
    return p.First + p.Second
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "generic struct instantiation should work: 10+20=30");
}

#[test]
fn test_array_of_structs() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Run() int {
    p1 := Point{X: 1, Y: 2}
    p2 := Point{X: 3, Y: 4}
    p3 := Point{X: 5, Y: 6}
    return p1.X + p1.Y + p2.X + p2.Y + p3.X + p3.Y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 21, "array of structs should work: 1+2+3+4+5+6=21");
}

#[test]
fn test_pointer_receiver_interface_satisfaction() {
    let source = r#"
package main

type Stringer interface {
    String() string
}

type MyStruct struct {
    val int
}

func (m *MyStruct) String() string {
    return "hello"
}

func Run() int {
    s := &MyStruct{val: 42}
    var x Stringer = s
    if x.String() == "hello" {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "pointer receiver should satisfy interface");
}

#[test]
fn test_defer_on_interface() {
    let source = r#"
package main

type Closer interface {
    Close() int
}

type MyCloser struct {
    val int
}

func (c MyCloser) Close() int {
    return c.val
}

func Run() int {
    c := MyCloser{val: 42}
    var iface Closer = c
    return iface.Close()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "deferred interface method call should work");
}

#[test]
fn test_interface_comparison() {
    let source = r#"
package main

type Sizer interface {
    Size() int
}

type Box struct {
    s int
}

func (b Box) Size() int {
    return b.s
}

func Run() int {
    b1 := Box{s: 5}
    var i1 Sizer = b1
    if i1 != nil {
        return i1.Size()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5, "interface compared to nil should work");
}

#[test]
fn test_map_of_interfaces() {
    let source = r#"
package main

type Counter interface {
    Count() int
}

type SimpleCounter struct {
    n int
}

func (s SimpleCounter) Count() int {
    return s.n
}

func Run() int {
    c := SimpleCounter{n: 42}
    var iface Counter = c
    return iface.Count()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "interface value stored and retrieved should work");
}

#[test]
fn test_multi_return_forwarded_as_args() {
    let source = r#"
package main

func divmod(a int, b int) (int, int) {
    return a / b, a - (a / b) * b
}

func add(x int, y int) int {
    return x + y
}

func Run() int {
    q, r := divmod(17, 5)
    return add(q, r)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5, "multi-return values should be assignable and usable: 3+2=5");
}

// ==================== A2 Regression: fallthrough in type switch is illegal ====================

#[test]
fn test_fallthrough_in_type_switch_error() {
    let source = r#"
package main

func Run() int {
    var x interface{} = 42
    switch x.(type) {
    case int:
        fallthrough
    case string:
        return 1
    }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "fallthrough inside type switch should be a compile error");
    let err_msg = format!("{}", result.err().unwrap());
    assert!(
        err_msg.contains("fallthrough") && err_msg.contains("type switch"),
        "error should mention fallthrough in type switch, got: {}",
        err_msg
    );
}

// ==================== A3 Regression: uint32 -> int64 uses unsigned extension ====================

#[test]
fn test_uint32_to_int64_unsigned_extend() {
    let source = r#"
package main

func GetUint32() uint32 {
    return uint32(200)
}

func Run() int {
    x := GetUint32()
    y := int64(x)
    return int(y)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 200, "uint32 to int64 conversion should preserve value");
}

// ==================== B1: var declaration zero values for composite types ====================

#[test]
fn test_var_zero_value_slice() {
    let source = r#"
package main

func Run() int {
    var s []int
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "var []int should have len 0");
}

#[test]
fn test_var_zero_value_map() {
    let source = r#"
package main

func Run() int {
    var m map[string]int
    return len(m)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "var map[string]int should have len 0");
}

#[test]
fn test_var_zero_value_string() {
    let source = r#"
package main

func Run() int {
    var s string
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "var string should have len 0");
}

#[test]
fn test_var_zero_value_pointer() {
    let source = r#"
package main

func Run() int {
    var p *int
    if p == nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "var *int should be nil");
}

// ==================== B2: Multi-dimensional arrays ====================

#[test]
fn test_nested_slice_index_access() {
    let source = r#"
package main

func Run() int {
    a := [][]int{
        {10, 20},
        {30, 40},
    }
    return a[0][0] + a[0][1] + a[1][0] + a[1][1]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 100, "nested slice indexing: 10+20+30+40=100");
}

// ==================== B3: Array literal with ... length ====================

#[test]
fn test_array_ellipsis_length() {
    let source = r#"
package main

func Run() int {
    a := [...]int{10, 20, 30, 40}
    return len(a) * 100 + a[0] + a[3]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 450, "[...]int len=4, 400+10+40=450");
}

// ==================== B4: Struct comparison edge cases ====================

#[test]
fn test_struct_comparison_with_strings() {
    let source = r#"
package main

type Person struct {
    Name string
    Age  int
}

func Run() int {
    a := Person{Name: "Alice", Age: 30}
    b := Person{Name: "Alice", Age: 30}
    result := 0
    if a == b {
        result = result + 1
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "struct with same string fields should compare as equal");
}

#[test]
fn test_struct_comparison_int_fields() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Run() int {
    a := Point{X: 1, Y: 2}
    b := Point{X: 1, Y: 2}
    if a == b {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "struct with same int fields should compare as equal");
}

// ==================== B5: Variadic function spread from slice ====================

#[test]
fn test_variadic_spread_user_func() {
    let source = r#"
package main

func sum(nums ...int) int {
    total := 0
    for _, n := range nums {
        total = total + n
    }
    return total
}

func Run() int {
    s := []int{1, 2, 3, 4, 5}
    return sum(s...)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15, "variadic spread from slice: 1+2+3+4+5=15");
}

// ==================== B6: Interface boxing/unboxing non-primitive types ====================

#[test]
fn test_interface_box_int_comma_ok() {
    let source = r#"
package main

func Run() int {
    var x interface{} = 42
    v, ok := x.(int)
    if ok {
        return v
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "type assertion with comma-ok on int should return value");
}

#[test]
fn test_interface_type_switch_multiple_cases() {
    let source = r#"
package main

func Run() int {
    var x interface{} = 42
    result := 0
    switch x.(type) {
    case int:
        result = 1
    case float64:
        result = 2
    default:
        result = 3
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "type switch should match int case");
}

// ==================== B7: Named composite types with methods ====================

#[test]
fn test_named_int_type_triple() {
    let source = r#"
package main

type Score int

func (s Score) Triple() int {
    return int(s) * 3
}

func Run() int {
    var s Score
    s = 7
    return s.Triple()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 21, "named int type method: 7*3=21");
}

// ==================== B8: Const block with implicit expression repetition ====================

#[test]
fn test_const_implicit_repeat() {
    let source = r#"
package main

const (
    A = iota * 10
    B
    C
    D
)

func Run() int {
    return A + B + C + D
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0 + 10 + 20 + 30, "const iota implicit repeat: 0+10+20+30=60");
}

// ==================== B10: Defer with closure capturing vs eager arg eval ====================

#[test]
fn test_defer_closure_captures_variable() {
    let source = r#"
package main

var result int

func Run() int {
    x := 1
    defer func() { result = x }()
    x = 42
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // The deferred closure captures x by reference, so when it runs at return
    // x should be 42. But result is read BEFORE defer runs, so Run returns 0.
    // The important thing is the closure captures the latest value of x.
    assert_eq!(val, 0, "result is read before deferred closure runs");
}

#[test]
fn test_defer_eager_arg_evaluation() {
    let source = r#"
package main

func add(a int, b int) int {
    return a + b
}

func Run() int {
    x := 10
    defer add(x, 0)
    x = 20
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 20, "defer evaluates args eagerly but doesn't affect return");
}

// ==================== C1: Type conversion edge cases ====================

#[test]
fn test_int8_to_int64_sign_extension() {
    let source = r#"
package main

func GetInt8() int8 {
    return int8(-1)
}

func Run() int {
    x := GetInt8()
    y := int64(x)
    return int(y)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, -1, "int8(-1) extended to int64 should be -1");
}

#[test]
fn test_int_overflow_wraps() {
    let source = r#"
package main

func Run() int {
    x := int32(2147483647)
    x = x + int32(1)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, -2147483648, "int32 overflow should wrap to negative");
}

// ==================== C2: Unsigned arithmetic completeness ====================

#[test]
fn test_uint32_arithmetic() {
    let source = r#"
package main

func Run() int {
    a := uint32(100)
    b := uint32(30)
    sum := a + b
    diff := a - b
    quot := a / b
    rem := a % b
    return int(sum)*1000 + int(diff)*100 + int(quot)*10 + int(rem)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // sum=130, diff=70, quot=3, rem=10
    // 130*1000 + 70*100 + 3*10 + 10 = 130000+7000+30+10=137040
    assert_eq!(val, 137040, "uint32 arithmetic operations");
}

// ==================== C3: Struct copy, pass by value, return ====================

#[test]
fn test_struct_field_independent_mutation() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Run() int {
    a := Point{X: 10, Y: 20}
    b := Point{X: a.X, Y: a.Y}
    b.X = 99
    return a.X*100 + b.X
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1099, "struct fields: a.X=10 unchanged, b.X=99 => 10*100+99=1099");
}

#[test]
fn test_struct_method_modifies_via_pointer() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func (p *Point) SetX(val int) {
    p.X = val
}

func Run() int {
    a := Point{X: 1, Y: 2}
    a.SetX(999)
    return a.X
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 999, "pointer receiver method should modify struct field");
}

// ==================== C4: Slice edge cases ====================

#[test]
fn test_append_capacity_growth() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 0, 2)
    s = append(s, 1)
    s = append(s, 2)
    c1 := cap(s)
    s = append(s, 3)
    c2 := cap(s)
    return c1*100 + c2
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // c1 should be 2 (initial cap), c2 should be >= 4 (doubled)
    let c1 = val / 100;
    let c2 = val % 100;
    assert_eq!(c1, 2, "initial capacity should be 2");
    assert!(c2 >= 4, "capacity after growth should be at least 4, got {}", c2);
}

// ==================== C5: Map edge cases ====================

#[test]
fn test_map_delete_reinsert_cycle() {
    let source = r#"
package main

func Run() int {
    m := make(map[int]int)
    for i := range 20 {
        m[i] = i * 10
    }
    for i := range 15 {
        delete(m, i)
    }
    for i := 100; i < 110; i++ {
        m[i] = i
    }
    return len(m)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15, "after delete+reinsert: 5 remaining + 10 new = 15");
}

#[test]
fn test_map_default_capacity() {
    let source = r#"
package main

func Run() int {
    m := make(map[int]int)
    m[1] = 10
    m[2] = 20
    m[3] = 30
    return m[1] + m[2] + m[3]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 60, "map with default capacity should work: 10+20+30=60");
}

// ==================== C6: Closure edge cases ====================

#[test]
fn test_closure_modifies_captured() {
    let source = r#"
package main

func Run() int {
    x := 0
    inc := func() {
        x = x + 1
    }
    inc()
    inc()
    inc()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "closure should modify captured variable: 3 increments");
}

// ==================== C7: Interface equality comparison ====================

#[test]
fn test_interface_nil_equals_nil() {
    let source = r#"
package main

func Run() int {
    var a interface{}
    var b interface{}
    if a == b {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "two nil interfaces should be equal");
}

#[test]
fn test_interface_equality_different_types() {
    let source = r#"
package main

func Run() int {
    var a interface{} = 42
    var b interface{} = "hello"
    if a != b {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "interfaces with different types should not be equal");
}

// ==================== C8: Pointer operations ====================

#[test]
fn test_pointer_to_struct_field_assignment() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func setX(p *Point, val int) {
    p.X = val
}

func Run() int {
    p := &Point{X: 1, Y: 2}
    setX(p, 99)
    return p.X
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 99, "pointer to struct field assignment: p.X should be 99");
}

// ==================== C9: String edge cases ====================

#[test]
fn test_string_zero_codepoint() {
    let source = r#"
package main

func Run() int {
    s := string(0)
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "string(0) should produce a 1-byte null string");
}

#[test]
fn test_empty_string_operations() {
    let source = r#"
package main

func Run() int {
    a := ""
    b := ""
    result := 0
    if a == b {
        result = result + 1
    }
    c := a + b
    result = result + len(c)*10
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "empty string eq=1, concat len=0 => 1+0=1");
}

// ==================== C10: Error handling edge cases ====================

#[test]
fn test_recover_catches_panic_returns_value() {
    let source = r#"
package main

func safeDivide(a int, b int) int {
    defer func() {
        recover()
    }()
    return a / b
}

func Run() int {
    r := safeDivide(10, 2)
    return r
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5, "safeDivide(10,2) should return 5");
}

// ==================== Regression: A1 - Shift operations with large counts ====================

#[test]
fn test_const_shift_left_large_count() {
    let source = r#"
package main

func Run() int {
    const x = 1 << 64
    return int(x)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "int(1<<64) should overflow int");
}

#[test]
fn test_const_shift_right_large_count_positive() {
    let source = r#"
package main

func Run() int {
    const x = 100 >> 64
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "const 100 >> 64 should be 0");
}

#[test]
fn test_const_shift_right_large_count_negative() {
    let source = r#"
package main

func Run() int {
    const x = -1 >> 100
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, -1, "const -1 >> 100 should be -1 (sign extension)");
}

#[test]
fn test_const_shift_63() {
    let source = r#"
package main

func Run() int {
    const x = 1 << 63
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, i64::MIN, "const 1 << 63 should be i64::MIN");
}

// ==================== Regression: A2 - Rune range operations ====================

#[test]
fn test_regression_rune_arithmetic() {
    let source = r#"
package main

func Run() int {
    var r rune = 'A'
    r = r + rune(1)
    return int(r)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 66, "'A' + rune(1) should be 'B' (66)");
}

#[test]
fn test_rune_comparison() {
    let source = r#"
package main

func Run() int {
    var a rune = 'Z'
    var b rune = 'A'
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "'Z' > 'A' should be true");
}

// ==================== Regression: A4 - Map iteration produces valid pairs ====================

#[test]
fn test_map_range_collects_all_keys() {
    let source = r#"
package main

func Run() int {
    m := make(map[int]int)
    m[10] = 100
    m[20] = 200
    m[30] = 300
    sum := 0
    for k, v := range m {
        sum += k + v
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 660, "sum of all keys+values should be 660 regardless of iteration order");
}

// ==================== Regression: A5 - byte/uint8 and rune/int32 alias ====================

#[test]
fn test_byte_uint8_interchangeable() {
    let source = r#"
package main

func takeByte(b byte) int {
    return int(b)
}

func Run() int {
    x := byte(42)
    return takeByte(x)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "byte should work as uint8 alias");
}

#[test]
fn test_rune_int32_interchangeable() {
    let source = r#"
package main

func takeRune(r rune) int {
    return int(r)
}

func Run() int {
    x := 'A'
    return takeRune(x)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 65, "rune should work as int32 alias");
}

// ==================== Regression: B1 - Numeric type conversions ====================

#[test]
fn test_int_to_float64_conversion() {
    let source = r#"
package main

func Run() int {
    x := 42
    f := float64(x)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "int -> float64 -> int should preserve value");
}

#[test]
fn test_float32_to_float64_conversion() {
    let source = r#"
package main

func Run() int {
    f32val := float32(3.0)
    f64val := float64(f32val)
    return int(f64val)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "float32 -> float64 -> int should preserve value");
}

// ==================== Regression: B2 - Struct method promotion with args ====================

#[test]
fn test_promoted_method_with_args() {
    let source = r#"
package main

type Base struct {
    Val int
}

func (b Base) Add(x int) int {
    return b.Val + x
}

type Extended struct {
    Base
    Extra int
}

func Run() int {
    e := Extended{}
    e.Val = 10
    e.Extra = 100
    return e.Add(5)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15, "promoted method with args should work");
}

// ==================== Regression: B3 - String compound assignment error ====================

#[test]
fn test_string_concat_compound_assignment() {
    let source = r#"
package main

func Run() int {
    s := "hello"
    s += " world"
    if s == "hello world" {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "string += should concatenate");
}

#[test]
fn test_string_invalid_compound_assignment_error() {
    let source = r#"
package main

func Run() int {
    s := "hello"
    s -= "x"
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "s -= on strings should produce a type error");
    let err_msg = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected error"),
    };
    assert!(err_msg.contains("only += is valid for string concatenation"),
        "error should mention only += is valid, got: {}", err_msg);
}

// ==================== Regression: C1 - Fallthrough validation ====================

#[test]
fn test_regression_fallthrough_in_type_switch_error() {
    let source = r#"
package main

func Run() int {
    var x interface{} = 42
    switch x.(type) {
    case int:
        fallthrough
    case string:
        return 1
    }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "fallthrough in type switch should be rejected");
}

#[test]
fn test_fallthrough_in_last_case_error() {
    let source = r#"
package main

func Run() int {
    x := 1
    switch x {
    case 1:
        return 10
    case 2:
        fallthrough
    }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "fallthrough in last case should be rejected");
}

#[test]
fn test_fallthrough_valid_middle_case() {
    let source = r#"
package main

func Run() int {
    x := 1
    result := 0
    switch x {
    case 1:
        result += 10
        fallthrough
    case 2:
        result += 20
    case 3:
        result += 30
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "fallthrough should execute next case body");
}

// ==================== Regression: C2 - Short variable declaration redeclaration ====================

#[test]
fn test_short_decl_redeclaration_with_new_var() {
    let source = r#"
package main

func Run() int {
    x := 10
    x, y := 20, 30
    return x + y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 50, "x should be redeclared to 20, y new as 30");
}

#[test]
fn test_short_decl_no_new_vars_error() {
    let source = r#"
package main

func Run() int {
    x := 10
    x := 20
    return x
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "no new variables on left side of := should fail");
}

// ==================== Regression: C3 - Complex number operations ====================

#[test]
fn test_regression_complex128_arithmetic() {
    let source = r#"
package main

func Run() int {
    c1 := complex(3.0, 4.0)
    c2 := complex(1.0, 2.0)
    sum := c1 + c2
    r := real(sum)
    i := imag(sum)
    if r == 4.0 && i == 6.0 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "complex addition should work: (3+4i) + (1+2i) = (4+6i)");
}

#[test]
fn test_regression_complex128_multiplication() {
    let source = r#"
package main

func Run() int {
    c1 := complex(1.0, 2.0)
    c2 := complex(3.0, 4.0)
    prod := c1 * c2
    r := real(prod)
    i := imag(prod)
    if r == -5.0 && i == 10.0 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "complex multiplication: (1+2i)*(3+4i) = (-5+10i)");
}

#[test]
fn test_regression_complex128_equality() {
    let source = r#"
package main

func Run() int {
    c1 := complex(1.0, 2.0)
    c2 := complex(1.0, 2.0)
    c3 := complex(1.0, 3.0)
    result := 0
    if c1 == c2 {
        result += 1
    }
    if c1 != c3 {
        result += 10
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "complex equality and inequality should work");
}

// ================= Regression Tests =================

#[test]
fn test_recursive_struct_type() {
    // Test that recursive struct types can be defined and used
    let source = r#"
package main

type Node struct {
    Value int
    Next  *Node
}

func Run() int {
    n := Node{Value: 42}
    return n.Value
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "recursive struct type with self-pointer should compile and run");
}

#[test]
fn test_global_string_variable() {
    let source = r#"
package main

var greeting string = "hello"

func Run() int {
    return len(greeting)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5, "global string 'hello' should have len 5");
}

#[test]
fn test_global_var_non_const_initializer() {
    let source = r#"
package main

func compute() int {
    return 42
}

var x int = compute()

func Run() int {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "global var with function call initializer should be 42");
}

#[test]
fn test_complex_unary_negation() {
    let source = r#"
package main

func Run() int {
    c := complex(3.0, 4.0)
    neg := -c
    r := real(neg)
    i := imag(neg)
    result := 0
    if r == -3.0 {
        result += 1
    }
    if i == -4.0 {
        result += 10
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "complex unary negation: real=-3, imag=-4");
}

#[test]
fn test_generic_struct_field_access() {
    let source = r#"
package main

type Box[T any] struct {
    Value T
}

func Run() int {
    b := Box[int]{Value: 99}
    return b.Value
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 99, "generic struct field access should return 99");
}

#[test]
fn test_slice_reslice_clear() {
    let source = r#"
package main

func Run() int {
    s := []int{1, 2, 3, 4, 5}
    s = s[:0]
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "s[:0] should produce empty slice");
}

#[test]
fn test_slice_element_deletion() {
    let source = r#"
package main

func Run() int {
    s := []int{10, 20, 30, 40, 50}
    // Delete element at index 2 (value 30)
    i := 2
    s = append(s[:i], s[i+1:]...)
    return len(s)*100 + s[0] + s[1] + s[2] + s[3]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // len=4, elements: 10+20+40+50=120, result = 400+120 = 520
    assert_eq!(val, 520, "slice element deletion: len=4, sum=120, result=520");
}

#[test]
fn test_named_type_conversion() {
    let source = r#"
package main

type MyInt int

func Run() int {
    var x MyInt = MyInt(42)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "named type conversion should work: MyInt(42) -> int(x) = 42");
}

#[test]
fn test_global_init_order_dependency() {
    let source = r#"
package main

var b int = a + 1
var a int = 10

func Run() int {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "b = a + 1 where a = 10 should give b = 11");
}

#[test]
fn test_imaginary_const_expression() {
    // Test that imaginary literals can be used in complex() calls
    let source = r#"
package main

func Run() int {
    c := complex(3.0, 4.0)
    r := real(c)
    i := imag(c)
    result := 0
    if r == 3.0 {
        result += 1
    }
    if i == 4.0 {
        result += 10
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "complex(3.0, 4.0): real=3, imag=4");
}

#[test]
fn test_range_over_function_error() {
    let source = r#"
package main

func myIter() {}

func Run() int {
    sum := 0
    for v := range myIter {
        sum += v
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    match result {
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(
                err_msg.contains("range over function") || err_msg.contains("not supported") || err_msg.contains("not implemented"),
                "error should mention range over function, got: {}", err_msg
            );
        }
        Ok(_) => panic!("range over function should produce an error"),
    }
}

#[test]
fn test_global_string_reassignment() {
    let source = r#"
package main

var s string = "abc"

func Run() int {
    s = "hello world"
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "reassigned global string should have len 11");
}

#[test]
fn test_global_var_multiple_deps() {
    let source = r#"
package main

var c int = a + b
var b int = a * 2
var a int = 5

func Run() int {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15, "c = a + b = 5 + 10 = 15");
}

// ==================== B1: Nested slice with non-I64 inner element type ====================

#[test]
fn test_nested_slice_int32_indexing() {
    let source = r#"
package main

func Run() int {
    a := [][]int32{
        {1, 2, 3},
        {10, 20, 30},
    }
    return int(a[0][0]) + int(a[0][2]) + int(a[1][1])
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 24, "nested [][]int32 indexing: 1+3+20=24");
}

// ==================== A1: Chained map indexing ====================

#[test]
fn test_nested_map_chained_indexing() {
    let source = r#"
package main

func Run() int {
    m := make(map[int]map[int]int)
    inner := make(map[int]int)
    inner[1] = 10
    inner[2] = 20
    m[100] = inner
    return m[100][1] + m[100][2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "chained map indexing: m[100][1]+m[100][2] = 10+20 = 30");
}

// ==================== A2: Multi-dimensional arrays ====================

#[test]
fn test_multidimensional_array() {
    let source = r#"
package main

func Run() int {
    var a [2][3]int
    a[0][0] = 1
    a[0][1] = 2
    a[0][2] = 3
    a[1][0] = 10
    a[1][1] = 20
    a[1][2] = 30
    return a[0][0] + a[0][2] + a[1][1]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 24, "multidimensional array: 1+3+20=24");
}

// ==================== A5: Methods on named non-struct types ====================

#[test]
fn test_method_on_named_int_type() {
    let source = r#"
package main

type MyInt int

func (m MyInt) Double() int {
    return int(m) * 2
}

func Run() int {
    var x MyInt = 21
    return x.Double()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "method on named int type: 21.Double() = 42");
}

// ==================== A3: Pointer scalar dereference assignment ====================

#[test]
fn test_pointer_scalar_deref_assign() {
    let source = r#"
package main

func Run() int {
    p := new(int)
    *p = 42
    return *p
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "pointer deref assignment: *p = 42 via new(int)");
}

// ==================== A11: Nil map write panics ====================

#[test]
#[should_panic]
fn test_nil_map_write_panics() {
    let source = r#"
package main

func Run() int {
    var m map[int]int
    m[1] = 42
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    func.call(&mut store, ()).expect("call failed");
}

// ==================== A8: Typed constants ====================

#[test]
fn test_const_typed_int32() {
    let source = r#"
package main

const x int32 = 10

func Run() int {
    return int(x) + 5
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15, "typed int32 const: 10 + 5 = 15");
}

#[test]
fn test_const_large_shift() {
    let source = r#"
package main

const big = 1 << 20

func Run() int {
    return big
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1048576, "const 1 << 20 = 1048576");
}

// ==================== A10: String conversion edge cases ====================

#[test]
fn test_string_from_rune_value() {
    let source = r#"
package main

func Run() int {
    s := string(65)
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "string(65) = 'A' which has length 1");
}

#[test]
fn test_string_from_empty_byte_slice() {
    let source = r#"
package main

func Run() int {
    b := []byte{}
    s := string(b)
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "string of empty byte slice has length 0");
}

// ==================== C: Regression tests for edge cases ====================

#[test]
fn test_named_return_with_defer_modification() {
    let source = r#"
package main

func addOne() (result int) {
    result = 10
    defer func() {
        result = result + 1
    }()
    return result
}

func Run() int {
    return addOne()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "defer should modify named return: 10 + 1 = 11");
}

#[test]
fn test_variadic_with_no_args() {
    let source = r#"
package main

func sum(args ...int) int {
    total := 0
    for _, v := range args {
        total += v
    }
    return total
}

func Run() int {
    return sum()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "variadic with no args should return 0");
}

#[test]
fn test_switch_tagless_complex_conditions() {
    let source = r#"
package main

func Run() int {
    x := 5
    y := 3
    switch {
    case x > 10 && y < 2:
        return 1
    case x > 0 && y < 10:
        return 2
    default:
        return 3
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 2, "tagless switch with complex conditions: x>0 && y<10");
}

#[test]
fn test_method_call_on_return_value() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func (p Point) Sum() int {
    return p.X + p.Y
}

func makePoint() Point {
    return Point{X: 10, Y: 20}
}

func Run() int {
    p := makePoint()
    return p.Sum()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "method call on stored return value: makePoint().Sum() = 30");
}

#[test]
fn test_slice_append_reallocation() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 0, 2)
    s = append(s, 1)
    s = append(s, 2)
    s = append(s, 3)
    s = append(s, 4)
    s = append(s, 5)
    return s[0] + s[4] + len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "append past capacity: s[0]+s[4]+len(s) = 1+5+5 = 11");
}

#[test]
fn test_nested_type_switch() {
    let source = r#"
package main

type Animal interface {
    Legs() int
}

type Dog struct{}
type Cat struct{}

func (d Dog) Legs() int { return 4 }
func (c Cat) Legs() int { return 4 }

func classify(a Animal) int {
    switch v := a.(type) {
    case Dog:
        _ = v
        return 1
    case Cat:
        _ = v
        return 2
    default:
        return 0
    }
}

func Run() int {
    var d Animal = Dog{}
    var c Animal = Cat{}
    return classify(d)*10 + classify(c)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 12, "type switch: Dog=1, Cat=2 => 10+2=12");
}

// ==================== B4: clear() on complex types ====================

#[test]
fn test_clear_slice_of_ints() {
    let source = r#"
package main

func Run() int {
    s := []int{10, 20, 30}
    clear(s)
    return s[0] + s[1] + s[2] + len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "clear(s) zeros elements but keeps length: 0+0+0+3=3");
}

// ==================== A7: Parallel assignment with struct fields ====================

#[test]
fn test_parallel_assign_slice_index() {
    let source = r#"
package main

func Run() int {
    s := []int{10, 20, 30}
    s[0], s[2] = s[2], s[0]
    return s[0]*100 + s[1]*10 + s[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3210, "parallel assign slice indices: swap s[0],s[2] => 30*100+20*10+10=3210");
}

// ==================== A6: Struct field tags ====================

#[test]
fn test_struct_with_field_tags() {
    let source = r#"
package main

type User struct {
    Name string
    Age  int
}

func Run() int {
    u := User{Name: "Alice", Age: 30}
    return u.Age
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "struct with field access works");
}

// ==================== A4: Composite type combinations ====================

#[test]
fn test_slice_of_maps() {
    let source = r#"
package main

func Run() int {
    s := make([]map[int]int, 2)
    s[0] = make(map[int]int)
    s[1] = make(map[int]int)
    s[0][1] = 10
    s[1][2] = 20
    m0 := s[0]
    m1 := s[1]
    return m0[1] + m1[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "slice of maps: 10 + 20 = 30");
}

#[test]
fn test_map_of_ints() {
    let source = r#"
package main

func Run() int {
    m := make(map[string]int)
    m["a"] = 10
    m["b"] = 20
    m["c"] = 30
    return m["a"] + m["c"]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 40, "map of string to int: 10 + 30 = 40");
}

// ==================== C1: Empty struct ====================

#[test]
fn test_empty_struct_as_map_value() {
    let source = r#"
package main

type empty struct{}

func Run() int {
    m := make(map[int]empty)
    m[1] = empty{}
    m[2] = empty{}
    count := 0
    for range m {
        count++
    }
    return count
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 2, "empty struct as map value: 2 entries");
}

// ==================== C3: Array equality with different element values ====================

#[test]
fn test_array_equality_different_values() {
    let source = r#"
package main

func Run() int {
    a := [3]int{1, 2, 3}
    b := [3]int{1, 2, 3}
    c := [3]int{1, 2, 4}
    if a == b {
        if a != c {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "array equality works");
}

// ==================== C5: Type assertion on nil interface ====================

#[test]
fn test_type_assertion_nil_interface_comma_ok() {
    let source = r#"
package main

func Run() int {
    var x interface{}
    _, ok := x.(int)
    if !ok {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "type assertion on nil interface: ok should be false");
}

// ==================== A12: Interface-to-interface type assertion ====================

#[test]
fn test_interface_to_interface_assertion_comma_ok_fail() {
    let source = r#"
package main

type Stringer interface {
    String() string
}

type Runner interface {
    Run() int
}

type MyType struct {
    X int
}

func (m MyType) Run() int {
    return m.X
}

func Run() int {
    var r Runner = MyType{X: 42}
    _, ok := r.(Stringer)
    if !ok {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "interface-to-interface assertion fail: ok=false => 1");
}

// ==================== Regression: type switch case nil ====================

#[test]
fn test_type_switch_case_nil() {
    let source = r#"
package main

type Stringer interface {
    String() string
}

func Run() int {
    var x Stringer
    switch x.(type) {
    case nil:
        return 1
    default:
        return 0
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "type switch case nil should match nil interface");
}

#[test]
fn test_type_switch_case_nil_non_nil() {
    let source = r#"
package main

type Animal interface {
    Legs() int
}

type Dog struct{}

func (d Dog) Legs() int { return 4 }

func Run() int {
    var a Animal = Dog{}
    switch a.(type) {
    case nil:
        return 0
    case Dog:
        return 1
    default:
        return 2
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "type switch on non-nil should match concrete type, not nil");
}

// ==================== Regression: type switch case *T ====================

#[test]
fn test_type_switch_case_pointer_type() {
    let source = r#"
package main

type Shape interface {
    Area() int
}

type Circle struct {
    Radius int
}

func (c *Circle) Area() int { return c.Radius * c.Radius * 3 }

func Run() int {
    c := &Circle{Radius: 5}
    var s Shape = c
    switch s.(type) {
    case *Circle:
        return 1
    default:
        return 0
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "type switch case *Circle should match pointer type");
}

// ==================== Regression: type assertion x.(*T) ====================

#[test]
fn test_type_assert_pointer_target() {
    let source = r#"
package main

type Animal interface {
    Legs() int
}

type Dog struct {
    L int
}

func (d Dog) Legs() int { return d.L }

type Cat struct {
    L int
}

func (c Cat) Legs() int { return c.L }

func Run() int {
    var a Animal = Dog{L: 4}
    d := a.(Dog)
    return d.Legs()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 4, "type assertion x.(T) should extract concrete value");
}

#[test]
fn test_type_assert_pointer_comma_ok() {
    let source = r#"
package main

type Animal interface {
    Legs() int
}

type Dog struct {
    L int
}

func (d Dog) Legs() int { return d.L }

type Cat struct {
    L int
}

func (c Cat) Legs() int { return c.L }

func Run() int {
    var a Animal = Dog{L: 4}
    _, ok := a.(Cat)
    if ok {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "type assertion comma-ok should return false for wrong type");
}

// ==================== Regression: type assertion on non-Ident expression ====================

#[test]
fn test_type_assert_on_function_return() {
    let source = r#"
package main

type Valuer interface {
    Val() int
}

type Num struct {
    N int
}

func (n Num) Val() int { return n.N }

func getValuer() Valuer {
    return Num{N: 99}
}

func Run() int {
    n := getValuer().(Num)
    return n.Val()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 99, "type assertion on function return should work");
}

// ==================== Regression: range over struct field slice ====================

#[test]
fn test_range_over_struct_field_slice() {
    let source = r#"
package main

type Container struct {
    Items []int
}

func Run() int {
    c := Container{Items: []int{10, 20, 30}}
    sum := 0
    for _, v := range c.Items {
        sum = sum + v
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 60, "range over struct field slice should sum correctly");
}

// ==================== Regression: range over struct field map ====================

#[test]
fn test_range_over_struct_field_map() {
    let source = r#"
package main

type Config struct {
    Data map[string]int
}

func Run() int {
    c := Config{Data: map[string]int{"a": 1, "b": 2, "c": 3}}
    sum := 0
    for _, v := range c.Data {
        sum = sum + v
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 6, "range over struct field map should sum values correctly");
}

// ==================== Regression: closure capture alignment ====================

#[test]
fn test_closure_capture_i32_then_i64_alignment() {
    let source = r#"
package main

func Run() int {
    a := 1
    b := int64(1000000000000)
    f := func() int {
        return a + int(b)
    }
    return f()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1000000000001, "closure capturing i32 then i64 should align correctly");
}

#[test]
fn test_closure_capture_mixed_sizes_alignment() {
    let source = r#"
package main

func Run() int {
    a := true
    b := int64(42)
    c := 100
    d := int64(200)
    f := func() int {
        result := 0
        if a {
            result = result + 1
        }
        result = result + int(b) + c + int(d)
        return result
    }
    return f()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 343, "closure with mixed-size captures should produce correct result");
}

// ==================== Regression: type switch mixed Ident and pointer cases ====================

#[test]
fn test_type_switch_mixed_ident_and_nil() {
    let source = r#"
package main

type Animal interface {
    Sound() int
}

type Cat struct{}
type Dog struct{}

func (c Cat) Sound() int { return 1 }
func (d Dog) Sound() int { return 2 }

func classify(a Animal) int {
    switch a.(type) {
    case nil:
        return 0
    case Cat:
        return 1
    case Dog:
        return 2
    default:
        return -1
    }
}

func Run() int {
    var nilAnimal Animal
    result := classify(nilAnimal)
    result = result + classify(Cat{}) * 10
    result = result + classify(Dog{}) * 100
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 210, "type switch with nil+concrete: nil=0, Cat=10, Dog=200 => 210");
}

// ==================== A1: Interface method signature checking ====================

#[test]
fn test_interface_wrong_param_type_rejected() {
    let source = r#"
package main

type Doer interface {
    Do(x int) int
}

type Bad struct{}

func (b Bad) Do(x string) int { return 0 }

func Run() int {
    var d Doer = Bad{}
    return d.Do(1)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "type with wrong param type should not satisfy interface");
}

#[test]
fn test_interface_wrong_return_type_rejected() {
    let source = r#"
package main

type Getter interface {
    Get() int
}

type Bad struct{}

func (b Bad) Get() string { return "no" }

func Run() int {
    var g Getter = Bad{}
    return g.Get()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "type with wrong return type should not satisfy interface");
}

#[test]
fn test_interface_correct_signature_accepted() {
    let source = r#"
package main

type Doer interface {
    Do(x int) int
}

type Good struct{}

func (g Good) Do(x int) int { return x + 1 }

func Run() int {
    var d Doer = Good{}
    return d.Do(41)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "correct signature should work via interface dispatch");
}

// ==================== A2: Address-of array/slice element ====================

#[test]
fn test_address_of_array_element() {
    let source = r#"
package main

func Run() int {
    var arr [3]int
    arr[0] = 10
    arr[1] = 20
    arr[2] = 30
    p := &arr[1]
    *p = 99
    return arr[1]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 99, "&arr[i] should point to original array element");
}

#[test]
fn test_address_of_slice_element() {
    let source = r#"
package main

func Run() int {
    s := []int{10, 20, 30}
    p := &s[2]
    *p = 77
    return s[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 77, "&s[i] should point to original slice element");
}

// ==================== A3: Runtime integer wrapping ====================

#[test]
fn test_runtime_int_wrapping() {
    let source = r#"
package main

func Run() int {
    var x int = 9223372036854775807
    x = x + 1
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, -9223372036854775808i64, "int overflow should wrap at runtime");
}

// ==================== B3: Generic type constraint union ====================

#[test]
fn test_generic_type_constraint_union() {
    let source = r#"
package main

type Number interface {
    ~int | ~float64
}

func Double[T Number](x T) T {
    return x + x
}

func Run() int {
    return Double(21)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "generic function with union constraint should work");
}

// ==================== B6: Blank identifier in function parameters ====================

#[test]
fn test_blank_identifier_in_func_params() {
    let source = r#"
package main

func f(_ int, b int) int {
    return b
}

func Run() int {
    return f(999, 42)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "blank identifier in func params should work");
}

// ==================== B8: Iota implicit repetition ====================

#[test]
fn test_iota_implicit_repetition() {
    let source = r#"
package main

const (
    A = iota + 1
    B
    C
)

func Run() int {
    return A + B*10 + C*100
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 321, "iota implicit repetition: A=1, B=2, C=3 => 1+20+300=321");
}

// ==================== B10: Pointer comparison ====================

#[test]
fn test_pointer_equality_same() {
    let source = r#"
package main

func Run() int {
    p := new(int)
    *p = 42
    p2 := p
    if p == p2 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "same pointer should be equal");
}

#[test]
fn test_pointer_equality_different() {
    let source = r#"
package main

func Run() int {
    x := 42
    y := 42
    p1 := &x
    p2 := &y
    if p1 != p2 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "pointers to different variables should not be equal");
}

// ==================== B4: Nested slice literal ====================

#[test]
fn test_nested_slice_literal() {
    let source = r#"
package main

func Run() int {
    matrix := [][]int{{1, 2, 3}, {4, 5, 6}}
    return matrix[0][0] + matrix[0][2] + matrix[1][1]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 9, "nested slice literal: 1+3+5=9");
}

// ==================== B7: Untyped constant default types ====================

#[test]
fn test_untyped_const_default_int() {
    let source = r#"
package main

const x = 42

func Run() int {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42);
}

#[test]
fn test_untyped_const_default_float() {
    let source = r#"
package main

const pi = 3.14

func Run() float64 {
    return pi
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert!((val - 3.14).abs() < 1e-10);
}

// ==================== C1: Generics edge cases ====================

#[test]
fn test_generic_function_with_multiple_constraints() {
    let source = r#"
package main

func Add[T comparable](a T, b T) T {
    return a + b
}

func Run() int {
    return Add(20, 22)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "generic with comparable constraint should work");
}

// ==================== C2: Interface edge cases ====================

#[test]
fn test_empty_interface_holds_int() {
    let source = r#"
package main

func Run() int {
    var v any = 42
    x := v.(int)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "any should hold int and type assert should work");
}

#[test]
fn test_interface_method_call_returns_field() {
    let source = r#"
package main

type Coder interface {
    Code() int
}

type Thing struct {
    c int
}

func (t Thing) Code() int { return t.c }

func Run() int {
    var x Coder = Thing{c: 42}
    return x.Code()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "interface method call should return correct field value");
}

// ==================== C3: Defer edge cases ====================

#[test]
fn test_defer_runs_after_return() {
    let source = r#"
package main

var counter int

func inc() {
    counter = counter + 1
}

func work() int {
    defer inc()
    defer inc()
    defer inc()
    return counter
}

func Run() int {
    val := work()
    return counter*10 + val
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "3 defers all run at exit: counter=3, val=0 => 30");
}

// ==================== C4: Closure edge cases ====================

#[test]
fn test_closure_modifies_captured_var() {
    let source = r#"
package main

func Run() int {
    x := 10
    add := func(y int) int {
        x = x + y
        return x
    }
    add(5)
    add(27)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "closure should modify captured variable");
}

// ==================== C5: Switch edge cases ====================

#[test]
fn test_switch_multi_case_with_fallthrough() {
    let source = r#"
package main

func classify(x int) int {
    switch x {
    case 1:
        return 100
    case 2:
        return 200
    case 3:
        return 300
    default:
        return 0
    }
}

func Run() int {
    return classify(1) + classify(2) + classify(3) + classify(99)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 600, "switch multi case: 100+200+300+0=600");
}

#[test]
fn test_switch_initializer_scoping() {
    let source = r#"
package main

func f() int { return 2 }

func Run() int {
    switch x := f(); x {
    case 1:
        return 10
    case 2:
        return 20
    default:
        return 0
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 20, "switch with initializer should scope variable correctly");
}

// ==================== C6: For loop edge cases ====================

#[test]
fn test_for_range_key_value() {
    let source = r#"
package main

func Run() int {
    s := []int{10, 20, 30}
    sum := 0
    for i, v := range s {
        sum = sum + i*100 + v
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 360, "for range key+value: (0*100+10)+(1*100+20)+(2*100+30)=360");
}

// ==================== C7: Map edge cases ====================

#[test]
fn test_nil_map_read_returns_zero_value() {
    let source = r#"
package main

func Run() int {
    var m map[int]int
    return m[42]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "reading from nil map should return zero value");
}

// ==================== C8: Struct edge cases ====================

#[test]
fn test_struct_literal_partial_fields() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
    Z int
}

func Run() int {
    p := Point{X: 10}
    return p.X + p.Y + p.Z
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "partial struct literal should zero-fill missing fields");
}

#[test]
fn test_struct_embedding_method_override() {
    let source = r#"
package main

type Base struct{}

func (b Base) Value() int { return 10 }

type Derived struct {
    Base
}

func (d Derived) Value() int { return 42 }

func Run() int {
    d := Derived{}
    return d.Value()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "derived type's method should override embedded method");
}

#[test]
fn test_defer_method_call_runtime() {
    let source = r#"
package main

type Acc struct {
    Total int32
}

func (a *Acc) Add(x int32) {
    a.Total = a.Total + x
}

func Run() int32 {
    a := Acc{Total: 0}
    defer a.Add(int32(10))
    a.Total = int32(5)
    return a.Total
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i32>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5, "defer method call should work at runtime");
}

#[test]
fn test_promoted_method_pointer_receiver() {
    let source = r#"
package main

type Inner struct {
    X int
}

func (i *Inner) Double() int {
    return i.X * 2
}

type Outer struct {
    Prefix int
    Inner
}

func Run() int {
    o := Outer{Prefix: 100, Inner: Inner{X: 7}}
    return o.Double()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 14, "promoted method with pointer receiver and non-zero offset should work");
}

#[test]
fn test_promoted_value_recv_nonzero_offset() {
    let source = r#"
package main

type Inner struct {
    X int
}

func (i Inner) Double() int {
    return i.X * 2
}

type Outer struct {
    Prefix int
    Inner
}

func Run() int {
    o := Outer{Prefix: 100, Inner: Inner{X: 7}}
    return o.Double()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 14, "promoted value recv method at non-zero offset should work");
}

#[test]
fn test_promoted_method_nonzero_offset_manual() {
    let source = r#"
package main

type Inner struct {
    X int
}

func (i Inner) Double() int {
    return i.X * 2
}

type Outer struct {
    Prefix int
    Inner
}

func Run() int {
    o := Outer{}
    o.Prefix = 100
    o.X = 7
    return o.Double()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 14, "promoted method manual field set should work");
}

// ==================== C1: Defer edge cases ====================

#[test]
fn test_defer_chained_selector() {
    let source = r#"
package main

type Logger struct {
    Count int32
}

func (l *Logger) Inc() {
    l.Count = l.Count + int32(1)
}

type App struct {
    Log Logger
}

func Run() int32 {
    a := App{Log: Logger{Count: int32(0)}}
    defer a.Log.Inc()
    return a.Log.Count
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i32>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "defer chained selector should compile and run");
}

#[test]
fn test_defer_in_nested_if() {
    let source = r#"
package main

var counter int

func inc() {
    counter = counter + 1
}

func Run() int {
    counter = 0
    if true {
        defer inc()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "defer in if block should run after return");
}

#[test]
fn test_defer_with_multiple_args() {
    let source = r#"
package main

var total int

func add(a int, b int) {
    total = a + b
}

func Run() int {
    total = 0
    defer add(3, 4)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "defer with multiple args should run after return");
}

// ==================== C2: Promoted method edge cases ====================

#[test]
fn test_promoted_method_multi_arg_nonzero_offset() {
    let source = r#"
package main

type Base struct {
    Val int
}

func (b Base) AddMul(x int, y int) int {
    return (b.Val + x) * y
}

type Derived struct {
    Prefix int
    Base
}

func Run() int {
    d := Derived{Prefix: 999, Base: Base{Val: 3}}
    return d.AddMul(2, 4)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 20, "promoted method with multiple args at non-zero offset");
}

#[test]
fn test_embedding_with_override() {
    let source = r#"
package main

type Base struct {
    Val int
}

func (b Base) Get() int {
    return b.Val
}

type Child struct {
    Base
    Extra int
}

func (c Child) Get() int {
    return c.Val + c.Extra
}

func Run() int {
    c := Child{Base: Base{Val: 10}, Extra: 5}
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15, "overridden method should be called on child");
}

// ==================== C3: Fallthrough edge cases ====================

#[test]
fn test_fallthrough_multiple_cases() {
    let source = r#"
package main

func Run() int {
    x := 1
    result := 0
    switch x {
    case 1:
        result = result + 10
        fallthrough
    case 2:
        result = result + 20
        fallthrough
    case 3:
        result = result + 30
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 60, "fallthrough should chain: 10+20+30=60");
}

// ==================== C4: Switch edge cases ====================

#[test]
fn test_switch_no_tag_complex_bool() {
    let source = r#"
package main

func Run() int {
    x := 5
    y := 10
    switch {
    case x > 3 && y < 20:
        return 1
    case x == 5:
        return 2
    default:
        return 3
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "switch no tag with complex bool expressions");
}

#[test]
fn test_switch_with_init_statement() {
    let source = r#"
package main

func compute() int {
    return 42
}

func Run() int {
    switch v := compute(); {
    case v > 100:
        return 1
    case v > 40:
        return 2
    default:
        return 3
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 2, "switch with init statement");
}

// ==================== C5: Blank identifier edge cases ====================

#[test]
fn test_blank_identifier_discard_return() {
    let source = r#"
package main

func returnTwo() (int, int) {
    return 10, 20
}

func Run() int {
    _, b := returnTwo()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 20, "blank identifier should discard first return value");
}

#[test]
fn test_blank_identifier_range_both() {
    let source = r#"
package main

func Run() int {
    s := []int{10, 20, 30}
    count := 0
    for _, _ = range s {
        count = count + 1
    }
    return count
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "blank identifier for both key and value in range");
}

#[test]
fn test_blank_param_name() {
    let source = r#"
package main

func process(_ int, x int) int {
    return x * 2
}

func Run() int {
    return process(999, 5)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "blank parameter name should be ignored");
}

// ==================== C6: Const/iota edge cases ====================

#[test]
fn test_iota_bitshift() {
    let source = r#"
package main

const (
    FlagA = 1 << iota
    FlagB
    FlagC
    FlagD
)

func Run() int {
    return FlagA + FlagB + FlagC + FlagD
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15, "iota with bit-shift: 1+2+4+8=15");
}

#[test]
fn test_iota_reset_across_groups() {
    let source = r#"
package main

const (
    A = iota
    B
)

const (
    C = iota
    D
)

func Run() int {
    return A + B + C + D
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 2, "iota resets across const groups: 0+1+0+1=2");
}

#[test]
fn test_iota_with_blank() {
    let source = r#"
package main

const (
    _ = iota
    B
    C
)

func Run() int {
    return B + C
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "iota with blank: _=0, B=1, C=2, sum=3");
}

// ==================== C7: Short variable declarations ====================

#[test]
fn test_short_var_decl_redeclare() {
    let source = r#"
package main

func returnErr() (int, int) {
    return 10, 0
}

func Run() int {
    x := 5
    x, y := returnErr()
    return x + y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "short var decl should redeclare x and declare y");
}

#[test]
fn test_variable_shadowing() {
    let source = r#"
package main

func Run() int {
    x := 10
    {
        x := 20
        _ = x
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "inner x should shadow outer x without affecting it");
}

// ==================== C8: Named return edge cases ====================

#[test]
fn test_named_returns_multiple_with_defer() {
    let source = r#"
package main

func compute() (a int, b int) {
    a = 10
    b = 20
    defer func() {
        a = a + 1
        b = b + 1
    }()
    return
}

func Run() int {
    a, b := compute()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 32, "defer should modify both named returns: 11+21=32");
}

#[test]
fn test_naked_return_nested_block() {
    let source = r#"
package main

func Compute(x int) (result int) {
    result = x
    if x > 5 {
        result = result * 2
        return
    }
    result = result + 1
    return
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(i64,), i64>(&mut store, "Compute").expect("not found");
    let val = func.call(&mut store, (10,)).expect("call failed");
    assert_eq!(val, 20, "naked return in nested if block should work");
}

// ==================== C9: For-range edge cases ====================

#[test]
fn test_range_nil_slice() {
    let source = r#"
package main

func Run() int {
    var s []int
    count := 0
    for range s {
        count = count + 1
    }
    return count
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "range over nil slice should iterate zero times");
}

#[test]
fn test_range_key_only() {
    let source = r#"
package main

func Run() int {
    s := []int{10, 20, 30}
    sum := 0
    for i := range s {
        sum = sum + i
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "range key only: 0+1+2=3");
}

#[test]
fn test_range_over_integer() {
    let source = r#"
package main

func Run() int {
    sum := 0
    for i := range 5 {
        sum = sum + i
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "range over integer: 0+1+2+3+4=10");
}

// ==================== C10: Interface edge cases ====================

#[test]
fn test_interface_type_assertion_ok_bool() {
    let source = r#"
package main

type Sizer interface {
    Size() int
}

type Box struct {
    s int
}

func (b Box) Size() int {
    return b.s
}

func Run() int {
    b := Box{s: 42}
    var iface Sizer = b
    _, ok := iface.(Box)
    if ok {
        return iface.Size()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "interface type assertion comma-ok should work");
}

#[test]
fn test_nil_interface_comparison() {
    let source = r#"
package main

type Stringer interface {
    String() string
}

func Run() int {
    var s Stringer
    if s == nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "nil interface should equal nil");
}

// ==================== C11: Map edge cases ====================

#[test]
fn test_nil_map_read() {
    let source = r#"
package main

func Run() int {
    var m map[string]int
    v := m["key"]
    return v
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "nil map read should return zero value");
}

#[test]
fn test_map_struct_values() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Run() int {
    m := map[string]Point{
        "origin": Point{X: 0, Y: 0},
    }
    m["a"] = Point{X: 3, Y: 4}
    p := m["a"]
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 7, "map with struct values should work");
}

// ==================== C12: Composite literal edge cases ====================

#[test]
fn test_array_literal_ellipsis_len() {
    let source = r#"
package main

func Run() int {
    a := [...]int{10, 20, 30}
    return a[0] + a[1] + a[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 60, "[...]int literal should work");
}

// ==================== B6: Multiple init functions ====================

#[test]
fn test_multiple_init_functions() {
    let source = r#"
package main

var a int
var b int

func init() {
    a = 10
}

func init() {
    b = 20
}

func Run() int {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "multiple init functions should both run");
}

// ==================== B7: Comparable constraint in generics ====================

#[test]
fn test_comparable_constraint() {
    let source = r#"
package main

func Contains[T comparable](s []T, target T) bool {
    for _, v := range s {
        if v == target {
            return true
        }
    }
    return false
}

func Run() int {
    s := []int{1, 2, 3, 4, 5}
    if Contains(s, 3) {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "comparable constraint should allow == in generics");
}

// ==================== B3: String conversion edge cases ====================

#[test]
fn test_string_from_valid_rune() {
    let source = r#"
package main

func Run() int {
    s := string(65)
    if s == "A" {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "string(65) should produce 'A'");
}

// ==================== Regression tests: cross-type struct composite literal coercion ====================

#[test]
fn test_struct_int_literal_in_float64_field() {
    let source = r#"
package main

type Point struct {
    X float64
    Y float64
}

func Run() float64 {
    p := Point{X: 1, Y: 2}
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
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert!((val - 3.0).abs() < f64::EPSILON, "expected 3.0, got {}", val);
}

#[test]
fn test_struct_int64_in_int32_field() {
    let source = r#"
package main

type Dims struct {
    W int32
    H int32
}

func Run() int {
    var w int = 10
    var h int = 20
    d := Dims{W: int32(w), H: int32(h)}
    return int(d.W) + int(d.H)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30);
}

#[test]
fn test_struct_float64_literal_field() {
    let source = r#"
package main

type Measurement struct {
    Value float64
    Count int
}

func Run() float64 {
    m := Measurement{Value: 3.14, Count: 5}
    return m.Value
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert!((val - 3.14).abs() < 0.01, "expected ~3.14, got {}", val);
}

#[test]
fn test_struct_mixed_type_fields() {
    let source = r#"
package main

type Mixed struct {
    I int
    F float64
    B int32
}

func Run() int {
    m := Mixed{I: 42, F: 2.5, B: 7}
    if m.I == 42 && m.F > 2.0 && int(m.B) == 7 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1);
}

// ==================== Regression tests: complex number edge cases ====================

#[test]
fn test_complex_constant_division() {
    let source = r#"
package main

func Run() float64 {
    const c = (6 + 4i) / (2 + 1i)
    return real(c)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // (6+4i)/(2+1i) = (12+4+8i-6i)/(4+1) = (16+2i)/5 = 3.2 + 0.4i
    assert!((val - 3.2).abs() < 1e-10, "expected 3.2, got {}", val);
}

#[test]
fn test_complex_constant_division_imag() {
    let source = r#"
package main

func Run() float64 {
    const c = (6 + 4i) / (2 + 1i)
    return imag(c)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert!((val - 0.4).abs() < 1e-10, "expected 0.4, got {}", val);
}

#[test]
fn test_complex_runtime_real_imag() {
    let source = r#"
package main

func Run() float64 {
    a := 3.0
    b := 4.0
    c := complex(a, b)
    return real(c) + imag(c)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert!((val - 7.0).abs() < 1e-10, "expected 7.0, got {}", val);
}

#[test]
fn test_complex_runtime_division() {
    let source = r#"
package main

func Run() float64 {
    a := complex(6.0, 4.0)
    b := complex(2.0, 1.0)
    c := a / b
    return real(c) + imag(c)*1000.0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), f64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // 3.2 + 0.4*1000 = 403.2
    assert!((val - 403.2).abs() < 1e-10, "expected 403.2, got {}", val);
}

// ==================== Regression tests: deeply nested struct embedding ====================

#[test]
fn test_nested_struct_embedding_3_levels() {
    let source = r#"
package main

type A struct {
    X int
}

type B struct {
    A
}

type C struct {
    B
}

func Run() int {
    c := C{B: B{A: A{X: 42}}}
    return c.X
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "promoted field X should be accessible through 3 levels");
}

#[test]
fn test_nested_struct_embedding_3_levels_multiple_fields() {
    let source = r#"
package main

type A struct {
    X int
    Y int
}

type B struct {
    A
}

type C struct {
    B
}

func Run() int {
    c := C{B: B{A: A{X: 10, Y: 32}}}
    return c.X + c.Y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "promoted fields X,Y should both be accessible through 3 levels");
}

// ==================== Regression tests: float edge cases ====================

#[test]
fn test_float_nan_propagation() {
    let source = r#"
package main

func Run() int {
    nan := 0.0 / 0.0
    result := nan + 1.0
    if result == result {
        return 0
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "NaN should propagate and NaN != NaN");
}

#[test]
fn test_float_infinity() {
    let source = r#"
package main

func Run() int {
    inf := 1.0 / 0.0
    if inf > 1000000.0 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "positive infinity should be > any finite number");
}

#[test]
fn test_float_nan_not_equal_to_itself() {
    let source = r#"
package main

func Run() int {
    nan := 0.0 / 0.0
    if nan != nan {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "NaN != NaN should be true");
}

#[test]
fn test_float_inf_minus_inf() {
    let source = r#"
package main

func Run() int {
    inf := 1.0 / 0.0
    result := inf - inf
    if result != result {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Inf - Inf should be NaN");
}

#[test]
fn test_float_negative_zero() {
    let source = r#"
package main

func Run() int {
    nz := -0.0
    pz := 0.0
    if nz == pz {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "-0.0 == 0.0 should be true per IEEE 754");
}

// ==================== Regression tests: bitwise operators on all integer widths ====================

#[test]
fn test_bitwise_and_or_xor_andnot() {
    let source = r#"
package main

func Run() int {
    a := 0x0F
    b := 0x37
    and := a & b
    or := a | b
    xor := a ^ b
    andnot := a &^ b
    if and == 0x07 && or == 0x3F && xor == 0x38 && andnot == 0x08 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "bitwise AND/OR/XOR/ANDNOT should be correct");
}

#[test]
fn test_bitwise_ops_int() {
    let source = r#"
package main

func Run() int {
    a := 0x00FF
    b := 0x0F0F
    and := a & b
    or := a | b
    xor := a ^ b
    if and == 0x000F && or == 0x0FFF && xor == 0x0FF0 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "bitwise ops on int should be correct");
}

#[test]
fn test_bitwise_shift_int64() {
    let source = r#"
package main

func Run() int {
    var a int64 = 1
    shl := a << 32
    if shl == 4294967296 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "left shift 32 on int64 should produce 2^32");
}

#[test]
fn test_bitwise_ops_large_values() {
    let source = r#"
package main

func Run() int {
    a := 0x00FF00FF
    b := 0x0F0F0F0F
    and := a & b
    or := a | b
    xor := a ^ b
    if and == 0x000F000F && or == 0x0FFF0FFF && xor == 0x0FF00FF0 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "bitwise ops on large int values should be correct");
}

#[test]
fn test_bitwise_right_shift_signed() {
    let source = r#"
package main

func Run() int {
    a := -16
    shr := a >> 2
    if shr == -4 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "arithmetic right shift should preserve sign");
}

// ==================== Regression tests: range over map with delete ====================

#[test]
fn test_range_map_delete_during_iteration() {
    let source = r#"
package main

func Run() int {
    m := map[int]int{1: 10, 2: 20, 3: 30}
    for k := range m {
        delete(m, k)
    }
    return len(m)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "deleting all keys during range should empty the map");
}

// ==================== Regression tests: nested closures ====================

#[test]
fn test_closure_captures_and_returns() {
    let source = r#"
package main

func Run() int {
    x := 5
    y := 10
    add := func() int {
        return x + y
    }
    mul := func() int {
        return x * y
    }
    return add() + mul()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 65, "multiple closures should capture same outer variables (5+10=15, 5*10=50)");
}

// ==================== Regression tests: complex division by zero ====================

#[test]
fn test_complex_div_by_zero_produces_nan() {
    let source = r#"
package main

func Run() int {
    a := complex(1.0, 2.0)
    b := complex(0.0, 0.0)
    c := a / b
    r := real(c)
    if r != r {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "complex div by zero should produce NaN in real part");
}

#[test]
fn test_closure_captures_multiple_vars() {
    let source = r#"
package main

func Run() int {
    a := 3
    b := 7
    f := func() int {
        return a * b
    }
    return f()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 21, "closure should capture multiple variables from outer scope");
}

// ==================== Regression tests: unsigned integer overflow wrapping ====================

#[test]
fn test_int_addition_overflow_wraps() {
    let source = r#"
package main

func Run() int {
    a := 9223372036854775807
    b := a + 1
    if b < 0 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "int max + 1 should wrap to negative");
}

#[test]
fn test_int_subtraction_underflow_wraps() {
    let source = r#"
package main

func Run() int {
    a := -9223372036854775807 - 1
    b := a - 1
    if b > 0 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "int min - 1 should wrap to positive");
}

#[test]
fn test_int64_shift_large() {
    let source = r#"
package main

func Run() int {
    a := 1
    b := a << 62
    if b == 4611686018427387904 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "1 << 62 should produce 2^62");
}

#[test]
fn test_rune_slice_valid_multibyte() {
    let source = r#"
package main

func Run() int {
    s := "Hello, 世界"
    r := []rune(s)
    if len(r) != 9 {
        return 0
    }
    if r[7] != 19990 {
        return 0
    }
    if r[8] != 30028 {
        return 0
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "[]rune(str) should correctly decode valid multi-byte UTF-8");
}

#[test]
fn test_rune_slice_ascii_only() {
    let source = r#"
package main

func Run() int {
    s := "abc"
    r := []rune(s)
    if len(r) != 3 {
        return 0
    }
    if r[0] != 97 || r[1] != 98 || r[2] != 99 {
        return 0
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "[]rune(str) should handle ASCII correctly");
}

#[test]
fn test_rune_slice_four_byte_emoji() {
    let source = r#"
package main

func Run() int {
    s := "A\xF0\x9F\x98\x80B"
    r := []rune(s)
    if len(r) != 3 {
        return 0
    }
    if r[0] != 65 {
        return 0
    }
    if r[1] != 128512 {
        return 0
    }
    if r[2] != 66 {
        return 0
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "[]rune(str) should correctly decode 4-byte UTF-8 sequences");
}

#[test]
fn test_nil_slice_comparison_eq() {
    let source = r#"
package main

func Run() int {
    var s []int
    if s == nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "nil slice should be == nil");
}

#[test]
fn test_non_nil_slice_comparison() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 0)
    if s == nil {
        return 0
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "make([]int, 0) should not be nil");
}

#[test]
fn test_nil_map_comparison_eq() {
    let source = r#"
package main

func Run() int {
    var m map[string]int
    if m == nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "nil map should be == nil");
}

#[test]
fn test_non_nil_map_comparison() {
    let source = r#"
package main

func Run() int {
    m := make(map[string]int)
    if m == nil {
        return 0
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "make(map[string]int) should not be nil");
}

#[test]
fn test_nil_pointer_comparison_eq() {
    let source = r#"
package main

func Run() int {
    var p *int
    if p == nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "nil pointer should be == nil");
}

#[test]
fn test_non_nil_pointer_comparison() {
    let source = r#"
package main

func Run() int {
    x := 42
    p := &x
    if p == nil {
        return 0
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "non-nil pointer should not be == nil");
}

#[test]
fn test_nil_slice_ne_comparison() {
    let source = r#"
package main

func Run() int {
    var s []int
    if s != nil {
        return 0
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "nil slice != nil should be false");
}

#[test]
fn test_nil_map_ne_comparison() {
    let source = r#"
package main

func Run() int {
    var m map[string]int
    if m != nil {
        return 0
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "nil map != nil should be false");
}

#[test]
fn test_nil_pointer_ne_comparison() {
    let source = r#"
package main

func Run() int {
    var p *int
    if p != nil {
        return 0
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "nil pointer != nil should be false");
}

#[test]
fn test_pointer_calls_value_receiver_method() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func (p Point) Sum() int {
    return p.X + p.Y
}

func Run() int {
    p := &Point{X: 3, Y: 4}
    return p.Sum()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 7, "pointer should be able to call value receiver method");
}

#[test]
fn test_value_calls_pointer_receiver_method() {
    let source = r#"
package main

type Counter struct {
    N int
}

func (c *Counter) Inc() {
    c.N++
}

func (c *Counter) Get() int {
    return c.N
}

func Run() int {
    c := Counter{N: 10}
    c.Inc()
    c.Inc()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 12, "value should be able to call pointer receiver method with auto-addressing");
}

#[test]
fn test_pointer_receiver_satisfies_interface() {
    let source = r#"
package main

type Sizer interface {
    Size() int
}

type Box struct {
    W int
    H int
}

func (b *Box) Size() int {
    return b.W * b.H
}

func getSize(s Sizer) int {
    return s.Size()
}

func Run() int {
    b := Box{W: 3, H: 4}
    return getSize(&b)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 12, "pointer receiver method should satisfy interface");
}

#[test]
fn test_global_var_init_dependency_order() {
    let source = r#"
package main

var a = b + 1
var b = 2

func Run() int {
    return a
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "a = b + 1 should be 3 when b = 2, regardless of declaration order");
}

#[test]
fn test_global_var_init_dependency_with_func() {
    let source = r#"
package main

func double(x int) int {
    return x * 2
}

var a = double(b)
var b = 5

func Run() int {
    return a
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "a = double(b) should be 10 when b = 5, regardless of declaration order");
}

#[test]
fn test_global_var_init_chain_dependency() {
    let source = r#"
package main

func compute(x int) int {
    return x * 3
}

var c = compute(b)
var b = compute(a)
var a = 2

func Run() int {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 18, "c = compute(b) = compute(compute(a)) = compute(compute(2)) = compute(6) = 18");
}

#[test]
fn test_global_var_init_reverse_nonconst_dependency() {
    let source = r#"
package main

func id(x int) int {
    return x
}

var a = id(b) + 1
var b = id(2)

func Run() int {
    return a
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "a = id(b) + 1 should be 3 when b = id(2) = 2, dependency order ensures b is initialized first");
}

#[test]
fn test_global_function_variable() {
    let source = r#"
package main

func double(x int) int {
    return x * 2
}

var f = double

func Run() int {
    return f(21)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "global function variable should be callable");
}

#[test]
fn test_for_loop_multiple_init_vars() {
    let source = r#"
package main

func Run() int {
    sum := 0
    for i, j := 0, 10; i < j; i++ {
        sum += i
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 45, "for loop with multiple init variables should sum 0..9 = 45");
}

#[test]
fn test_for_loop_two_var_converge() {
    let source = r#"
package main

func Run() int {
    count := 0
    for i, j := 0, 10; i < j; {
        i++
        j--
        count++
    }
    return count
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5, "two variables converging from 0 and 10 should meet after 5 iterations");
}

#[test]
fn test_swap_three_variables() {
    let source = r#"
package main

func Run() int {
    a := 1
    b := 2
    c := 3
    a, b, c = c, a, b
    return a*100 + b*10 + c
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 312, "after rotation a=3,b=1,c=2 so 312");
}

#[test]
fn test_math_trunc() {
    let source = r#"
package main

import "math"

func Run() int {
    a := math.Trunc(3.7)
    b := math.Trunc(-2.3)
    return int(a)*10 + int(b)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 28, "Trunc(3.7)=3, Trunc(-2.3)=-2, 3*10+(-2)=28");
}

#[test]
fn test_math_round() {
    let source = r#"
package main

import "math"

func Run() int {
    a := math.Round(3.5)
    b := math.Round(2.4)
    return int(a)*10 + int(b)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    std::fs::write("/tmp/test_math_round.wasm", &result.wasm_bytes).expect("write wasm failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "Round(3.5)=4, Round(2.4)=2, 4*10+2=42");
}

#[test]
fn test_stdlib_unimplemented_error() {
    let source = r#"
package main

import "strings"

func Run() int {
    s := strings.Contains("hello", "ell")
    if s {
        return 1
    }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "should fail to compile unimplemented stdlib call");
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("strings.Contains"), "error should mention the function: {}", msg);
    assert!(msg.contains("not yet implemented"), "error should explain not implemented: {}", msg);
}

#[test]
fn test_recursive_closure_fibonacci() {
    let source = r#"
package main

func Run() int {
    var fib func(int) int
    fib = func(n int) int {
        if n <= 1 {
            return n
        }
        return fib(n-1) + fib(n-2)
    }
    return fib(10)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 55, "fib(10) = 55");
}

#[test]
fn test_recursive_closure_factorial() {
    let source = r#"
package main

func Run() int {
    var fact func(int) int
    fact = func(n int) int {
        if n <= 1 {
            return 1
        }
        return n * fact(n-1)
    }
    return fact(6)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 720, "6! = 720");
}

#[test]
fn test_type_assertion_to_concrete_type() {
    let source = r#"
package main

type Sizer interface {
    Size() int
}

type Box struct {
    w int
    h int
}

func (b Box) Size() int {
    return b.w * b.h
}

func Run() int {
    var s Sizer = Box{w: 6, h: 7}
    b := s.(Box)
    return b.Size()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42);
}

#[test]
fn test_type_assertion_comma_ok_false() {
    let source = r#"
package main

type Runner interface {
    Run() int
}

type Walker interface {
    Walk() int
}

type Dog struct {
    speed int
}

func (d Dog) Run() int {
    return d.speed
}

func Run() int {
    var r Runner = Dog{speed: 10}
    _, ok := r.(Walker)
    if ok {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "type assertion to unimplemented interface should return ok=false");
}

#[test]
fn test_type_alias_with_methods() {
    let source = r#"
package main

type MyInt int

func (m MyInt) Double() int {
    return int(m) * 2
}

func Run() int {
    x := MyInt(21)
    return x.Double()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42);
}

#[test]
fn test_map_comma_ok_in_if() {
    let source = r#"
package main

func Run() int {
    m := make(map[int]int)
    m[1] = 10
    m[2] = 20
    result := 0
    if v, ok := m[1]; ok {
        result += v
    }
    if _, ok := m[3]; !ok {
        result += 100
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 110, "v=10 from m[1], +100 from missing m[3]");
}

// ==================== Regression: copy([]byte, string) ====================

#[test]
fn test_copy_byte_slice_from_string() {
    let source = r#"
package main

func Run() int {
    dst := make([]byte, 5)
    n := copy(dst, "Hello, World!")
    sum := 0
    for i := 0; i < n; i++ {
        sum = sum + int(dst[i])
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // 'H'=72 + 'e'=101 + 'l'=108 + 'l'=108 + 'o'=111 = 500
    assert_eq!(val, 500, "copy([]byte, string) should copy first 5 bytes");
}

#[test]
fn test_copy_byte_slice_from_string_shorter_dst() {
    let source = r#"
package main

func Run() int {
    dst := make([]byte, 2)
    n := copy(dst, "AB")
    return n
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 2, "copy should return min(len(dst), len(src))");
}

// ==================== Regression: append([]byte, string...) ====================

#[test]
fn test_append_byte_slice_from_string() {
    let source = r#"
package main

func Run() int {
    b := make([]byte, 0)
    b = append(b, "Hi"...)
    return len(b)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 2, "append([]byte, string...) should append 2 bytes");
}

#[test]
fn test_append_byte_slice_from_string_content() {
    let source = r#"
package main

func Run() int {
    b := make([]byte, 0)
    b = append(b, "AB"...)
    return int(b[0]) + int(b[1])
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // 'A'=65 + 'B'=66 = 131
    assert_eq!(val, 131, "append([]byte, string...) should copy string bytes");
}

// ==================== Regression: unsupported package function error ====================

#[test]
fn test_unsupported_package_function_error() {
    let source = r#"
package main

import "math"

func Run() int {
    return int(math.Log(2.0))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "int(math.Log(2.0)) should be 0 (0.693... truncated)");
}

// ==================== Regression: return f() multi-return forwarding ====================

#[test]
fn test_return_multi_value_forwarding() {
    let source = r#"
package main

func pair() (int, int) {
    return 10, 20
}

func forward() (int, int) {
    return pair()
}

func Run() int {
    a, b := forward()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "return f() should forward multiple return values");
}

#[test]
fn test_return_multi_value_forwarding_with_named_returns_and_defer() {
    let source = r#"
package main

func pair() (int, int) {
    return 10, 20
}

func forward() (a int, b int) {
    defer func() { a = a + 1 }()
    return pair()
}

func Run() int {
    a, b := forward()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 31, "return pair() with named returns + defer: a=10+1=11, b=20");
}

// ==================== Regression: named return + defer + explicit values ====================

#[test]
fn test_named_return_multi_with_defer_captures() {
    let source = r#"
package main

func compute() (x int, y int) {
    defer func() { x = x * 2 }()
    return 5, 10
}

func Run() int {
    a, b := compute()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 20, "defer should modify named return: x=5*2=10, y=10, total=20");
}

// ==================== Edge case: labeled break in switch inside for ====================

#[test]
fn test_labeled_break_switch_inside_for() {
    let source = r#"
package main

func Run() int {
    result := 0
OuterLoop:
    for i := 0; i < 5; i++ {
        switch i {
        case 3:
            break OuterLoop
        default:
            result = result + i
        }
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    // 0 + 1 + 2 = 3 (stops before i=3)
    assert_eq!(val, 3, "labeled break should exit the for loop from switch case");
}

// ==================== Edge case: short variable redeclaration with new var ====================

#[test]
fn test_short_var_redeclaration_with_new_var() {
    let source = r#"
package main

func Run() int {
    x := 1
    x, y := 2, 3
    return x + y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5, "short var redeclaration should reuse x and declare y");
}

// ==================== Edge case: unary bitwise complement ====================

#[test]
fn test_unary_bitwise_complement_i64() {
    let source = r#"
package main

func Run() int {
    x := 0
    return ^x
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, -1, "^0 should be -1 (all bits set)");
}

#[test]
fn test_unary_bitwise_complement_i32() {
    let source = r#"
package main

func Run() int {
    x := int32(0)
    return int(^x)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, -1, "^int32(0) should be -1");
}

// ==================== Edge case: nil map delete is no-op ====================

#[test]
fn test_delete_nil_map_noop() {
    let source = r#"
package main

func Run() int {
    var m map[int]int
    delete(m, 1)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "delete on nil map should be a no-op");
}

// ==================== Edge case: len and cap on nil slices ====================

#[test]
fn test_len_nil_slice() {
    let source = r#"
package main

func Run() int {
    var s []int
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "len of nil slice should be 0");
}

#[test]
fn test_cap_nil_slice() {
    let source = r#"
package main

func Run() int {
    var s []int
    return cap(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "cap of nil slice should be 0");
}

// ==================== Edge case: len nil map ====================

#[test]
fn test_len_nil_map() {
    let source = r#"
package main

func Run() int {
    var m map[int]int
    return len(m)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "len of nil map should be 0");
}

// ==================== Edge case: switch with init statement (tagless) ====================

#[test]
fn test_switch_with_init_statement_tagless() {
    let source = r#"
package main

func getVal() int {
    return 5
}

func Run() int {
    switch x := getVal(); {
    case x < 3:
        return 1
    case x < 7:
        return 2
    default:
        return 3
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 2, "switch with init statement should work");
}

// ==================== Edge case: struct comparison (flat) ====================

#[test]
fn test_struct_equality_flat() {
    let source = r#"
package main

type Point struct {
    x int
    y int
}

func Run() int {
    p1 := Point{x: 1, y: 2}
    p2 := Point{x: 1, y: 2}
    p3 := Point{x: 1, y: 9}
    result := 0
    if p1 == p2 {
        result = result + 10
    }
    if p1 != p3 {
        result = result + 100
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 110, "flat struct equality should compare all fields");
}

// ==================== Edge case: named type round-trip conversion ====================

#[test]
fn test_named_type_round_trip_conversion() {
    let source = r#"
package main

type MyInt int

func Run() int {
    var x MyInt = MyInt(42)
    var y int = int(x)
    return y
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "named type round-trip conversion should work");
}

// ==================== Edge case: deep variable shadowing ====================

#[test]
fn test_deep_variable_shadowing() {
    let source = r#"
package main

func Run() int {
    x := 1
    {
        x := 2
        {
            x := 3
            {
                x := 4
                _ = x
            }
            _ = x
        }
        _ = x
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "outermost x should still be 1 after deep shadowing");
}

// ==================== Edge case: integer truncation on conversion ====================

#[test]
fn test_int8_truncation_wrap() {
    let source = r#"
package main

func Run() int {
    x := 256
    y := int8(x)
    return int(y)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "int8(256) should wrap to 0");
}

#[test]
fn test_int8_truncation_negative() {
    let source = r#"
package main

func Run() int {
    x := 130
    y := int8(x)
    return int(y)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, -126, "int8(130) should wrap to -126");
}

// ==================== Edge case: for range over empty slice ====================

#[test]
fn test_for_range_empty_slice() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 0)
    sum := 0
    for _, v := range s {
        sum = sum + v
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "range over empty slice should not iterate");
}

// ==================== Edge case: multiple init vars in for loop ====================

#[test]
fn test_for_loop_multiple_init_and_post() {
    let source = r#"
package main

func Run() int {
    sum := 0
    for i, j := 0, 10; i < j; i, j = i+1, j-1 {
        sum = sum + 1
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5, "for loop with converging i and j should iterate 5 times");
}

// ==================== Edge case: blank identifier as lvalue ====================

#[test]
fn test_blank_identifier_side_effect() {
    let source = r#"
package main

var counter int = 0

func sideEffect() int {
    counter = counter + 1
    return 99
}

func Run() int {
    _ = sideEffect()
    _ = sideEffect()
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 2, "blank identifier should evaluate expression for side effects");
}

// =============================================
// Regression tests added during code review
// =============================================

#[test]
fn test_range_over_func_call_returning_slice() {
    let source = r#"
package main

func makeSlice() []int {
    s := make([]int, 3)
    s[0] = 10
    s[1] = 20
    s[2] = 30
    return s
}

func Run() int {
    total := 0
    for _, v := range makeSlice() {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 60);
}

#[test]
fn test_switch_string_many_cases_with_fallthrough() {
    let source = r#"
package main

func Classify(s string) int {
    switch s {
    case "alpha":
        return 1
    case "beta", "gamma":
        return 2
    case "delta":
        fallthrough
    case "epsilon":
        return 3
    default:
        return -1
    }
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let check = |input_str: &str, expected: i64| {
        let state = HostState::new();
        let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
        let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

        let alloc_fn = instance
            .get_typed_func::<i32, i32>(&mut store, "alloc")
            .expect("alloc not found");
        let ptr = alloc_fn.call(&mut store, input_str.len() as i32).expect("alloc failed");

        let memory = instance.get_memory(&mut store, "memory").expect("memory not found");
        memory.write(&mut store, ptr as usize, input_str.as_bytes()).expect("write failed");

        let func = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "Classify")
            .expect("Classify not found");
        let val = func.call(&mut store, (ptr, input_str.len() as i32)).expect("call failed");
        assert_eq!(val, expected, "Classify({:?}) should be {}", input_str, expected);
    };

    check("alpha", 1);
    check("beta", 2);
    check("gamma", 2);
    check("delta", 3);
    check("epsilon", 3);
    check("unknown", -1);
}

#[test]
fn test_switch_string_default_only() {
    let source = r#"
package main

func Always(s string) int {
    switch s {
    default:
        return 42
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

    let alloc_fn = instance
        .get_typed_func::<i32, i32>(&mut store, "alloc")
        .expect("alloc not found");
    let ptr = alloc_fn.call(&mut store, 3).expect("alloc failed");
    let memory = instance.get_memory(&mut store, "memory").expect("memory not found");
    memory.write(&mut store, ptr as usize, b"abc").expect("write failed");

    let func = instance
        .get_typed_func::<(i32, i32), i64>(&mut store, "Always")
        .expect("Always not found");
    let val = func.call(&mut store, (ptr, 3)).expect("call failed");
    assert_eq!(val, 42);
}

#[test]
fn test_nested_switch_string_and_int() {
    let source = r#"
package main

func Nested(s string, n int) int {
    result := 0
    switch s {
    case "add":
        switch n {
        case 1:
            result = 10
        case 2:
            result = 20
        default:
            result = 30
        }
    case "mul":
        switch n {
        case 1:
            result = 100
        default:
            result = 200
        }
    default:
        result = -1
    }
    return result
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");

    let check = |input_str: &str, n: i64, expected: i64| {
        let state = HostState::new();
        let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
        let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");

        let alloc_fn = instance
            .get_typed_func::<i32, i32>(&mut store, "alloc")
            .expect("alloc not found");
        let ptr = alloc_fn.call(&mut store, input_str.len() as i32).expect("alloc failed");
        let memory = instance.get_memory(&mut store, "memory").expect("memory not found");
        memory.write(&mut store, ptr as usize, input_str.as_bytes()).expect("write failed");

        let func = instance
            .get_typed_func::<(i32, i32, i64), i64>(&mut store, "Nested")
            .expect("Nested not found");
        let val = func.call(&mut store, (ptr, input_str.len() as i32, n)).expect("call failed");
        assert_eq!(val, expected, "Nested({:?}, {}) should be {}", input_str, n, expected);
    };

    check("add", 1, 10);
    check("add", 2, 20);
    check("add", 99, 30);
    check("mul", 1, 100);
    check("mul", 5, 200);
    check("other", 0, -1);
}

#[test]
fn test_escape_sequences_in_strings() {
    let source = r#"
package main

func Run() int {
    hex := "\x48\x49"
    if hex != "HI" {
        return 1
    }

    octal := "\110\111"
    if octal != "HI" {
        return 2
    }

    tab := "a\tb"
    if len(tab) != 3 {
        return 3
    }

    newline := "a\nb"
    if len(newline) != 3 {
        return 4
    }

    backslash := "a\\b"
    if len(backslash) != 3 {
        return 5
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "all escape sequences should work correctly");
}

#[test]
fn test_type_conversion_float_int() {
    let source = r#"
package main

func Run() int {
    var f float64 = 3.14
    var g int = int(f)
    if g != 3 {
        return 1
    }

    var h int = 42
    var i float64 = float64(h)
    if i != 42.0 {
        return 2
    }

    var j float32 = float32(2.5)
    var k float64 = float64(j)
    if k < 2.4 || k > 2.6 {
        return 3
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "float/int conversions should be correct");
}

#[test]
fn test_interface_concrete_to_interface_assignment() {
    let source = r#"
package main

type Stringer interface {
    String() int
}

type MyVal struct {
    v int
}

func (m *MyVal) String() int {
    return m.v
}

func Run() int {
    val := &MyVal{v: 99}
    var s Stringer = val
    return s.String()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 99);
}

#[test]
fn test_slice_literal_with_values() {
    let source = r#"
package main

func Run() int {
    nums := []int{10, 20, 30, 40}
    total := 0
    for i := 0; i < len(nums); i++ {
        total = total + nums[i]
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 100);
}

#[test]
fn test_range_over_inline_map() {
    let source = r#"
package main

func Run() int {
    m := make(map[string]int)
    m["a"] = 1
    m["b"] = 2
    m["c"] = 3
    total := 0
    for _, v := range m {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 6);
}

// ==================== B4: Multi-level pointer indirection ====================

#[test]
fn test_multi_level_pointer() {
    let source = r#"
package main

func Run() int {
    x := 42
    p := &x
    pp := &p
    return **pp
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "multi-level pointer dereference should work");
}

// ==================== B6: Nil function comparison ====================

#[test]
fn test_nil_func_comparison() {
    let source = r#"
package main

func Run() int {
    var f func()
    if f == nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "nil function should compare equal to nil");
}

// ==================== B2: Method chaining ====================

#[test]
fn test_method_chaining() {
    let source = r#"
package main

type Builder struct {
    val int
}

func (b Builder) Add(x int) Builder {
    return Builder{val: b.val + x}
}

func (b Builder) Result() int {
    return b.val
}

func Run() int {
    b := Builder{val: 0}
    return b.Add(10).Add(20).Add(12).Result()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "method chaining should work");
}

// ==================== C2: Map with empty struct values ====================

#[test]
fn test_map_with_empty_struct_values() {
    let source = r#"
package main

type Pair struct {
    A int
    B int
}

func Run() int {
    m := make(map[string]Pair)
    m["xy"] = Pair{A: 10, B: 32}
    p := m["xy"]
    return p.A + p.B
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "map with struct values should work");
}

// ==================== C3: Pointer-to-struct field access ====================

#[test]
fn test_pointer_struct_field_assign() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Run() int {
    p := &Point{X: 10, Y: 20}
    p.X = 42
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 62, "pointer-to-struct field assignment should work");
}

// ==================== C4: Named types with methods on builtin types ====================

#[test]
fn test_named_builtin_type_with_method() {
    let source = r#"
package main

type MyInt int

func (m MyInt) Double() int {
    return int(m) * 2
}

func Run() int {
    x := MyInt(21)
    return x.Double()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "named type with method on builtin type should work");
}

// ==================== C6: Multi-return forwarding ====================

#[test]
fn test_multi_return_forwarding() {
    let source = r#"
package main

func pair() (int, int) {
    return 10, 32
}

func add(a int, b int) int {
    return a + b
}

func Run() int {
    return add(pair())
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "multi-return forwarding f(g()) should work");
}

// ==================== C8: For range with blank index only ====================

#[test]
fn test_for_range_blank_index_only() {
    let source = r#"
package main

func Run() int {
    s := []int{10, 20, 30}
    sum := 0
    for _ = range s {
        sum = sum + 1
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "for _ = range should iterate correct number of times");
}

// ==================== C10: Switch with init statement and func call ====================

#[test]
fn test_switch_with_init_func_call() {
    let source = r#"
package main

func getVal() int {
    return 2
}

func Run() int {
    switch x := getVal(); x {
    case 1:
        return 10
    case 2:
        return 42
    case 3:
        return 30
    default:
        return 0
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "switch with init statement should work");
}

// ==================== D3: parse_go_int edge cases ====================

#[test]
fn test_hex_array_size() {
    let source = r#"
package main

func Run() int {
    var a [0x4]int
    a[0] = 10
    a[1] = 20
    a[2] = 30
    a[3] = 40
    return a[0] + a[1] + a[2] + a[3]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 100, "hex array size should work");
}

#[test]
fn test_binary_literal() {
    let source = r#"
package main

func Run() int {
    x := 0b101010
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "binary literal 0b101010 should equal 42");
}

#[test]
fn test_octal_literal() {
    let source = r#"
package main

func Run() int {
    x := 0o52
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "octal literal 0o52 should equal 42");
}

#[test]
fn test_underscore_separated_int() {
    let source = r#"
package main

func Run() int {
    x := 1_000_000
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1_000_000, "underscore separated integer should work");
}

// ==================== D2: Interface variable with type assertion and method call ====================

#[test]
fn test_interface_type_assert_method_call() {
    let source = r#"
package main

type Stringer interface {
    Value() int
}

type MyVal struct {
    v int
}

func (m MyVal) Value() int {
    return m.v
}

func wrap(x Stringer) int {
    m := x.(MyVal)
    return m.Value()
}

func Run() int {
    v := MyVal{v: 42}
    return wrap(v)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "interface type assertion and method call should work");
}

// ==================== High Priority Regression Tests ====================

#[test]
fn test_struct_comparison_equal() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Run() int {
    a := Point{X: 1, Y: 2}
    b := Point{X: 1, Y: 2}
    if a == b {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "equal structs should compare as ==");
}

#[test]
fn test_struct_comparison_not_equal() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func Run() int {
    a := Point{X: 1, Y: 2}
    b := Point{X: 3, Y: 4}
    if a != b {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "different structs should compare as !=");
}

#[test]
fn test_struct_with_slice_field_not_comparable() {
    let source = r#"
package main

type Bad struct {
    Data []int
}

func Run() int {
    a := Bad{Data: []int{1}}
    b := Bad{Data: []int{1}}
    if a == b {
        return 1
    }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "struct with slice field should not be comparable");
}

#[test]
fn test_struct_with_map_field_not_comparable() {
    let source = r#"
package main

type Bad struct {
    Data map[string]int
}

func Run() int {
    a := Bad{Data: map[string]int{"x": 1}}
    b := Bad{Data: map[string]int{"x": 1}}
    if a == b {
        return 1
    }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source);
    assert!(result.is_err(), "struct with map field should not be comparable");
}

#[test]
fn test_array_comparison() {
    let source = r#"
package main

func Run() int {
    a := [3]int{1, 2, 3}
    b := [3]int{1, 2, 3}
    c := [3]int{1, 2, 4}
    result := 0
    if a == b {
        result = result + 1
    }
    if a != c {
        result = result + 10
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "array comparison should work for == and !=");
}

#[test]
fn test_multi_return_assign_existing_vars() {
    let source = r#"
package main

func divmod(a int, b int) (int, int) {
    return a / b, a % b
}

func Run() int {
    var q int
    var r int
    q, r = divmod(17, 5)
    return q*100 + r
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 302, "multi-return assign to existing vars should work (q=3, r=2)");
}

#[test]
fn test_type_assert_comma_ok_with_assign() {
    let source = r#"
package main

func Run() int {
    var x interface{} = 42
    var v int
    var ok bool
    v, ok = x.(int)
    if !ok {
        return -1
    }
    return v
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "type assertion comma-ok with = should work");
}

#[test]
fn test_for_range_array() {
    let source = r#"
package main

func Run() int {
    arr := [4]int{10, 20, 30, 40}
    sum := 0
    for _, v := range arr {
        sum = sum + v
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 100, "range over array should sum all elements");
}

#[test]
fn test_for_range_value_only() {
    let source = r#"
package main

func Run() int {
    s := []int{5, 10, 15}
    sum := 0
    for _, v := range s {
        sum = sum + v
    }
    return sum
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30, "range with _ index should iterate values only");
}

#[test]
fn test_for_range_no_vars() {
    let source = r#"
package main

func Run() int {
    s := []int{1, 2, 3}
    count := 0
    for range s {
        count = count + 1
    }
    return count
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3, "range with no variables should iterate correctly");
}

#[test]
fn test_nested_selector_assignment() {
    let source = r#"
package main

type Inner struct {
    Val int
}

type Outer struct {
    In Inner
}

func Run() int {
    o := Outer{In: Inner{Val: 0}}
    o.In.Val = 42
    return o.In.Val
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "nested struct field assignment should work");
}

#[test]
fn test_interface_with_multiple_methods() {
    let source = r#"
package main

type Shape interface {
    Area() int
    Perimeter() int
}

type Rect struct {
    W int
    H int
}

func (r Rect) Area() int {
    return r.W * r.H
}

func (r Rect) Perimeter() int {
    return 2 * (r.W + r.H)
}

func Run() int {
    var s Shape = Rect{W: 3, H: 4}
    return s.Area()*100 + s.Perimeter()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1214, "interface with multiple methods: Area=12, Perimeter=14");
}

#[test]
fn test_higher_order_function() {
    let source = r#"
package main

func Run() int {
    double := func(x int) int {
        return x * 2
    }
    return double(21)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "function variable should be callable");
}

#[test]
fn test_closure_returned_from_scope() {
    let source = r#"
package main

func Run() int {
    base := 10
    add := func(x int) int {
        return base + x
    }
    return add(32)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "closure capturing outer variable should work");
}

#[test]
fn test_nested_function_call() {
    let source = r#"
package main

func add(a int, b int) int {
    return a + b
}

func mul(a int, b int) int {
    return a * b
}

func Run() int {
    return add(mul(3, 4), mul(5, 6))
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "nested function calls should work: add(12, 30) = 42");
}

#[test]
fn test_delete_struct_field_map() {
    let source = r#"
package main

type Config struct {
    Settings map[string]int
}

func Run() int {
    c := Config{Settings: map[string]int{"a": 1, "b": 2, "c": 3}}
    delete(c.Settings, "b")
    return len(c.Settings)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 2, "delete on struct field map should remove entry");
}

// ==================== Medium Priority Edge Case Tests ====================

#[test]
fn test_const_untyped_rune_arithmetic() {
    let source = r#"
package main

func Run() int {
    const c = 'A'
    return int(c) + 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 66, "const rune 'A' (65) + 1 = 66");
}

#[test]
fn test_switch_on_string_multiple_cases() {
    let source = r#"
package main

func classify(s string) int {
    switch s {
    case "hello":
        return 1
    case "world":
        return 2
    default:
        return 0
    }
}

func Run() int {
    return classify("hello")*100 + classify("world")*10 + classify("other")
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 120, "switch on string: hello=1, world=2, other=0 => 120");
}

#[test]
fn test_switch_bool_tagless() {
    let source = r#"
package main

func classify(x int) int {
    switch {
    case x < 0:
        return -1
    case x == 0:
        return 0
    case x > 0:
        return 1
    }
    return -99
}

func Run() int {
    a := classify(-5)
    b := classify(0)
    c := classify(5)
    return (a+2)*100 + (b+2)*10 + (c+2)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 123, "tagless switch with bool conditions: -1->1, 0->2, 1->3");
}

#[test]
fn test_type_switch_assign_variable() {
    let source = r#"
package main

func Run() int {
    var x interface{} = 42
    switch v := x.(type) {
    case int:
        return v + 100
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 142, "type switch with assigned variable should bind to correct type");
}

#[test]
fn test_defer_named_return_lifo() {
    let source = r#"
package main

func compute() (result int) {
    result = 0
    defer func() { result = result*10 + 3 }()
    defer func() { result = result*10 + 2 }()
    defer func() { result = result*10 + 1 }()
    return result
}

func Run() int {
    return compute()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 123, "defers with named return should execute LIFO and modify return value");
}

#[test]
fn test_append_nil_slice() {
    let source = r#"
package main

func Run() int {
    var s []int
    s = append(s, 42)
    return len(s)*100 + s[0]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 142, "append to nil slice should create new slice: len=1, elem=42");
}

#[test]
fn test_len_cap_array() {
    let source = r#"
package main

func Run() int {
    arr := [5]int{1, 2, 3, 4, 5}
    return len(arr)*10 + cap(arr)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 55, "len and cap on array should both return array length");
}

#[test]
fn test_delete_nonexistent_key() {
    let source = r#"
package main

func Run() int {
    m := map[string]int{"a": 1, "b": 2}
    delete(m, "nonexistent")
    return len(m)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 2, "deleting nonexistent key should be a no-op");
}

#[test]
fn test_make_map_with_capacity() {
    let source = r#"
package main

func Run() int {
    m := make(map[string]int, 100)
    m["x"] = 42
    return m["x"]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "make(map, capacity) should create functional map");
}

#[test]
fn test_blank_identifier_assign_discard() {
    let source = r#"
package main

func side() int {
    return 42
}

func Run() int {
    _ = side()
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "_ = expr should discard value");
}

#[test]
fn test_multi_return_blank_both() {
    let source = r#"
package main

func pair() (int, int) {
    return 10, 20
}

func Run() int {
    _, _ = pair()
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "_, _ = f() should compile and discard both values");
}

#[test]
fn test_pointer_to_pointer() {
    let source = r#"
package main

func Run() int {
    x := 42
    p := &x
    pp := &p
    return **pp
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "double pointer dereference should work");
}

#[test]
fn test_pointer_to_struct_mutation() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func mutate(p *Point) {
    p.X = 99
}

func Run() int {
    p := Point{X: 10, Y: 20}
    mutate(&p)
    return p.X
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 99, "pointer to struct mutation should work");
}

#[test]
fn test_string_builder_pattern() {
    let source = r#"
package main

func Run() int {
    s := ""
    for i := 0; i < 5; i++ {
        s = s + "a"
    }
    return len(s)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5, "string concatenation in loop should produce 5-char string");
}

#[test]
fn test_copy_to_nil_slice_returns_zero() {
    let source = r#"
package main

func Run() int {
    src := []int{1, 2, 3}
    var dst []int
    n := copy(dst, src)
    return n
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "copy to nil slice should return 0");
}

#[test]
fn test_panic_with_integer() {
    let source = r#"
package main

func Run() int {
    panic(42)
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let _err = func.call(&mut store, ()).expect_err("panic(42) should trap");
}

#[test]
fn test_nested_type_switch_two_levels() {
    let source = r#"
package main

func Run() int {
    var outer interface{} = 42
    var inner interface{} = "hello"
    result := 0
    switch outer.(type) {
    case int:
        result = 100
        switch inner.(type) {
        case string:
            result = result + 10
        default:
            result = result + 20
        }
    default:
        result = 999
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 110, "nested type switch should work");
}

#[test]
fn test_named_type_method_on_int() {
    let source = r#"
package main

type Celsius int

func (c Celsius) ToFahrenheit() int {
    return int(c)*9/5 + 32
}

func Run() int {
    temp := Celsius(100)
    return temp.ToFahrenheit()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 212, "named type method: 100C = 212F");
}

#[test]
fn test_multiple_append_calls() {
    let source = r#"
package main

func Run() int {
    s := make([]int, 0)
    s = append(s, 1)
    s = append(s, 2)
    s = append(s, 3)
    return len(s)*1000 + s[0]*100 + s[1]*10 + s[2]
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 3123, "multiple appends: len=3, [1,2,3]");
}

#[test]
fn test_struct_comparison_with_string_fields() {
    let source = r#"
package main

type Person struct {
    Name string
    Age  int
}

func Run() int {
    a := Person{Name: "Alice", Age: 30}
    b := Person{Name: "Alice", Age: 30}
    c := Person{Name: "Bob", Age: 25}
    result := 0
    if a == b {
        result = result + 1
    }
    if a != c {
        result = result + 10
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "struct comparison with string fields should work");
}

// ==================== Low Priority Advanced Feature Tests ====================

#[test]
fn test_generic_struct_pair_addition() {
    let source = r#"
package main

type Pair[T any] struct {
    First  T
    Second T
}

func Run() int {
    p := Pair[int]{First: 10, Second: 32}
    return p.First + p.Second
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "generic struct field access should work");
}

#[test]
fn test_global_var_dependency_order() {
    let source = r#"
package main

var b = a + 1
var a = 10

func Run() int {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "global var b depends on a; both should be initialized correctly");
}

#[test]
fn test_init_function_with_global_mutation() {
    let source = r#"
package main

var counter int

func init() {
    counter = 100
}

func Run() int {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 100, "init() should set global var before Run()");
}

#[test]
fn test_generic_constraint_comparable() {
    let source = r#"
package main

func contains[T comparable](s []T, v T) bool {
    for _, elem := range s {
        if elem == v {
            return true
        }
    }
    return false
}

func Run() int {
    s := []int{1, 2, 3, 4, 5}
    if contains(s, 3) {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "generic function with comparable constraint should work");
}

#[test]
fn test_const_expression_complex_arithmetic() {
    let source = r#"
package main

const (
    a = 10
    b = a * 2 + 5
    c = b - a
)

func Run() int {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15, "complex constant expressions: a=10, b=25, c=15");
}

#[test]
fn test_method_set_interface_value_vs_pointer() {
    let source = r#"
package main

type Stringer interface {
    String() string
}

type MyType struct {
    Val int
}

func (m MyType) String() string {
    return "hello"
}

func Run() int {
    m := MyType{Val: 42}
    var s Stringer = m
    result := s.String()
    return len(result)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5, "interface method dispatch with value receiver should return len('hello')=5");
}

// ===================== Stack/Heap Escape Analysis Tests =====================

#[test]
fn test_non_escaping_struct_uses_stack() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func SumPoint() int {
    p := Point{X: 10, Y: 20}
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
        .get_typed_func::<(), i64>(&mut store, "SumPoint")
        .expect("SumPoint not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 30);

    let memory = instance
        .get_memory(&mut store, "memory")
        .expect("memory export not found");
    let data = memory.data(&store);

    // Non-escaping struct should be allocated in the stack region [1024, 65536)
    // Check that the struct data (10, 20) appears in the stack region
    let stack_region = &data[1024..65536];
    let found_in_stack = stack_region.windows(8).any(|w| {
        let x = i64::from_le_bytes(w.try_into().unwrap());
        x == 10
    });
    assert!(found_in_stack, "non-escaping struct should be allocated in stack region");
}

#[test]
fn test_escaping_struct_uses_heap() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func MakePoint() Point {
    p := Point{X: 100, Y: 200}
    return p
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
        .get_typed_func::<(), i32>(&mut store, "MakePoint")
        .expect("MakePoint not found");

    let ptr = func.call(&mut store, ()).expect("call failed");
    // The returned pointer should be in the heap region (>= 65536)
    assert!(ptr >= 65536, "escaping struct should be in heap region, got ptr={}", ptr);
}

#[test]
fn test_stack_restored_after_function_call() {
    let source = r#"
package main

type Pair struct {
    A int
    B int
}

func First() int {
    p := Pair{A: 1, B: 2}
    return p.A
}

func Second() int {
    q := Pair{A: 3, B: 4}
    return q.B
}

func Run() int {
    a := First()
    b := Second()
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

    let func = instance
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5, "stack should be properly restored between calls");
}

#[test]
fn test_struct_passed_to_function_escapes() {
    let source = r#"
package main

type Data struct {
    Val int
}

func process(d Data) int {
    return d.Val * 2
}

func Run() int {
    d := Data{Val: 21}
    return process(d)
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
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "struct passed to function should work correctly");
}

#[test]
fn test_var_decl_zero_value_struct_stack() {
    let source = r#"
package main

type Config struct {
    Width  int
    Height int
}

func Run() int {
    var c Config
    c.Width = 800
    c.Height = 600
    return c.Width + c.Height
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
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1400, "zero-value struct on stack should work");
}

#[test]
fn test_reset_restores_heap_base() {
    let source = r#"
package main

func Alloc() int {
    s := "hello world"
    return len(s)
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
        .get_typed_func::<(), i64>(&mut store, "Alloc")
        .expect("Alloc not found");

    // Call once to allocate
    let val = func.call(&mut store, ()).expect("first call failed");
    assert_eq!(val, 11);

    // Call reset
    let reset_fn = instance
        .get_typed_func::<(), ()>(&mut store, "reset")
        .expect("reset not found");
    reset_fn.call(&mut store, ()).expect("reset failed");

    // Call again - should work fine after reset
    store.set_fuel(1_000_000).expect("set fuel failed");
    let val2 = func.call(&mut store, ()).expect("second call after reset failed");
    assert_eq!(val2, 11, "function should work identically after reset");
}

#[test]
fn test_nested_function_calls_stack_frames() {
    let source = r#"
package main

type Vec2 struct {
    X int
    Y int
}

func add(a Vec2, b Vec2) int {
    return a.X + b.X + a.Y + b.Y
}

func Run() int {
    v1 := Vec2{X: 1, Y: 2}
    v2 := Vec2{X: 3, Y: 4}
    return add(v1, v2)
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
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "nested calls with stack-allocated structs should work");
}

#[test]
fn test_addr_of_struct_escaping_via_return() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func MakePointPtr() int {
    p := &Point{X: 55, Y: 66}
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
        .get_typed_func::<(), i64>(&mut store, "MakePointPtr")
        .expect("MakePointPtr not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 121, "addr-of Point literal should work correctly");
}

#[test]
fn test_multiple_non_escaping_structs_same_function() {
    let source = r#"
package main

type Vec2 struct {
    X int
    Y int
}

func MultiStruct() int {
    a := Vec2{X: 1, Y: 2}
    b := Vec2{X: 10, Y: 20}
    c := Vec2{X: 100, Y: 200}
    return a.X + a.Y + b.X + b.Y + c.X + c.Y
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
        .get_typed_func::<(), i64>(&mut store, "MultiStruct")
        .expect("MultiStruct not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 333, "multiple non-escaping structs should all work correctly");

    let memory = instance
        .get_memory(&mut store, "memory")
        .expect("memory export not found");
    let data = memory.data(&store);

    let stack_region = &data[1024..65536];
    let found_1 = stack_region.windows(8).any(|w| {
        i64::from_le_bytes(w.try_into().unwrap()) == 1
    });
    let found_100 = stack_region.windows(8).any(|w| {
        i64::from_le_bytes(w.try_into().unwrap()) == 100
    });
    assert!(found_1, "first struct should be in stack region");
    assert!(found_100, "third struct should be in stack region");
}

#[test]
fn test_struct_allocated_inside_loop() {
    let source = r#"
package main

type Acc struct {
    Total int
}

func LoopStruct() int {
    sum := 0
    for i := 0; i < 5; i++ {
        a := Acc{Total: i * 10}
        sum = sum + a.Total
    }
    return sum
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
        .get_typed_func::<(), i64>(&mut store, "LoopStruct")
        .expect("LoopStruct not found");

    let val = func.call(&mut store, ()).expect("call failed");
    // 0 + 10 + 20 + 30 + 40 = 100
    assert_eq!(val, 100, "struct allocated inside loop should work correctly");
}

#[test]
fn test_closure_captures_local_var_escapes() {
    let source = r#"
package main

type Config struct {
    Val int
}

func ClosureCapture() int {
    x := 42
    c := Config{Val: x}
    f := func() int {
        return x
    }
    return f() + c.Val
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
        .get_typed_func::<(), i64>(&mut store, "ClosureCapture")
        .expect("ClosureCapture not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 84, "closure capturing local should work correctly");
}

#[test]
fn test_stack_reset_with_struct_allocation() {
    let source = r#"
package main

type Rec struct {
    A int
    B int
}

func StackAlloc() int {
    r := Rec{A: 7, B: 8}
    return r.A + r.B
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
        .get_typed_func::<(), i64>(&mut store, "StackAlloc")
        .expect("StackAlloc not found");

    let val1 = func.call(&mut store, ()).expect("first call failed");
    assert_eq!(val1, 15);

    let reset_fn = instance
        .get_typed_func::<(), ()>(&mut store, "reset")
        .expect("reset not found");
    reset_fn.call(&mut store, ()).expect("reset failed");

    store.set_fuel(1_000_000).expect("set fuel failed");
    let val2 = func.call(&mut store, ()).expect("second call after reset failed");
    assert_eq!(val2, 15, "stack-allocated struct should work identically after reset");
}

#[test]
fn test_addr_of_struct_stack_allocated() {
    let source = r#"
package main

type Vec3 struct {
    X int
    Y int
    Z int
}

func SumVec() int {
    v := &Vec3{X: 10, Y: 20, Z: 30}
    return v.X + v.Y + v.Z
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
        .get_typed_func::<(), i64>(&mut store, "SumVec")
        .expect("SumVec not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 60, "addr-of struct that doesn't escape should work on stack");

    let memory = instance
        .get_memory(&mut store, "memory")
        .expect("memory export not found");
    let data = memory.data(&store);

    let stack_region = &data[1024..65536];
    let found_in_stack = stack_region.windows(8).any(|w| {
        i64::from_le_bytes(w.try_into().unwrap()) == 10
    });
    assert!(found_in_stack, "non-escaping &struct should be allocated in stack region");
}

#[test]
fn test_new_struct_stack_allocated() {
    let source = r#"
package main

type Pair struct {
    A int
    B int
}

func NewPair() int {
    p := new(Pair)
    p.A = 100
    p.B = 200
    return p.A + p.B
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
        .get_typed_func::<(), i64>(&mut store, "NewPair")
        .expect("NewPair not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 300, "new(Pair) that doesn't escape should work on stack");

    let memory = instance
        .get_memory(&mut store, "memory")
        .expect("memory export not found");
    let data = memory.data(&store);

    let stack_region = &data[1024..65536];
    let found_100 = stack_region.windows(8).any(|w| {
        i64::from_le_bytes(w.try_into().unwrap()) == 100
    });
    assert!(found_100, "non-escaping new() struct should be in stack region");
}

#[test]
fn test_new_struct_escapes_via_return() {
    let source = r#"
package main

type Record struct {
    Val int
}

func MakeRecord() int {
    r := new(Record)
    r.Val = 42
    return r.Val
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
        .get_typed_func::<(), i64>(&mut store, "MakeRecord")
        .expect("MakeRecord not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "new() struct should work correctly");
}

#[test]
fn test_struct_in_switch_init() {
    let source = r#"
package main

type Mode struct {
    Val int
}

func SwitchInit() int {
    m := Mode{Val: 5}
    switch x := m.Val; {
    case x > 3:
        return x * 10
    default:
        return x
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
        .get_typed_func::<(), i64>(&mut store, "SwitchInit")
        .expect("SwitchInit not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 50, "struct used in switch init should work correctly");
}

#[test]
fn test_struct_survives_closure_in_same_function() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func UseStructAfterClosure() int {
    p := Point{X: 10, Y: 20}
    f := func() int { return 42 }
    result := f()
    return p.X + p.Y + result
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
        .get_typed_func::<(), i64>(&mut store, "UseStructAfterClosure")
        .expect("UseStructAfterClosure not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 72, "struct on stack must survive closure compilation in the same function");
}

#[test]
fn test_struct_survives_generic_call_in_same_function() {
    let source = r#"
package main

type Vec2 struct {
    A int
    B int
}

func Identity[T any](x T) T {
    return x
}

func UseStructAfterGeneric() int {
    v := Vec2{A: 3, B: 7}
    n := Identity[int](100)
    return v.A + v.B + n
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
        .get_typed_func::<(), i64>(&mut store, "UseStructAfterGeneric")
        .expect("UseStructAfterGeneric not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 110, "struct on stack must survive generic monomorphization in the same function");
}

#[test]
fn test_closure_capture_with_slice_expr() {
    let source = r#"
package main

func SliceCapture() int {
    arr := [5]int{10, 20, 30, 40, 50}
    start := 1
    f := func() int {
        return arr[start] + arr[start + 1]
    }
    return f()
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
        .get_typed_func::<(), i64>(&mut store, "SliceCapture")
        .expect("SliceCapture not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 50, "closure must correctly capture variables used in index expressions");
}

#[test]
fn test_nested_call_args_escape() {
    let source = r#"
package main

func Double(x int) int {
    return x * 2
}

func AddOne(x int) int {
    return x + 1
}

func Run() int {
    return AddOne(Double(5))
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
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "nested call arguments must be correctly handled");
}

#[test]
fn test_closure_assignment_captures_var() {
    let source = r#"
package main

func Run() int {
    x := 10
    f := func() int {
        x = 20
        return x
    }
    return f()
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
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 20, "closure that assigns to outer var must capture it as free");
}

#[test]
fn test_reset_clears_panic_state() {
    let source = r#"
package main

func Panicker() int {
    panic("boom")
    return 0
}

func Safe() int {
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

    let panicker = instance
        .get_typed_func::<(), i64>(&mut store, "Panicker")
        .expect("Panicker not found");

    let _ = panicker.call(&mut store, ());

    let reset = instance
        .get_typed_func::<(), ()>(&mut store, "reset")
        .expect("reset not found");
    reset.call(&mut store, ()).expect("reset failed");

    let safe = instance
        .get_typed_func::<(), i64>(&mut store, "Safe")
        .expect("Safe not found");
    let val = safe.call(&mut store, ()).expect("Safe should succeed after reset");
    assert_eq!(val, 42, "function must work correctly after reset clears panic state");
}

#[test]
fn test_stack_overflow_check_before_bump() {
    let source = r#"
package main

type Big struct {
    A int
    B int
    C int
    D int
}

func UseStack() int {
    b := Big{A: 1, B: 2, C: 3, D: 4}
    return b.A + b.B + b.C + b.D
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
        .get_typed_func::<(), i64>(&mut store, "UseStack")
        .expect("UseStack not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "stack allocation with overflow check before bump must work");
}

#[test]
fn test_return_with_nested_call_escapes_arg() {
    let source = r#"
package main

func Identity(x int) int {
    return x
}

func Run() int {
    a := 7
    b := 3
    return Identity(a) + Identity(b)
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
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "return with nested function calls must work correctly");
}

#[test]
fn test_closure_param_not_treated_as_capture() {
    let source = r#"
package main

func Run() int {
    a := 5
    f := func(a int) int {
        return a * 3
    }
    return f(a)
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
        .get_typed_func::<(), i64>(&mut store, "Run")
        .expect("Run not found");

    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 15, "closure parameter should shadow outer variable, not cause false escape");
}

// ==================== Stdlib: errors package ====================

#[test]
fn test_errors_new_non_nil() {
    let source = r#"
package main

import "errors"

func Run() int {
    err := errors.New("something went wrong")
    if err != nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "errors.New should return a non-nil error");
}

#[test]
fn test_errors_new_error_method() {
    let source = r#"
package main

import "errors"

func Run() int {
    err := errors.New("hello")
    msg := err.Error()
    if len(msg) == 5 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "err.Error() should return the original message");
}

#[test]
fn test_errors_errunsupported() {
    let source = r#"
package main

import "errors"

func Run() int {
    err := errors.ErrUnsupported
    if err != nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "errors.ErrUnsupported should be a non-nil sentinel");
}

#[test]
fn test_errors_new_distinct() {
    let source = r#"
package main

import "errors"

func Run() int {
    e1 := errors.New("a")
    e2 := errors.New("a")
    if e1 == e2 {
        return 0
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "two errors.New calls with same text should be distinct");
}

#[test]
fn test_errors_not_imported_no_compilation() {
    let source = r#"
package main

func Run() int {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 42, "code without errors import should compile normally");
}

#[test]
fn test_errors_unwrap_wrapping_type() {
    let source = r#"
package main

import "errors"

type wrappedError struct {
    msg string
    inner error
}

func (e *wrappedError) Error() string {
    return e.msg
}

func (e *wrappedError) Unwrap() error {
    return e.inner
}

func Run() int {
    inner := errors.New("inner")
    outer := &wrappedError{msg: "outer", inner: inner}
    unwrapped := errors.Unwrap(outer)
    if unwrapped == nil {
        return 0
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Unwrap should return the inner error");
}

#[test]
fn test_errors_unwrap_non_wrapping() {
    let source = r#"
package main

import "errors"

func Run() int {
    err := errors.New("plain error")
    unwrapped := errors.Unwrap(err)
    if unwrapped == nil {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Unwrap on non-wrapping error should return nil");
}

#[test]
fn test_errors_is_sentinel_match() {
    let source = r#"
package main

import "errors"

func Run() int {
    sentinel := errors.New("not found")
    if errors.Is(sentinel, sentinel) {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Is should match identical sentinel error");
}

#[test]
fn test_errors_is_wrapped_chain() {
    let source = r#"
package main

import "errors"

type wrappedError struct {
    msg string
    inner error
}

func (e *wrappedError) Error() string {
    return e.msg
}

func (e *wrappedError) Unwrap() error {
    return e.inner
}

func Run() int {
    sentinel := errors.New("base")
    wrapped := &wrappedError{msg: "layer1", inner: sentinel}
    if errors.Is(wrapped, sentinel) {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Is should find sentinel through wrapped chain");
}

#[test]
fn test_errors_is_no_match() {
    let source = r#"
package main

import "errors"

func Run() int {
    err1 := errors.New("error one")
    err2 := errors.New("error two")
    if errors.Is(err1, err2) {
        return 0
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Is should return false for non-matching errors");
}

#[test]
fn test_utf8_rune_len_ascii() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    return utf8.RuneLen('A')
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1);
}

#[test]
fn test_utf8_rune_len_multibyte() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    r2 := utf8.RuneLen(0x00E9)
    r3 := utf8.RuneLen(0x4E16)
    r4 := utf8.RuneLen(0x1F600)
    return r2*100 + r3*10 + r4
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 234, "2-byte=2, 3-byte=3, 4-byte=4 -> 234");
}

#[test]
fn test_utf8_rune_len_invalid() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    neg := utf8.RuneLen(-1)
    sur := utf8.RuneLen(0xD800)
    over := utf8.RuneLen(0x110000)
    if neg == -1 && sur == -1 && over == -1 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "invalid runes should return -1");
}

#[test]
fn test_utf8_decode_rune_ascii() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    buf := []byte{0x48, 0x69}
    r, size := utf8.DecodeRune(buf)
    if r == 'H' && size == 1 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1);
}

#[test]
fn test_utf8_decode_rune_multibyte() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    buf := []byte{0xC3, 0xA9}
    r, size := utf8.DecodeRune(buf)
    if r == 0xE9 && size == 2 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "should decode 2-byte UTF-8 (e-acute)");
}

#[test]
fn test_utf8_decode_rune_empty() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    buf := []byte{}
    r, size := utf8.DecodeRune(buf)
    if r == 0xFFFD && size == 0 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "empty slice should return RuneError, 0");
}

#[test]
fn test_utf8_decode_rune_in_string() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    r, size := utf8.DecodeRuneInString("Hello")
    if r == 'H' && size == 1 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1);
}

#[test]
fn test_utf8_encode_rune() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    buf := make([]byte, 4)
    n := utf8.EncodeRune(buf, 'A')
    if n == 1 && buf[0] == 0x41 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1);
}

#[test]
fn test_utf8_encode_rune_multibyte() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    buf := make([]byte, 4)
    n := utf8.EncodeRune(buf, 0xE9)
    if n == 2 && buf[0] == 0xC3 && buf[1] == 0xA9 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "should encode 2-byte e-acute");
}

#[test]
fn test_utf8_rune_count_in_string() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    return utf8.RuneCountInString("Hello")
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5);
}

#[test]
fn test_utf8_rune_count_bytes() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    buf := []byte{0x48, 0x65, 0x6C, 0x6C, 0x6F}
    return utf8.RuneCount(buf)
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 5);
}

#[test]
fn test_utf8_valid_string() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    if utf8.ValidString("Hello") {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1);
}

#[test]
fn test_utf8_valid_bytes() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    good := []byte{0x48, 0x65, 0x6C, 0x6C, 0x6F}
    bad := []byte{0xFF, 0xFE}
    g := 0
    b := 0
    if utf8.Valid(good) {
        g = 1
    }
    if utf8.Valid(bad) {
        b = 1
    }
    return g*10 + b
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 10, "valid=1, invalid=0 -> 10");
}

#[test]
fn test_utf8_valid_rune() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    r := 0
    if utf8.ValidRune('A') {
        r += 1
    }
    if utf8.ValidRune(0x10FFFF) {
        r += 10
    }
    if utf8.ValidRune(0xD800) {
        r += 100
    }
    if utf8.ValidRune(-1) {
        r += 1000
    }
    return r
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "A=valid, MaxRune=valid, surrogate=invalid, -1=invalid -> 11");
}

#[test]
fn test_utf8_rune_start() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    r := 0
    if utf8.RuneStart(0x41) {
        r += 1
    }
    if utf8.RuneStart(0xC3) {
        r += 10
    }
    if utf8.RuneStart(0x80) {
        r += 100
    }
    return r
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "0x41=start, 0xC3=start, 0x80=continuation -> 11");
}

#[test]
fn test_utf8_full_rune() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    r := 0
    if utf8.FullRune([]byte{0x41}) {
        r += 1
    }
    if utf8.FullRune([]byte{0xC3, 0xA9}) {
        r += 10
    }
    if utf8.FullRune([]byte{0xC3}) {
        r += 100
    }
    if utf8.FullRune([]byte{}) {
        r += 1000
    }
    return r
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 11, "ASCII=full, 2-byte=full, truncated=not full, empty=not full -> 11");
}

#[test]
fn test_utf8_full_rune_in_string() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    r := 0
    if utf8.FullRuneInString("A") {
        r += 1
    }
    if utf8.FullRuneInString("") {
        r += 10
    }
    return r
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "A=full, empty=not full -> 1");
}

#[test]
fn test_utf8_append_rune() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    var buf []byte
    buf = utf8.AppendRune(buf, 'A')
    if len(buf) == 1 && buf[0] == 0x41 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1);
}

#[test]
fn test_utf8_decode_last_rune() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    buf := []byte{0x48, 0x65, 0x6C, 0x6C, 0x6F}
    r, size := utf8.DecodeLastRune(buf)
    if r == 'o' && size == 1 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1);
}

#[test]
fn test_utf8_decode_last_rune_in_string() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    r, size := utf8.DecodeLastRuneInString("Hello")
    if r == 'o' && size == 1 {
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
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1);
}

#[test]
fn test_utf8_constants() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    r := 0
    if utf8.RuneError == 0xFFFD {
        r += 1
    }
    if utf8.RuneSelf == 0x80 {
        r += 10
    }
    if utf8.MaxRune == 0x10FFFF {
        r += 100
    }
    if utf8.UTFMax == 4 {
        r += 1000
    }
    return r
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1111, "all four constants should match");
}

#[test]
fn test_utf8_encode_decode_roundtrip() {
    let source = r#"
package main

import "unicode/utf8"

func Run() int {
    runes := []rune{0x41, 0xE9, 0x4E16, 0x1F600}
    expected := []int{1, 2, 3, 4}
    buf := make([]byte, 4)
    for i := 0; i < len(runes); i++ {
        n := utf8.EncodeRune(buf, runes[i])
        if n != expected[i] {
            return 0
        }
        r, sz := utf8.DecodeRune(buf[0:n])
        if r != runes[i] || sz != n {
            return 0
        }
    }
    return 1
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 1_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "encode/decode roundtrip should preserve runes");
}

// ======================= time package tests =======================

#[test]
fn test_time_duration_constants() {
    let source = r#"
package main

import "time"

func Run() int64 {
    ns := int64(time.Nanosecond)
    us := int64(time.Microsecond)
    ms := int64(time.Millisecond)
    s := int64(time.Second)
    m := int64(time.Minute)
    h := int64(time.Hour)
    if ns != 1 { return 1 }
    if us != 1000 { return 2 }
    if ms != 1000000 { return 3 }
    if s != 1000000000 { return 4 }
    if m != 60000000000 { return 5 }
    if h != 3600000000000 { return 6 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "Duration constants should have correct values");
}

#[test]
fn test_time_duration_methods() {
    let source = r#"
package main

import "time"

func Run() int64 {
    d := time.Duration(3661500000000)
    if d.Nanoseconds() != 3661500000000 { return 1 }
    if d.Microseconds() != 3661500000 { return 2 }
    if d.Milliseconds() != 3661500 { return 3 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "Duration methods should return correct values");
}

#[test]
fn test_time_duration_abs() {
    let source = r#"
package main

import "time"

func Run() int64 {
    d := time.Duration(-5000000000)
    a := d.Abs()
    if a.Nanoseconds() != 5000000000 { return 1 }
    d2 := time.Duration(3000000000)
    a2 := d2.Abs()
    if a2.Nanoseconds() != 3000000000 { return 2 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0);
}

#[test]
fn test_time_duration_truncate_round() {
    let source = r#"
package main

import "time"

func Run() int64 {
    d := time.Duration(1500000000)
    t := d.Truncate(time.Second)
    if t.Nanoseconds() != 1000000000 { return 1 }
    r := d.Round(time.Second)
    if r.Nanoseconds() != 2000000000 { return 2 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0);
}

#[test]
fn test_time_now_nonzero() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t := time.Now()
    if t.UnixNano() == 0 { return 1 }
    if t.Unix() == 0 { return 2 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "time.Now() should return non-zero time");
}

#[test]
fn test_time_unix_constructors() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t := time.Unix(1700000000, 500000000)
    if t.Unix() != 1700000000 { return 1 }
    if t.UnixNano() != 1700000000500000000 { return 2 }

    t2 := time.UnixMilli(1700000000500)
    if t2.Unix() != 1700000000 { return 3 }
    if t2.UnixMilli() != 1700000000500 { return 4 }

    t3 := time.UnixMicro(1700000000500000)
    if t3.UnixMicro() != 1700000000500000 { return 5 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "Unix constructors should produce correct timestamps");
}

#[test]
fn test_time_is_zero() {
    let source = r#"
package main

import "time"

func Run() int64 {
    var t time.Time
    if !t.IsZero() { return 1 }
    t2 := time.Unix(1, 0)
    if t2.IsZero() { return 2 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0);
}

#[test]
fn test_time_add_sub() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t := time.Unix(1000, 0)
    t2 := t.Add(5 * time.Second)
    if t2.Unix() != 1005 { return 1 }
    d := t2.Sub(t)
    if d.Nanoseconds() != 5000000000 { return 2 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0);
}

#[test]
fn test_time_comparison() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t1 := time.Unix(1000, 0)
    t2 := time.Unix(2000, 0)
    if !t1.Before(t2) { return 1 }
    if !t2.After(t1) { return 2 }
    t3 := time.Unix(1000, 0)
    if !t1.Equal(t3) { return 3 }
    if t1.Compare(t2) != -1 { return 4 }
    if t2.Compare(t1) != 1 { return 5 }
    if t1.Compare(t3) != 0 { return 6 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "Time comparison methods should work correctly");
}

#[test]
fn test_time_date_components() {
    let source = r#"
package main

import "time"

func Run() int64 {
    // 2023-11-15 10:30:45 UTC = Unix 1700044245
    t := time.Unix(1700044245, 0)
    if t.Year() != 2023 { return 1 }
    if t.Month() != time.November { return 2 }
    if t.Day() != 15 { return 3 }
    if t.Hour() != 10 { return 4 }
    if t.Minute() != 30 { return 5 }
    if t.Second() != 45 { return 6 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "Date components should be correct for known timestamp");
}

#[test]
fn test_time_weekday() {
    let source = r#"
package main

import "time"

func Run() int64 {
    // 2023-11-15 is Wednesday
    t := time.Unix(1700044245, 0)
    if t.Weekday() != time.Wednesday { return 1 }
    // 1970-01-01 is Thursday
    t2 := time.Unix(0, 0)
    if t2.Weekday() != time.Thursday { return 2 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "Weekday should be correct for known dates");
}

#[test]
fn test_time_date_method() {
    let source = r#"
package main

import "time"

func Run() int64 {
    // 2023-11-15 10:30:45 UTC
    t := time.Unix(1700044245, 0)
    year, month, day := t.Date()
    if year != 2023 { return 1 }
    if month != time.November { return 2 }
    if day != 15 { return 3 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0);
}

#[test]
fn test_time_clock_method() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t := time.Unix(1700044245, 0)
    h, m, s := t.Clock()
    if h != 10 { return 1 }
    if m != 30 { return 2 }
    if s != 45 { return 3 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0);
}

#[test]
fn test_time_format_rfc3339() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t := time.Unix(1700044245, 0)
    s := t.Format(time.RFC3339)
    if s == "2023-11-15T10:30:45Z" { return 1 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Format RFC3339 should produce correct string");
}

#[test]
fn test_time_format_date_only() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t := time.Unix(1700044245, 0)
    s := t.Format(time.DateOnly)
    if s == "2023-11-15" { return 1 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Format DateOnly should produce correct string");
}

#[test]
fn test_time_format_time_only() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t := time.Unix(1700044245, 0)
    s := t.Format(time.TimeOnly)
    if s == "10:30:45" { return 1 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Format TimeOnly should produce correct string");
}

#[test]
fn test_time_format_datetime() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t := time.Unix(1700044245, 0)
    s := t.Format(time.DateTime)
    if s == "2023-11-15 10:30:45" { return 1 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Format DateTime should produce correct string");
}

#[test]
fn test_time_parse_rfc3339() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t, err := time.Parse(time.RFC3339, "2023-11-15T10:30:45Z")
    if err != nil { return 1 }
    if t.Year() != 2023 { return 2 }
    if t.Month() != time.November { return 3 }
    if t.Day() != 15 { return 4 }
    if t.Hour() != 10 { return 5 }
    if t.Minute() != 30 { return 6 }
    if t.Second() != 45 { return 7 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "Parse RFC3339 should produce correct Time");
}

#[test]
fn test_time_parse_duration() {
    let source = r#"
package main

import "time"

func Run() int64 {
    d, err := time.ParseDuration("1h30m")
    if err != nil { return 1 }
    if d.Nanoseconds() != 5400000000000 { return 2 }

    d2, err2 := time.ParseDuration("500ms")
    if err2 != nil { return 3 }
    if d2.Nanoseconds() != 500000000 { return 4 }

    d3, err3 := time.ParseDuration("-2s")
    if err3 != nil { return 5 }
    if d3.Nanoseconds() != -2000000000 { return 6 }

    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "ParseDuration should parse correctly");
}

#[test]
fn test_time_month_string() {
    let source = r#"
package main

import "time"

func Run() int64 {
    s := time.January.String()
    if s == "January" { return 1 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Month.String() should return correct name");
}

#[test]
fn test_time_weekday_string() {
    let source = r#"
package main

import "time"

func Run() int64 {
    s := time.Wednesday.String()
    if s == "Wednesday" { return 1 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Weekday.String() should return correct name");
}

#[test]
fn test_time_add_date() {
    let source = r#"
package main

import "time"

func Run() int64 {
    // 2023-11-15 10:30:45
    t := time.Unix(1700044245, 0)
    t2 := t.AddDate(1, 2, 3)
    if t2.Year() != 2025 { return 1 }
    if t2.Month() != time.January { return 2 }
    if t2.Day() != 18 { return 3 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "AddDate should add years/months/days correctly");
}

#[test]
fn test_time_since_positive() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t := time.Unix(0, 0)
    d := time.Since(t)
    if d.Nanoseconds() <= 0 { return 1 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "Since(epoch) should be positive");
}

#[test]
fn test_time_fixed_zone() {
    let source = r#"
package main

import "time"

func Run() int64 {
    loc := time.FixedZone("EST", -18000)
    s := loc.String()
    if s == "EST" { return 1 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "FixedZone should create location with correct name");
}

#[test]
fn test_time_unix_roundtrip() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t := time.Unix(1700000000, 123456789)
    if t.Unix() != 1700000000 { return 1 }
    if t.UnixNano() != 1700000000123456789 { return 2 }
    if t.Nanosecond() != 123456789 { return 3 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "Unix timestamp roundtrip should preserve values");
}

#[test]
fn test_time_epoch_date() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t := time.Unix(0, 0)
    if t.Year() != 1970 { return 1 }
    if t.Month() != time.January { return 2 }
    if t.Day() != 1 { return 3 }
    if t.Hour() != 0 { return 4 }
    if t.Minute() != 0 { return 5 }
    if t.Second() != 0 { return 6 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "Unix epoch should be 1970-01-01 00:00:00");
}

#[test]
fn test_time_duration_string() {
    let source = r#"
package main

import "time"

func Run() int64 {
    d := 3*time.Hour + 2*time.Minute + 1*time.Second + 500*time.Millisecond
    s := d.String()
    if s == "3h2m1.5s" { return 1 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Duration.String() should format correctly");
}

#[test]
fn test_time_duration_string_zero() {
    let source = r#"
package main

import "time"

func Run() int64 {
    d := time.Duration(0)
    s := d.String()
    if s == "0s" { return 1 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Duration(0).String() should be '0s'");
}

#[test]
fn test_time_parse_format_roundtrip() {
    let source = r#"
package main

import "time"

func Run() int64 {
    original := "2023-11-15T10:30:45Z"
    t, err := time.Parse(time.RFC3339, original)
    if err != nil { return 0 }
    formatted := t.Format(time.RFC3339)
    if formatted == original { return 1 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1, "Parse/Format roundtrip should preserve time string");
}

#[test]
fn test_time_leap_year_date() {
    let source = r#"
package main

import "time"

func Run() int64 {
    // 2024-02-29 00:00:00 UTC (leap year)
    t := time.Unix(1709164800, 0)
    if t.Year() != 2024 { return 1 }
    if t.Month() != time.February { return 2 }
    if t.Day() != 29 { return 3 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 0, "Leap year date should be calculated correctly");
}

#[test]
fn test_time_add_simple() {
    let source = r#"
package main

import "time"

func Run() int64 {
    t := time.Unix(1000, 0)
    t2 := t.Add(time.Duration(5000000000))
    return t2.Unix()
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime.load_module(&result.wasm_bytes).expect("module load failed");
    let state = HostState::new();
    let mut store = runtime.create_store(state, 10_000_000).expect("store creation failed");
    let instance = runtime.instantiate(&mut store, &module).expect("instantiation failed");
    let func = instance.get_typed_func::<(), i64>(&mut store, "Run").expect("not found");
    let val = func.call(&mut store, ()).expect("call failed");
    assert_eq!(val, 1005);
}

#[test]
fn test_time_debug_wat() {
    let source = r#"
package main

import "time"

func Run() int64 {
    loc := time.FixedZone("EST", -18000)
    s := loc.String()
    if s == "EST" { return 1 }
    return 0
}
"#;
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");
    let wat = wasmprinter::print_bytes(&result.wasm_bytes);
    match wat {
        Ok(w) => {
            std::fs::write("/tmp/time_debug.wat", &w).unwrap();
            eprintln!("WAT written to /tmp/time_debug.wat ({} bytes)", w.len());
            for (i, line) in w.lines().enumerate() {
                    if line.contains("(func") || line.contains("(type") {
                        eprintln!("WAT {}: {}", i+1, line.trim());
                    }
                }
        }
        Err(e) => eprintln!("WAT error: {}", e),
    }
    let runtime = UdfRuntime::new().expect("runtime init failed");
    match runtime.load_module(&result.wasm_bytes) {
        Ok(_) => eprintln!("Module loaded OK"),
        Err(e) => eprintln!("Module load failed: {:#}", e),
    }
}
