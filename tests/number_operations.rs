use gno_rs::wasm::compiler::WasmCompiler;
use gno_rs::wasm::runtime::{HostState, UdfRuntime};

fn compile_and_instantiate(
    source: &str,
) -> (wasmtime::Store<HostState>, wasmtime::Instance) {
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
// 1. Integer Division and Remainder (Go Spec: truncated division)
//    x = q*y + r, |r| < |y|, truncated towards zero
// =============================================================================

#[test]
fn test_int_division_positive_positive() {
    let source = r#"
package main

func Div(x int64, y int64) int64 { return x / y }
func Rem(x int64, y int64) int64 { return x % y }
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let div = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Div")
        .unwrap();
    let rem = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Rem")
        .unwrap();
    assert_eq!(div.call(&mut store, (5, 3)).unwrap(), 1);
    assert_eq!(rem.call(&mut store, (5, 3)).unwrap(), 2);
}

#[test]
fn test_int_division_negative_dividend() {
    let source = r#"
package main

func Div(x int64, y int64) int64 { return x / y }
func Rem(x int64, y int64) int64 { return x % y }
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let div = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Div")
        .unwrap();
    let rem = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Rem")
        .unwrap();
    assert_eq!(div.call(&mut store, (-5, 3)).unwrap(), -1);
    assert_eq!(rem.call(&mut store, (-5, 3)).unwrap(), -2);
}

#[test]
fn test_int_division_negative_divisor() {
    let source = r#"
package main

func Div(x int64, y int64) int64 { return x / y }
func Rem(x int64, y int64) int64 { return x % y }
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let div = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Div")
        .unwrap();
    let rem = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Rem")
        .unwrap();
    assert_eq!(div.call(&mut store, (5, -3)).unwrap(), -1);
    assert_eq!(rem.call(&mut store, (5, -3)).unwrap(), 2);
}

#[test]
fn test_int_division_both_negative() {
    let source = r#"
package main

func Div(x int64, y int64) int64 { return x / y }
func Rem(x int64, y int64) int64 { return x % y }
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let div = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Div")
        .unwrap();
    let rem = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Rem")
        .unwrap();
    assert_eq!(div.call(&mut store, (-5, -3)).unwrap(), 1);
    assert_eq!(rem.call(&mut store, (-5, -3)).unwrap(), -2);
}

#[test]
fn test_int_division_power_of_two() {
    let source = r#"
package main

func Div(x int64, y int64) int64 { return x / y }
func Rem(x int64, y int64) int64 { return x % y }
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let div = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Div")
        .unwrap();
    let rem = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Rem")
        .unwrap();
    assert_eq!(div.call(&mut store, (11, 4)).unwrap(), 2);
    assert_eq!(rem.call(&mut store, (11, 4)).unwrap(), 3);
    assert_eq!(div.call(&mut store, (-11, 4)).unwrap(), -2);
    assert_eq!(rem.call(&mut store, (-11, 4)).unwrap(), -3);
}

#[test]
fn test_int_division_large_values() {
    let source = r#"
package main

func Div(x int64, y int64) int64 { return x / y }
func Rem(x int64, y int64) int64 { return x % y }
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let div = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Div")
        .unwrap();
    let rem = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "Rem")
        .unwrap();
    assert_eq!(div.call(&mut store, (i64::MAX, 2)).unwrap(), i64::MAX / 2);
    assert_eq!(rem.call(&mut store, (i64::MAX, 2)).unwrap(), 1);
}

