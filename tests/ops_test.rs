use gno_rs::wasm::compiler::WasmCompiler;
use gno_rs::wasm::runtime::{HostState, UdfRuntime};

fn compile_and_instantiate(source: &str) -> (wasmtime::Store<HostState>, wasmtime::Instance) {
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime
        .load_module(&result.wasm_bytes)
        .expect("module load failed");

    let state = HostState::new();
    let mut store = runtime
        .create_store(state, 1_000_000)
        .expect("store creation failed");
    let instance = runtime
        .instantiate(&mut store, &module)
        .expect("instantiation failed");

    (store, instance)
}

// =============================================================================
// 1. Arithmetic — int (i64)
// =============================================================================

#[test]
fn test_int_add() {
    let src = "package main\nfunc F() int { return 3 + 4 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 7);
}

#[test]
fn test_int_sub() {
    let src = "package main\nfunc F() int { return 10 - 3 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 7);
}

#[test]
fn test_int_mul() {
    let src = "package main\nfunc F() int { return 6 * 7 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 42);
}

#[test]
fn test_int_div() {
    let src = "package main\nfunc F() int { return 20 / 4 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 5);
}

#[test]
fn test_int_mod() {
    let src = "package main\nfunc F() int { return 17 % 5 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 2);
}

#[test]
fn test_int_negative() {
    let src = "package main\nfunc F() int { return -42 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), -42);
}

// =============================================================================
// 1b. Arithmetic — int32
// =============================================================================

#[test]
fn test_int32_add() {
    let src = "package main\nfunc F() int32 { var a int32 = 10; var b int32 = 20; return a + b }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i32>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 30);
}

#[test]
fn test_int32_mul() {
    let src = "package main\nfunc F() int32 { var a int32 = 5; var b int32 = 6; return a * b }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i32>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 30);
}

// =============================================================================
// 1c. Arithmetic — float64
// =============================================================================

#[test]
fn test_float64_add() {
    let src = "package main\nfunc F() float64 { return 1.5 + 2.5 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), f64>(&mut s, "F").unwrap();
    assert!((f.call(&mut s, ()).unwrap() - 4.0).abs() < 1e-9);
}

#[test]
fn test_float64_sub() {
    let src = "package main\nfunc F() float64 { return 5.0 - 1.5 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), f64>(&mut s, "F").unwrap();
    assert!((f.call(&mut s, ()).unwrap() - 3.5).abs() < 1e-9);
}

#[test]
fn test_float64_mul() {
    let src = "package main\nfunc F() float64 { return 3.0 * 2.5 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), f64>(&mut s, "F").unwrap();
    assert!((f.call(&mut s, ()).unwrap() - 7.5).abs() < 1e-9);
}

#[test]
fn test_float64_div() {
    let src = "package main\nfunc F() float64 { return 10.0 / 4.0 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), f64>(&mut s, "F").unwrap();
    assert!((f.call(&mut s, ()).unwrap() - 2.5).abs() < 1e-9);
}

#[test]
fn test_mixed_int_float() {
    let src = r#"package main
func F() float64 {
    var a int = 3
    return float64(a) + 1.5
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), f64>(&mut s, "F").unwrap();
    assert!((f.call(&mut s, ()).unwrap() - 4.5).abs() < 1e-9);
}

// =============================================================================
// 2. Comparisons
// =============================================================================

#[test]
fn test_cmp_eq_true() {
    let src = "package main\nfunc F() int { if 5 == 5 { return 1 }; return 0 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_cmp_eq_false() {
    let src = "package main\nfunc F() int { if 5 == 6 { return 1 }; return 0 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0);
}

#[test]
fn test_cmp_ne() {
    let src = "package main\nfunc F() int { if 5 != 6 { return 1 }; return 0 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_cmp_lt() {
    let src = "package main\nfunc F() int { if 3 < 5 { return 1 }; return 0 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_cmp_gt() {
    let src = "package main\nfunc F() int { if 7 > 3 { return 1 }; return 0 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_cmp_le() {
    let src = "package main\nfunc F() int { if 5 <= 5 { return 1 }; return 0 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_cmp_ge() {
    let src = "package main\nfunc F() int { if 5 >= 6 { return 1 }; return 0 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0);
}

