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
