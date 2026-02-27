use gno_rs::wasm::compiler::WasmCompiler;
use gno_rs::wasm::runtime::{HostState, UdfRuntime};

fn compile_and_instantiate(source: &str) -> (wasmtime::Store<HostState>, wasmtime::Instance) {
    let mut compiler = WasmCompiler::new();
    let result = compiler.compile_source(source).expect("compilation failed");

    let runtime = UdfRuntime::new().expect("runtime init failed");
    let module = runtime
        .load_module(&result.wasm_bytes)
        .expect("module load failed");

    let state = HostState::new().with_type_layouts(result.type_layouts);
    let mut store = runtime
        .create_store(state, 1_000_000)
        .expect("store creation failed");
    let instance = runtime
        .instantiate(&mut store, &module)
        .expect("instantiation failed");

    (store, instance)
}

// =============================================================================
// 1. Deterministic compilation
// =============================================================================

#[test]
fn test_deterministic_compilation() {
    let src = r#"package main

type Speaker interface { Speak() int }
type Walker interface { Walk() int }
type Flyer interface { Fly() int }

type Dog struct { N int }
func (d Dog) Speak() int { return d.N }
func (d Dog) Walk() int { return d.N + 1 }

type Cat struct { N int }
func (c Cat) Speak() int { return c.N * 2 }

type Bird struct { N int }
func (b Bird) Speak() int { return b.N + 10 }
func (b Bird) Fly() int { return b.N + 20 }

func F() int {
    var s Speaker = Dog{N: 5}
    return s.Speak()
}
"#;
    let mut first_bytes: Option<Vec<u8>> = None;
    for i in 0..5 {
        let mut compiler = WasmCompiler::new();
        let result = compiler
            .compile_source(src)
            .unwrap_or_else(|e| panic!("compilation #{} failed: {:?}", i, e));
        match &first_bytes {
            None => first_bytes = Some(result.wasm_bytes),
            Some(expected) => {
                assert_eq!(
                    expected.len(),
                    result.wasm_bytes.len(),
                    "WASM byte length differs on compilation #{}",
                    i
                );
                assert_eq!(
                    expected, &result.wasm_bytes,
                    "WASM bytes differ on compilation #{}",
                    i
                );
            }
        }
    }
}

// =============================================================================
// 2. Interface equality — different type, different value
// =============================================================================

#[test]
fn test_interface_eq_same_type_diff_value() {
    let src = r#"package main

type Valuer interface { Val() int }
type MyNum struct { N int }
func (m MyNum) Val() int { return m.N }

func F() int {
    var a Valuer = MyNum{N: 1}
    var b Valuer = MyNum{N: 2}
    if a == b {
        return 0
    }
    return 1
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        1,
        "interfaces holding same type but different values should not be equal"
    );
}

// =============================================================================
// 4. Interface equality — different concrete types
// =============================================================================

#[test]
fn test_interface_eq_different_types() {
    let src = r#"package main

type Valuer interface { Val() int }
type TypeA struct { N int }
func (a TypeA) Val() int { return a.N }
type TypeB struct { N int }
func (b TypeB) Val() int { return b.N }

func F() int {
    var a Valuer = TypeA{N: 42}
    var b Valuer = TypeB{N: 42}
    if a == b {
        return 0
    }
    return 1
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        1,
        "interfaces holding different concrete types should not be equal"
    );
}

// =============================================================================
// 5. Struct with string field comparison via interface
// =============================================================================

#[test]
fn test_struct_string_field_comparison() {
    let src = r#"package main

type Named struct {
    Name string
    Age  int
}

func F() int {
    a := Named{Name: "alice", Age: 30}
    b := Named{Name: "alice", Age: 30}
    if a == b {
        return 1
    }
    return 0
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        1,
        "structs with same string and int fields should be equal"
    );
}