#[test]
fn test_cmp_float64() {
    let src = "package main\nfunc F() int { if 3.14 > 2.71 { return 1 }; return 0 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

// =============================================================================
// 3. Logical and Bitwise
// =============================================================================

#[test]
fn test_logical_and() {
    let src = r#"package main
func F() int {
    a := true
    b := false
    if a && b { return 1 }
    return 0
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0);
}

#[test]
fn test_logical_or() {
    let src = r#"package main
func F() int {
    a := true
    b := false
    if a || b { return 1 }
    return 0
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_logical_not() {
    let src = r#"package main
func F() int {
    a := false
    if !a { return 1 }
    return 0
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_bitwise_and() {
    let src = "package main\nfunc F() int { return 0xFF & 0x0F }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0x0F);
}

#[test]
fn test_bitwise_or() {
    let src = "package main\nfunc F() int { return 0xF0 | 0x0F }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0xFF);
}

#[test]
fn test_bitwise_xor() {
    let src = "package main\nfunc F() int { return 0xFF ^ 0x0F }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0xF0);
}

#[test]
fn test_shift_left() {
    let src = "package main\nfunc F() int { return 1 << 8 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 256);
}

#[test]
fn test_shift_right() {
    let src = "package main\nfunc F() int { return 256 >> 4 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 16);
}

// =============================================================================
// 4. Variables
// =============================================================================

#[test]
fn test_var_zero_int() {
    let src = "package main\nfunc F() int { var x int; return x }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0);
}

#[test]
fn test_var_zero_float64() {
    let src = "package main\nfunc F() float64 { var x float64; return x }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), f64>(&mut s, "F").unwrap();
    assert!((f.call(&mut s, ()).unwrap()).abs() < 1e-15);
}

#[test]
fn test_var_zero_bool() {
    let src = "package main\nfunc F() int { var x bool; if x { return 1 }; return 0 }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0);
}

#[test]
fn test_short_decl() {
    let src = "package main\nfunc F() int { x := 42; return x }";
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 42);
}

#[test]
fn test_multiple_assign() {
    let src = r#"package main
func F() int {
    a, b := 10, 20
    return a + b
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 30);
}

#[test]
fn test_swap() {
    let src = r#"package main
func F() int {
    a, b := 1, 2
    a, b = b, a
    return a*10 + b
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 21);
}

// =============================================================================
// 5. Control Flow
// =============================================================================

#[test]
fn test_if_true() {
    let src = r#"package main
func F() int {
    x := 5
    if x > 3 { return 1 }
    return 0
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_if_else() {
    let src = r#"package main
func F() int {
    x := 2
    if x > 3 {
        return 1
    } else {
        return 0
    }
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0);
}

#[test]
fn test_for_loop_sum() {
    let src = r#"package main
func F() int {
    sum := 0
    for i := 1; i <= 10; i++ {
        sum = sum + i
    }
    return sum
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 55);
}

#[test]
fn test_for_while_style() {
    let src = r#"package main
func F() int {
    n := 1
    for n < 100 {
        n = n * 2
    }
    return n
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 128);
}

#[test]
fn test_for_break() {
    let src = r#"package main
func F() int {
    sum := 0
    for i := 0; i < 100; i++ {
        if i == 5 { break }
        sum = sum + i
    }
    return sum
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 10);
}

#[test]
fn test_for_continue() {
    let src = r#"package main
func F() int {
    sum := 0
    for i := 0; i < 10; i++ {
        if i % 2 == 0 { continue }
        sum = sum + i
    }
    return sum
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 25);
}

#[test]
fn test_switch_case() {
    let src = r#"package main
func F() int {
    x := 2
    switch x {
    case 1:
        return 10
    case 2:
        return 20
    case 3:
        return 30
    }
    return -1
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 20);
}

#[test]
fn test_switch_default() {
    let src = r#"package main
func F() int {
    x := 99
    switch x {
    case 1:
        return 10
    default:
        return 42
    }
}"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 42);
}

// =============================================================================
// 6. Functions
// =============================================================================

#[test]
fn test_func_params_return() {
    let src = r#"package main
func add(a int, b int) int { return a + b }
func F() int { return add(3, 4) }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 7);
}

#[test]
fn test_func_multi_return() {
    let src = r#"package main
func divmod(a int, b int) (int, int) { return a / b, a % b }
func F() int {
    q, r := divmod(17, 5)
    return q*10 + r
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 32);
}

#[test]
fn test_func_recursive() {
    let src = r#"package main
func fact(n int) int {
    if n <= 1 { return 1 }
    return n * fact(n - 1)
}
func F() int { return fact(6) }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 720);
}

#[test]
fn test_func_closure() {
    let src = r#"package main
func F() int {
    x := 10
    add := func(y int) int { return x + y }
    return add(5)
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 15);
}

// =============================================================================
// 7. Strings
// =============================================================================

#[test]
fn test_string_len() {
    let src = r#"package main
func F() int { s := "hello"; return len(s) }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 5);
}

#[test]
fn test_string_index_byte() {
    let src = r#"package main
func F() int { s := "ABCDE"; return int(s[2]) }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 67); // 'C'
}

#[test]
fn test_string_concat() {
    let src = r#"package main
func F() int {
    a := "hel"
    b := "lo"
    c := a + b
    return len(c)
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 5);
}

#[test]
fn test_string_compare_eq() {
    let src = r#"package main
func F() int {
    a := "abc"
    b := "abc"
    if a == b { return 1 }
    return 0
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_string_compare_ne() {
    let src = r#"package main
func F() int {
    a := "abc"
    b := "xyz"
    if a != b { return 1 }
    return 0
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_string_slice() {
    let src = r#"package main
func F() int {
    s := "hello world"
    sub := s[0:5]
    return len(sub)
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 5);
}

// =============================================================================
// 8. Slices
// =============================================================================

#[test]
fn test_slice_literal_len() {
    let src = r#"package main
func F() int {
    s := []int{10, 20, 30}
    return len(s)
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 3);
}

#[test]
fn test_slice_index() {
    let src = r#"package main
func F() int {
    s := []int{10, 20, 30}
    return s[1]
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 20);
}

#[test]
fn test_slice_append() {
    let src = r#"package main
func F() int {
    s := []int{1, 2}
    s = append(s, 3)
    return len(s)
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 3);
}

#[test]
fn test_slice_append_value() {
    let src = r#"package main
func F() int {
    s := []int{1, 2}
    s = append(s, 3)
    return s[2]
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 3);
}

#[test]
fn test_slice_for_range_sum() {
    let src = r#"package main
func F() int {
    s := []int{1, 2, 3, 4, 5}
    sum := 0
    for _, v := range s {
        sum = sum + v
    }
    return sum
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 15);
}

#[test]
fn test_slice_cap() {
    let src = r#"package main
func F() int {
    s := make([]int, 3, 10)
    return cap(s)
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 10);
}

// =============================================================================
// 9. Structs
// =============================================================================

#[test]
fn test_struct_init_read() {
    let src = r#"package main
type Point struct { X int; Y int }
func F() int {
    p := Point{X: 3, Y: 7}
    return p.X + p.Y
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 10);
}

#[test]
fn test_struct_field_write() {
    let src = r#"package main
type Point struct { X int; Y int }
func F() int {
    p := Point{}
    p.X = 42
    return p.X
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 42);
}

#[test]
fn test_struct_method() {
    let src = r#"package main
type Rect struct { W int; H int }
func (r Rect) Area() int { return r.W * r.H }
func F() int {
    r := Rect{W: 4, H: 5}
    return r.Area()
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 20);
}

#[test]
fn test_struct_nested() {
    let src = r#"package main
type Inner struct { Val int }
type Outer struct { In Inner }
func F() int {
    o := Outer{In: Inner{Val: 99}}
    return o.In.Val
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 99);
}

#[test]
fn test_struct_pointer_receiver() {
    let src = r#"package main
type Counter struct { N int }
func (c *Counter) Inc() { c.N = c.N + 1 }
func F() int {
    c := Counter{N: 0}
    c.Inc()
    c.Inc()
    return c.N
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 2);
}

#[test]
fn test_struct_with_string_field() {
    let src = r#"package main
type Named struct { Name string; Age int }
func F() int {
    n := Named{Name: "alice", Age: 30}
    return n.Age
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 30);
}

#[test]
fn test_struct_string_field_len() {
    let src = r#"package main
type Named struct { Name string; Age int }
func F() int {
    n := Named{Name: "alice", Age: 30}
    return len(n.Name)
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 5);
}

// =============================================================================
// 10. Interfaces
// =============================================================================

#[test]
fn test_interface_basic() {
    let src = r#"package main
type Stringer interface { Len() int }
type MyStr struct { S string }
func (m MyStr) Len() int { return len(m.S) }
func F() int {
    var s Stringer = MyStr{S: "hello"}
    return s.Len()
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 5);
}

#[test]
fn test_interface_nil_check() {
    let src = r#"package main
type Doer interface { Do() int }
func F() int {
    var d Doer
    if d == nil { return 1 }
    return 0
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_interface_assign_and_call() {
    let src = r#"package main
type Adder interface { Add(int) int }
type Base struct { N int }
func (b Base) Add(x int) int { return b.N + x }
func F() int {
    var a Adder = Base{N: 10}
    return a.Add(5)
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 15);
}

// =============================================================================
// 11. Global Variables
// =============================================================================

#[test]
fn test_global_int() {
    let src = r#"package main
var x int = 42
func F() int { return x }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 42);
}

#[test]
fn test_global_int_computed() {
    let src = r#"package main
var x int = 3 + 4
func F() int { return x }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 7);
}

#[test]
fn test_global_string_len() {
    let src = r#"package main
var greeting string = "hello"
func F() int { return len(greeting) }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 5);
}

#[test]
fn test_global_bool() {
    let src = r#"package main
var flag bool = true
func F() int { if flag { return 1 }; return 0 }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_global_float64() {
    let src = r#"package main
var pi float64 = 3.14
func F() float64 { return pi }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), f64>(&mut s, "F").unwrap();
    assert!((f.call(&mut s, ()).unwrap() - 3.14).abs() < 1e-9);
}

#[test]
fn test_global_slice_int() {
    let src = r#"package main
var nums = []int{10, 20, 30}
func F() int { return nums[1] }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 20);
}

#[test]
fn test_global_slice_len() {
    let src = r#"package main
var nums = []int{10, 20, 30}
func F() int { return len(nums) }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 3);
}

#[test]
fn test_global_struct() {
    let src = r#"package main
type Pair struct { A int; B int }
var p = Pair{A: 5, B: 7}
func F() int { return p.A + p.B }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 12);
}

#[test]
fn test_global_struct_with_string() {
    let src = r#"package main
type Entry struct { Name string; Val int }
var e = Entry{Name: "test", Val: 42}
func F() int { return e.Val }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 42);
}

#[test]
fn test_global_struct_string_len() {
    let src = r#"package main
type Entry struct { Name string; Val int }
var e = Entry{Name: "test", Val: 42}
func F() int { return len(e.Name) }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 4);
}