#[test]
fn test_int_division_invariant() {
    let source = r#"
package main

func Check(x int64, y int64) bool {
    q := x / y
    r := x % y
    return x == q*y+r
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(i64, i64), i32>(&mut store, "Check")
        .unwrap();
    assert_eq!(f.call(&mut store, (17, 5)).unwrap(), 1);
    assert_eq!(f.call(&mut store, (-17, 5)).unwrap(), 1);
    assert_eq!(f.call(&mut store, (17, -5)).unwrap(), 1);
    assert_eq!(f.call(&mut store, (-17, -5)).unwrap(), 1);
    assert_eq!(f.call(&mut store, (100, 7)).unwrap(), 1);
    assert_eq!(f.call(&mut store, (-100, 7)).unwrap(), 1);
}

// =============================================================================
// 2. Integer Overflow (Go Spec: modular for unsigned, deterministic for signed)
// =============================================================================

#[test]
fn test_unsigned_addition_overflow() {
    let source = r#"
package main

func F() uint64 {
    var x uint64 = ^uint64(0)
    x = x + 1
    return x
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_unsigned_subtraction_underflow() {
    let source = r#"
package main

func F() uint64 {
    var x uint64 = 0
    x = x - 1
    return x
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), -1i64);
}

#[test]
fn test_unsigned_multiplication_overflow() {
    let source = r#"
package main

func F() uint64 {
    var x uint64 = ^uint64(0)
    return x * 2
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    let expected = (-1i64 as u64).wrapping_mul(2) as i64;
    assert_eq!(f.call(&mut store, ()).unwrap(), expected);
}

#[test]
fn test_signed_addition_overflow() {
    let source = r#"
package main

func F() int64 {
    var x int64 = 9223372036854775807
    x = x + 1
    return x
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), i64::MIN);
}

#[test]
fn test_signed_subtraction_overflow() {
    let source = r#"
package main

func F(x int64) int64 {
    return x - 1
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, i64::MIN).unwrap(), i64::MAX);
}

#[test]
fn test_unsigned_left_shift_overflow() {
    let source = r#"
package main

func F() uint64 {
    var x uint64 = 1
    return x << 63
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), i64::MIN);
}

// =============================================================================
// 3. Shift Operators (Go Spec: arithmetic for signed, logical for unsigned)
// =============================================================================

#[test]
fn test_signed_right_shift_negative() {
    let source = r#"
package main

func F(x int64) int64 {
    return x >> 1
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, -8).unwrap(), -4);
    assert_eq!(f.call(&mut store, -1).unwrap(), -1);
    assert_eq!(f.call(&mut store, -100).unwrap(), -50);
}

#[test]
fn test_unsigned_right_shift_high_bit() {
    let source = r#"
package main

func F() uint64 {
    var x uint64 = ^uint64(0)
    return x >> 1
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), i64::MAX);
}

#[test]
fn test_left_shift_various() {
    let source = r#"
package main

func F(x int64, n int) int64 {
    return x << n
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, (1, 0)).unwrap(), 1);
    assert_eq!(f.call(&mut store, (1, 1)).unwrap(), 2);
    assert_eq!(f.call(&mut store, (1, 10)).unwrap(), 1024);
    assert_eq!(f.call(&mut store, (3, 4)).unwrap(), 48);
}

#[test]
fn test_shift_multiplication_equivalence() {
    let source = r#"
package main

func ShiftLeft(x int64) int64  { return x << 1 }
func Double(x int64) int64     { return x * 2 }
func ShiftRight(x int64) int64 { return x >> 1 }
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let sl = instance
        .get_typed_func::<i64, i64>(&mut store, "ShiftLeft")
        .unwrap();
    let dbl = instance
        .get_typed_func::<i64, i64>(&mut store, "Double")
        .unwrap();
    let sr = instance
        .get_typed_func::<i64, i64>(&mut store, "ShiftRight")
        .unwrap();

    for &x in &[0i64, 1, -1, 42, -42, 1000, -1000] {
        assert_eq!(sl.call(&mut store, x).unwrap(), dbl.call(&mut store, x).unwrap());
    }
    assert_eq!(sr.call(&mut store, 7).unwrap(), 3);
    // right shift truncates towards negative infinity, not zero
    assert_eq!(sr.call(&mut store, -7).unwrap(), -4);
}

#[test]
fn test_shift_by_zero() {
    let source = r#"
package main

func F(x int64) int64 {
    return (x << 0) + (x >> 0)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, 42).unwrap(), 84);
    assert_eq!(f.call(&mut store, -10).unwrap(), -20);
}

// =============================================================================
// 4. Bitwise Unary Complement (Go Spec: ^x)
//    signed:   ^x = -1 ^ x = -(x+1)
//    unsigned: ^x = m ^ x where m = all bits 1
// =============================================================================