#[test]
fn test_struct_string_field_not_equal() {
    let src = r#"package main

type Named struct {
    Name string
    Age  int
}

func F() int {
    a := Named{Name: "alice", Age: 30}
    b := Named{Name: "bob", Age: 30}
    if a != b {
        return 1
    }
    return 0
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        1,
        "structs with different string fields should not be equal"
    );
}

// =============================================================================
// 6. Vtable dispatch — interface method call
// =============================================================================

#[test]
fn test_vtable_dispatch() {
    let src = r#"package main

type Worker interface { Work() int }

type FastWorker struct { Speed int }
func (f FastWorker) Work() int { return f.Speed * 2 }

type SlowWorker struct { Speed int }
func (s SlowWorker) Work() int { return s.Speed }

func F() int {
    var w Worker = FastWorker{Speed: 10}
    r1 := w.Work()
    w = SlowWorker{Speed: 5}
    r2 := w.Work()
    return r1 + r2
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        25,
        "vtable dispatch should correctly call methods on different concrete types"
    );
}

// =============================================================================
// 7. Multiple interface dispatch
// =============================================================================

#[test]
fn test_multiple_interface_dispatch() {
    let src = r#"package main

type Greeter interface { Greet() int }
type Counter interface { Count() int }

type Bot struct { N int }
func (b Bot) Greet() int { return b.N + 100 }
func (b Bot) Count() int { return b.N }

func F() int {
    var g Greeter = Bot{N: 5}
    var c Counter = Bot{N: 5}
    return g.Greet() + c.Count()
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        110,
        "a type implementing multiple interfaces should dispatch correctly through each"
    );
}

// =============================================================================
// Bug 1: Right-shift signedness should depend only on LHS type
// =============================================================================

#[test]
fn test_right_shift_signed_lhs_unsigned_rhs() {
    let src = r#"package main

func F() int {
    var x int = -8
    var n uint = 1
    return x >> n
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        -4,
        "right-shift of signed int by unsigned count should be arithmetic (sign-preserving)"
    );
}

#[test]
fn test_right_shift_unsigned_lhs() {
    let src = r#"package main

func F() int {
    var x uint = 16
    var n uint = 2
    return int(x >> n)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        4,
        "right-shift of unsigned int should be logical"
    );
}

// =============================================================================
// Bug 2: /= and %= should not compile as addition
// =============================================================================

#[test]
fn test_quo_assign_local() {
    let src = r#"package main

func F() int {
    x := 100
    x /= 5
    return x
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        20,
        "/= on local variable should divide, not add"
    );
}

#[test]
fn test_rem_assign_local() {
    let src = r#"package main

func F() int {
    x := 17
    x %= 5
    return x
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        2,
        "%= on local variable should compute remainder, not add"
    );
}

// =============================================================================
// Bug 5: append() on nil slice should not corrupt memory
// =============================================================================

#[test]
fn test_append_nil_slice() {
    let src = r#"package main

func F() int {
    var s []int
    s = append(s, 10)
    s = append(s, 20)
    s = append(s, 30)
    result := 0
    for i := 0; i < len(s); i++ {
        result = result + s[i]
    }
    return result
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        60,
        "appending to a nil slice should produce correct results"
    );
}

#[test]
fn test_append_nil_slice_no_corruption() {
    let src = r#"package main

func F() int {
    var s []int
    s = append(s, 1)
    var s2 []int
    return len(s) + len(s2)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        1,
        "appending to nil slice should not corrupt other nil slice lengths"
    );
}

// =============================================================================
// Bug 9: Octal escape sequences in character literals
// =============================================================================

#[test]
fn test_octal_escape_nul() {
    let src = r#"package main

func F() int {
    c := '\000'
    return int(c)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        0,
        "\\000 should be NUL character (0)"
    );
}

#[test]
fn test_octal_escape_question_mark() {
    let src = r#"package main

func F() int {
    c := '\077'
    return int(c)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        63,
        "\\077 should be '?' (63)"
    );
}