#[test]
fn test_global_slice_of_structs() {
    let src = r#"package main
type Item struct { Delta int; Tag string }
var items = []Item{
    {Delta: 0, Tag: ""},
    {Delta: 1, Tag: "5"},
    {Delta: 1, Tag: "25"},
    {Delta: 1, Tag: "125"},
}
func F() int { return items[2].Delta }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_global_slice_of_structs_string_len() {
    let src = r#"package main
type Item struct { Delta int; Tag string }
var items = []Item{
    {Delta: 0, Tag: ""},
    {Delta: 1, Tag: "5"},
    {Delta: 1, Tag: "25"},
    {Delta: 1, Tag: "125"},
}
func F() int { return len(items[3].Tag) }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 3);
}

#[test]
fn test_global_slice_of_structs_count() {
    let src = r#"package main
type Item struct { Delta int; Tag string }
var items = []Item{
    {Delta: 0, Tag: ""},
    {Delta: 1, Tag: "5"},
    {Delta: 1, Tag: "25"},
    {Delta: 1, Tag: "125"},
}
func F() int { return len(items) }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 4);
}

// =============================================================================
// 12. Errors
// =============================================================================

#[test]
fn test_errors_new() {
    let src = r#"package main
import "errors"
var e = errors.New("boom")
func F() int {
    if e != nil { return 1 }
    return 0
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_error_nil_check() {
    let src = r#"package main
func mayFail(fail bool) (int, error) {
    if fail {
        return 0, nil
    }
    return 42, nil
}
func F() int {
    n, err := mayFail(false)
    if err != nil { return -1 }
    return n
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 42);
}

#[test]
fn test_error_return_and_check() {
    let src = r#"package main
import "errors"
func divide(a int, b int) (int, error) {
    if b == 0 {
        return 0, errors.New("div by zero")
    }
    return a / b, nil
}
func F() int {
    _, err := divide(10, 0)
    if err != nil { return -1 }
    return 0
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), -1);
}

#[test]
fn test_error_return_success() {
    let src = r#"package main
import "errors"
func divide(a int, b int) (int, error) {
    if b == 0 {
        return 0, errors.New("div by zero")
    }
    return a / b, nil
}
func F() int {
    n, err := divide(20, 4)
    if err != nil { return -1 }
    return n
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 5);
}

#[test]
fn test_global_interface_errors_new() {
    let src = r#"package main
import "errors"
var ErrNotFound = errors.New("not found")
var ErrTimeout = errors.New("timeout")
func F() int {
    if ErrNotFound != nil { return 1 }
    return 0
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_multiple_global_errors() {
    let src = r#"package main
import "errors"
var ErrA = errors.New("a")
var ErrB = errors.New("b")
var ErrC = errors.New("c")
func F() int {
    if ErrA == nil { return -1 }
    if ErrB == nil { return -2 }
    if ErrC == nil { return -3 }
    return 3
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 3);
}

// ============================================================
// Multi-return and math stdlib tests
// ============================================================

#[test]
fn test_multi_return_float64() {
    let src = r#"package main
func split(x float64) (float64, float64) {
    return x + 1.0, x + 2.0
}
func F() int {
    a, b := split(10.0)
    return int(a + b)
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 23);
}

#[test]
fn test_multi_return_discard() {
    let src = r#"package main
func split(x float64) (float64, float64) {
    return x + 1.0, x + 2.0
}
func F() int {
    a, _ := split(10.0)
    return int(a)
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 11);
}

#[test]
fn test_multi_return_discard_first() {
    let src = r#"package main
func split(x float64) (float64, float64) {
    return x + 1.0, x + 2.0
}
func F() int {
    _, b := split(10.0)
    return int(b)
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 12);
}

#[test]
fn test_math_abs() {
    let src = r#"package main
import "math"
func F() int { return int(math.Abs(-5.0)) }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 5);
}

#[test]
fn test_math_trunc() {
    let src = r#"package main
import "math"
func F() int { return int(math.Trunc(3.7)) }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 3);
}

#[test]
fn test_math_modf() {
    let src = r#"package main
import "math"
func F() int {
    i, f := math.Modf(3.7)
    return int(i) * 10 + int(f * 10)
}
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 37);
}

#[test]
fn test_math_floor() {
    let src = r#"package main
import "math"
func F() int { return int(math.Floor(3.7)) }
"#;
    let (mut s, i) = compile_and_instantiate(src);
    let f = i.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 3);
}