#[test]
fn test_bitwise_complement_signed_values() {
    let source = r#"
package main

func F(x int64) int64 {
    return ^x
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, 0).unwrap(), -1);
    assert_eq!(f.call(&mut store, 1).unwrap(), -2);
    assert_eq!(f.call(&mut store, -1).unwrap(), 0);
    assert_eq!(f.call(&mut store, 100).unwrap(), -101);
    assert_eq!(f.call(&mut store, i64::MAX).unwrap(), i64::MIN);
}

#[test]
fn test_bitwise_complement_unsigned_zero() {
    let source = r#"
package main

func F() uint64 {
    return ^uint64(0)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), -1i64);
}

#[test]
fn test_bitwise_complement_unsigned_nonzero() {
    let source = r#"
package main

func F(x uint64) uint64 {
    return ^x
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, 1).unwrap(), -2i64);
    assert_eq!(f.call(&mut store, -1i64).unwrap(), 0);
}

// =============================================================================
// 5. Type Conversion Edge Cases
// =============================================================================

#[test]
fn test_conversion_sign_extension_chain() {
    let source = r#"
package main

func F() int64 {
    v := uint16(0x10F0)
    return int64(uint32(int8(v)))
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0xFFFFFFF0u32 as i64);
}

#[test]
fn test_conversion_truncation_int8_from_256() {
    let source = r#"
package main

func F() int64 {
    return int64(int8(256))
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_conversion_truncation_int8_from_255() {
    let source = r#"
package main

func F() int64 {
    return int64(int8(255))
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), -1);
}

#[test]
fn test_conversion_truncation_uint8_from_256() {
    let source = r#"
package main

func F() int64 {
    return int64(uint8(256))
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_conversion_truncation_uint8_from_257() {
    let source = r#"
package main

func F() int64 {
    return int64(uint8(257))
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}

#[test]
fn test_conversion_float_to_int_truncates_positive() {
    let source = r#"
package main

func F(x float64) int64 {
    return int64(x)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<f64, i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, 2.9).unwrap(), 2);
    assert_eq!(f.call(&mut store, 2.1).unwrap(), 2);
    assert_eq!(f.call(&mut store, 0.999).unwrap(), 0);
}

#[test]
fn test_conversion_float_to_int_truncates_negative() {
    let source = r#"
package main

func F(x float64) int64 {
    return int64(x)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<f64, i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, -2.9).unwrap(), -2);
    assert_eq!(f.call(&mut store, -2.1).unwrap(), -2);
    assert_eq!(f.call(&mut store, -0.999).unwrap(), 0);
}

#[test]
fn test_conversion_int_to_float_roundtrip() {
    let source = r#"
package main

func F(x int64) int64 {
    return int64(float64(x))
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<i64, i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, 42).unwrap(), 42);
    assert_eq!(f.call(&mut store, -42).unwrap(), -42);
    assert_eq!(f.call(&mut store, 0).unwrap(), 0);
}

#[test]
fn test_conversion_widening_chain() {
    let source = r#"
package main

func F() int64 {
    return int64(int32(int16(int8(-5))))
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), -5);
}

#[test]
fn test_conversion_narrowing_chain() {
    let source = r#"
package main

func F() int64 {
    var a int64 = 0x01020304
    var b int32 = int32(a)
    var c int16 = int16(b)
    var d int8 = int8(c)
    return int64(d)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 4);
}

#[test]
fn test_conversion_uint16_masking() {
    let source = r#"
package main

func F() int64 {
    var x int64 = 0x1FFFF
    return int64(uint16(x))
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 65535);
}

// =============================================================================
// 6. Operator Precedence (Go Spec: 5 levels)
//    Level 5: *  /  %  <<  >>  &  &^
//    Level 4: +  -  |  ^
//    Level 3: ==  !=  <  <=  >  >=
//    Level 2: &&
//    Level 1: ||
// =============================================================================

#[test]
fn test_precedence_mul_before_add() {
    let source = r#"
package main

func F() int64 {
    return 2 + 3*4
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 14);
}

#[test]
fn test_precedence_left_to_right_associativity() {
    let source = r#"
package main

func F() int64 {
    return 100 / 10 * 2
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 20);
}