#[test]
fn test_octal_escape_uppercase_a() {
    let src = r#"package main

func F() int {
    c := '\101'
    return int(c)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        65,
        "\\101 should be 'A' (65)"
    );
}

#[test]
fn test_bitwise_and_assign() {
    let src = r#"package main

func F() int {
    x := 0xFF
    x &= 0x0F
    return x
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0x0F, "0xFF &= 0x0F should be 0x0F");
}

#[test]
fn test_bitwise_or_assign() {
    let src = r#"package main

func F() int {
    x := 0xF0
    x |= 0x0F
    return x
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0xFF, "0xF0 |= 0x0F should be 0xFF");
}

#[test]
fn test_bitwise_xor_assign() {
    let src = r#"package main

func F() int {
    x := 0xFF
    x ^= 0x0F
    return x
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0xF0, "0xFF ^= 0x0F should be 0xF0");
}

#[test]
fn test_shl_assign() {
    let src = r#"package main

func F() int {
    x := 1
    x <<= 4
    return x
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 16, "1 <<= 4 should be 16");
}

#[test]
fn test_shr_assign() {
    let src = r#"package main

func F() int {
    x := 256
    x >>= 4
    return x
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 16, "256 >>= 4 should be 16");
}

#[test]
fn test_andnot_assign() {
    let src = r#"package main

func F() int {
    x := 0xFF
    x &^= 0x0F
    return x
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0xF0, "0xFF &^= 0x0F should be 0xF0");
}

#[test]
fn test_make_slice_zero_initialized() {
    let src = r#"package main

func F() int {
    s := make([]int, 5)
    sum := 0
    for i := 0; i < 5; i++ {
        sum += s[i]
    }
    return sum
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0, "make([]int, 5) should be zero-initialized");
}

#[test]
fn test_append_spread_nil_dst() {
    let src = r#"package main

func F() int {
    src := []int{10, 20, 30}
    var dst []int
    dst = append(dst, src...)
    return dst[0] + dst[1] + dst[2]
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 60, "append(nil, src...) should work");
}

#[test]
fn test_append_spread_nil_src() {
    let src = r#"package main

func F() int {
    dst := []int{1, 2, 3}
    var src []int
    dst = append(dst, src...)
    return len(dst)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 3, "append(dst, nil...) should keep dst unchanged");
}