#[test]
fn test_precedence_shift_at_level5() {
    let source = r#"
package main

func F() int64 {
    return 1 + 2<<3
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 17);
}

#[test]
fn test_precedence_bitwise_and_before_or() {
    let source = r#"
package main

func F() int64 {
    return 0x0F | 0xFF & 0xF0
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 255);
}

#[test]
fn test_precedence_arithmetic_before_comparison() {
    let source = r#"
package main

func F() bool {
    return 2+3 > 4
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i32>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}

#[test]
fn test_precedence_comparison_before_logical_and() {
    let source = r#"
package main

func F() bool {
    return 3 > 2 && 5 > 4
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i32>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}

#[test]
fn test_precedence_and_before_or() {
    let source = r#"
package main

func F() bool {
    return false && true || true
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i32>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}

#[test]
fn test_precedence_or_lowest() {
    let source = r#"
package main

func F() bool {
    return true || true && false
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i32>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}

#[test]
fn test_precedence_complex_expression() {
    let source = r#"
package main

func F() int64 {
    return 2 + 3*4 - 10/5
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 12);
}

#[test]
fn test_precedence_modulo_at_level5() {
    let source = r#"
package main

func F() int64 {
    return 10 + 7%3
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 11);
}

// =============================================================================
// 7. Numeric Literals
// =============================================================================

#[test]
fn test_literal_binary() {
    let source = r#"
package main

func F() int64 {
    return 0b1010
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 10);
}

#[test]
fn test_literal_binary_uppercase_prefix() {
    let source = r#"
package main

func F() int64 {
    return 0B11111111
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 255);
}

#[test]
fn test_literal_octal_modern() {
    let source = r#"
package main

func F() int64 {
    return 0o17
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 15);
}

#[test]
fn test_literal_octal_legacy() {
    let source = r#"
package main

func F() int64 {
    return 0600
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 384);
}

#[test]
fn test_literal_octal_uppercase_prefix() {
    let source = r#"
package main

func F() int64 {
    return 0O17
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 15);
}

#[test]
fn test_literal_hex_lowercase() {
    let source = r#"
package main

func F() int64 {
    return 0xff
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 255);
}

#[test]
fn test_literal_hex_uppercase() {
    let source = r#"
package main

func F() int64 {
    return 0XFF
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 255);
}

#[test]
fn test_literal_hex_mixed_case_digits() {
    let source = r#"
package main

func F() int64 {
    return 0xBadFace
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0xBadFace);
}

#[test]
fn test_literal_underscore_decimal() {
    let source = r#"
package main

func F() int64 {
    return 1_000_000
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1_000_000);
}

#[test]
fn test_literal_underscore_hex() {
    let source = r#"
package main

func F() int64 {
    return 0xBad_Face
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0xBadFace);
}

#[test]
fn test_literal_underscore_binary() {
    let source = r#"
package main

func F() int64 {
    return 0b1111_0000
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0xF0);
}

#[test]
fn test_literal_underscore_octal() {
    let source = r#"
package main

func F() int64 {
    return 0o7_7_7
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0o777);
}

#[test]
fn test_literal_single_zero() {
    let source = r#"
package main

func F() int64 {
    return 0
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0);
}

// =============================================================================
// 8. Unsigned Integer Operations
// =============================================================================

#[test]
fn test_unsigned_comparison_large_values() {
    let source = r#"
package main

func F(a uint64, b uint64) bool {
    return a > b
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(i64, i64), i32>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, (-1i64, -2i64)).unwrap(), 1);
    assert_eq!(f.call(&mut store, (0i64, -1i64)).unwrap(), 0);
}

#[test]
fn test_unsigned_division_large() {
    let source = r#"
package main

func F(x uint64, y uint64) uint64 {
    return x / y
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, (-1i64, 2i64)).unwrap(), i64::MAX);
}

#[test]
fn test_unsigned_remainder_large() {
    let source = r#"
package main

func F(x uint64, y uint64) uint64 {
    return x % y
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, (-1i64, 2i64)).unwrap(), 1);
    assert_eq!(f.call(&mut store, (-1i64, 10i64)).unwrap(), 5);
}

#[test]
fn test_unsigned_subtraction_no_underflow() {
    let source = r#"
package main

func F(a uint64, b uint64) uint64 {
    return a - b
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(i64, i64), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, (10, 3)).unwrap(), 7);
    assert_eq!(f.call(&mut store, (-1i64, -1i64)).unwrap(), 0);
}