#[test]
fn test_make_len_gt_cap_panics() {
    let src = r#"package main

func F() int {
    s := make([]int, 10, 5)
    return len(s)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert!(f.call(&mut s, ()).is_err(), "make([]int, 10, 5) should panic when len > cap");
}

#[test]
fn test_make_len_eq_cap_ok() {
    let src = r#"package main

func F() int {
    s := make([]int, 5, 5)
    return len(s)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 5, "make([]int, 5, 5) should work fine");
}

#[test]
fn test_type_switch_init_scope() {
    let src = r#"package main

type Stringer interface {
    String() string
}

type MyStr struct {
    val int
}

func (m MyStr) String() string {
    return "hello"
}

func F() int {
    x := 10
    var i Stringer
    i = MyStr{val: 42}
    switch x := 100; i.(type) {
    case MyStr:
        return x
    }
    return 0
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 100, "type switch init var x should shadow outer x");
}

#[test]
fn test_struct_field_not_misidentified_as_companion() {
    let src = r#"package main

type S struct {
    A   int
    A_1 int
    B   int
}

func F() int {
    s := S{10, 20, 30}
    return s.A + s.A_1 + s.B
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(
        f.call(&mut s, ()).unwrap(),
        60,
        "positional struct literal with A, A_1, B should assign 10, 20, 30 correctly"
    );
}

// =============================================================================
// Regression: cap() on 2-result expression must return length, not pointer
// =============================================================================

#[test]
fn test_cap_returns_length_not_pointer() {
    let src = r#"package main

func F() int {
    s := make([]int, 5, 10)
    return cap(s)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 10, "cap() must return the capacity, not a pointer");
}

// =============================================================================
// Regression: %= (RemAssign) must use correct integer rem instructions
// =============================================================================

#[test]
fn test_rem_assign_integer() {
    let src = r#"package main

func F() int {
    x := 17
    x %= 5
    return x
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 2, "17 %%= 5 should be 2");
}

// =============================================================================
// Regression: clear() on struct field map must not corrupt memory
// =============================================================================

#[test]
fn test_clear_map_on_struct_field() {
    let src = r#"package main

type S struct {
    M map[int]int
}

func F() int {
    s := S{M: map[int]int{1: 10, 2: 20}}
    clear(s.M)
    return len(s.M)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 0, "clear(s.M) should empty the map");
}

// =============================================================================
// Regression: range over map with string values must preserve string length
// =============================================================================

#[test]
fn test_range_map_string_values() {
    let src = r#"package main

func F() int {
    m := map[int]string{1: "hello", 2: "world"}
    total := 0
    for _, v := range m {
        total += len(v)
    }
    return total
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 10, "range over string-valued map should preserve string lengths");
}

// =============================================================================
// Regression: arrays of strings must use 8-byte elements (ptr + len)
// =============================================================================

#[test]
fn test_string_array_element_size() {
    let src = r#"package main

func F() int {
    a := [3]string{"ab", "cd", "ef"}
    total := 0
    total += len(a[0])
    total += len(a[1])
    total += len(a[2])
    return total
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 6, "string array elements must not overlap");
}

// =============================================================================
// Host runtime function tests
// =============================================================================

#[test]
fn test_host_rt_streq() {
    let src = r#"package main
func F() int {
    a := "hello"
    b := "hello"
    c := "world"
    r := 0
    if a == b { r += 1 }
    if a != c { r += 10 }
    if "" == "" { r += 100 }
    return r
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 111);
}

#[test]
fn test_host_rt_strcmp() {
    let src = r#"package main
func F() int {
    r := 0
    if "abc" < "abd" { r += 1 }
    if "xyz" > "abc" { r += 10 }
    if "abc" <= "abc" { r += 100 }
    if "abc" >= "abc" { r += 1000 }
    return r
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1111);
}

#[test]
fn test_host_rt_str_concat() {
    let src = r#"package main
func F() int {
    a := "hello" + " " + "world"
    return len(a)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 11);
}

#[test]
fn test_host_rt_str_concat_empty() {
    let src = r#"package main
func F() int {
    a := "" + ""
    b := "x" + ""
    c := "" + "y"
    return len(a) + len(b) + len(c)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 2);
}

#[test]
fn test_host_rt_i64_to_str() {
    let src = r#"package main
func F() int {
    x := 42
    s := "" + string(rune(48 + x%10))
    return len(s)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 1);
}

#[test]
fn test_host_rt_alloc() {
    let src = r#"package main
func F() int {
    a := "first"
    b := "second"
    c := "third"
    return len(a) + len(b) + len(c)
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 16);
}

#[test]
fn test_host_struct_eq() {
    let src = r#"package main
type Point struct {
    X int
    Y int
}
func F() int {
    a := Point{X: 1, Y: 2}
    b := Point{X: 1, Y: 2}
    c := Point{X: 3, Y: 4}
    r := 0
    if a == b { r += 1 }
    if a != c { r += 10 }
    return r
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 11);
}

#[test]
fn test_host_struct_string_field_eq() {
    let src = r#"package main
type Person struct {
    Name string
    Age  int
}
func F() int {
    a := Person{Name: "Alice", Age: 30}
    b := Person{Name: "Alice", Age: 30}
    c := Person{Name: "Bob", Age: 30}
    r := 0
    if a == b { r += 1 }
    if a != c { r += 10 }
    return r
}
"#;
    let (mut s, inst) = compile_and_instantiate(src);
    let f = inst.get_typed_func::<(), i64>(&mut s, "F").unwrap();
    assert_eq!(f.call(&mut s, ()).unwrap(), 11);
}