#[test]
fn test_unsigned_less_than() {
    let source = r#"
package main

func F(a uint64, b uint64) bool {
    return a < b
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(i64, i64), i32>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, (1, 2)).unwrap(), 1);
    assert_eq!(f.call(&mut store, (2, 1)).unwrap(), 0);
    assert_eq!(f.call(&mut store, (0i64, -1i64)).unwrap(), 1);
}

// =============================================================================
// 9. Mixed Signed/Unsigned Conversions
// =============================================================================

#[test]
fn test_signed_to_unsigned_negative() {
    let source = r#"
package main

func F() uint64 {
    var x int64 = -1
    return uint64(x)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), -1i64);
}

#[test]
fn test_unsigned_to_signed_large() {
    let source = r#"
package main

func F() int64 {
    var x uint64 = ^uint64(0)
    return int64(x)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), -1);
}

#[test]
fn test_int8_to_uint8_negative() {
    let source = r#"
package main

func F() int64 {
    return int64(uint8(int8(-1)))
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 255);
}

#[test]
fn test_unsigned_widening_chain() {
    let source = r#"
package main

func F() uint64 {
    return uint64(uint32(uint16(uint8(200))))
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 200);
}

#[test]
fn test_conversion_narrowing_sign_change() {
    let source = r#"
package main

func F() int64 {
    var x int64 = -1
    var y uint8 = uint8(x)
    var z int16 = int16(y)
    return int64(z)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 255);
}

// =============================================================================
// 10. Constants and Constant Expressions
// =============================================================================

#[test]
fn test_const_arithmetic() {
    let source = r#"
package main

const x = 3*5 + 2

func F() int {
    return x
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 17);
}

#[test]
fn test_const_iota_basic() {
    let source = r#"
package main

const (
    A = iota
    B
    C
    D
)

func F() int {
    return A + B + C + D
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 6);
}

#[test]
fn test_const_iota_with_shift_expression() {
    let source = r#"
package main

const (
    _  = iota
    KB = 1 << (10 * iota)
    MB
    GB
)

func F() int { return KB }
func G() int { return MB }
func H() int { return GB }
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let kb = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    let mb = instance
        .get_typed_func::<(), i64>(&mut store, "G")
        .unwrap();
    let gb = instance
        .get_typed_func::<(), i64>(&mut store, "H")
        .unwrap();
    assert_eq!(kb.call(&mut store, ()).unwrap(), 1024);
    assert_eq!(mb.call(&mut store, ()).unwrap(), 1048576);
    assert_eq!(gb.call(&mut store, ()).unwrap(), 1073741824);
}

#[test]
fn test_const_multiple_on_one_line() {
    let source = r#"
package main

const a, b = 10, 20

func F() int {
    return a + b
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 30);
}

#[test]
fn test_const_typed_int32() {
    let source = r#"
package main

const x int32 = 42

func F() int32 {
    return x
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i32>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 42);
}

#[test]
fn test_const_shift_expression() {
    let source = r#"
package main

const bits = 1 << 10

func F() int {
    return bits
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1024);
}

#[test]
fn test_const_untyped_in_typed_context() {
    let source = r#"
package main

const val = 100

func F() int64 {
    var x int64 = val
    return x
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 100);
}

#[test]
fn test_const_bitwise_expression() {
    let source = r#"
package main

const mask = 0xFF & 0x0F

func F() int {
    return mask
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0x0F);
}

#[test]
fn test_const_iota_bitmask_flags() {
    let source = r#"
package main

const (
    FlagA = 1 << iota
    FlagB
    FlagC
    FlagD
)

func F() int {
    return FlagA | FlagB | FlagC | FlagD
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance
        .get_typed_func::<(), i64>(&mut store, "F")
        .unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 15);
}
