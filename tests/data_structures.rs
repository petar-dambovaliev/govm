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
// 1. Struct Value Semantics and Operations
// =============================================================================

#[test]
fn test_struct_field_read_after_init() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func F() int {
    a := Point{X: 1, Y: 2}
    return a.X + a.Y
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 3);
}

#[test]
fn test_struct_assignment_copies_value() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func F() int {
    a := Point{X: 1, Y: 2}
    b := a
    b.X = 99
    return a.X
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}

#[test]
fn test_struct_passed_to_function_read() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func sum(p Point) int {
    return p.X + p.Y
}

func F() int {
    a := Point{X: 3, Y: 7}
    return sum(a)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 10);
}

#[test]
fn test_struct_passed_to_function_is_copy() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func mutate(p Point) int {
    p.X = 99
    return p.X
}

func F() int {
    a := Point{X: 1, Y: 2}
    mutate(a)
    return a.X
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}

#[test]
fn test_struct_zero_value_fields() {
    let source = r#"
package main

type Data struct {
    I int
    F float64
    B bool
}

func FI() int     { var d Data; return d.I }
func FF() float64 { var d Data; return d.F }
func FB() bool    { var d Data; return d.B }
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let fi = instance.get_typed_func::<(), i64>(&mut store, "FI").unwrap();
    let ff = instance.get_typed_func::<(), f64>(&mut store, "FF").unwrap();
    let fb = instance.get_typed_func::<(), i32>(&mut store, "FB").unwrap();
    assert_eq!(fi.call(&mut store, ()).unwrap(), 0);
    assert!((ff.call(&mut store, ()).unwrap()).abs() < f64::EPSILON);
    assert_eq!(fb.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_struct_nested_field_assignment() {
    let source = r#"
package main

type Inner struct {
    Val int
}

type Outer struct {
    In Inner
}

func F() int {
    o := Outer{}
    o.In.Val = 42
    return o.In.Val
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 42);
}

#[test]
fn test_struct_pointer_receiver_modifies_original() {
    let source = r#"
package main

type Counter struct {
    N int
}

func (c *Counter) Inc() {
    c.N = c.N + 1
}

func F() int {
    c := Counter{N: 0}
    c.Inc()
    c.Inc()
    c.Inc()
    return c.N
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 3);
}

#[test]
fn test_struct_promoted_field_access() {
    let source = r#"
package main

type Base struct {
    Value int
}

type Derived struct {
    Base
    Extra int
}

func F() int {
    d := Derived{Base: Base{Value: 10}, Extra: 20}
    return d.Value + d.Extra
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 30);
}

#[test]
fn test_struct_promoted_method() {
    let source = r#"
package main

type Base struct {
    Val int
}

func (b Base) GetVal() int {
    return b.Val
}

type Wrapper struct {
    Base
}

func F() int {
    w := Wrapper{Base: Base{Val: 55}}
    return w.GetVal()
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 55);
}

// =============================================================================
// 2. Slice Operations and Invariants
// =============================================================================

#[test]
fn test_slice_aliasing_shared_array() {
    let source = r#"
package main

func F() int {
    s := []int{10, 20, 30, 40, 50}
    t := s[1:4]
    t[0] = 99
    return s[1]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 99);
}

#[test]
fn test_slice_append_beyond_capacity_grows() {
    let source = r#"
package main

func F() int {
    s := make([]int, 2, 2)
    s[0] = 1
    s[1] = 2
    t := append(s, 3)
    return len(t)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 3);
}


#[test]
fn test_slice_len_cap_invariant() {
    let source = r#"
package main

func F() int {
    s := make([]int, 3, 10)
    if len(s) > cap(s) {
        return -1
    }
    if len(s) < 0 {
        return -2
    }
    s = append(s, 1, 2, 3)
    if len(s) > cap(s) {
        return -3
    }
    return len(s)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 6);
}

#[test]
fn test_nil_slice_len_cap_zero() {
    let source = r#"
package main

func FL() int { var s []int; return len(s) }
func FC() int { var s []int; return cap(s) }
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let fl = instance.get_typed_func::<(), i64>(&mut store, "FL").unwrap();
    let fc = instance.get_typed_func::<(), i64>(&mut store, "FC").unwrap();
    assert_eq!(fl.call(&mut store, ()).unwrap(), 0);
    assert_eq!(fc.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_append_to_nil_slice() {
    let source = r#"
package main

func F() int {
    var s []int
    s = append(s, 1, 2, 3)
    return len(s)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 3);
}

#[test]
fn test_slice_full_expression_capacity() {
    let source = r#"
package main

func F() int {
    s := []int{10, 20, 30, 40, 50}
    t := s[1:3:4]
    return cap(t)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 3);
}

#[test]
fn test_slice_full_expression_len() {
    let source = r#"
package main

func F() int {
    s := []int{10, 20, 30, 40, 50}
    t := s[1:3:4]
    return len(t)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 2);
}

#[test]
fn test_copy_returns_min_len() {
    let source = r#"
package main

func F() int {
    dst := make([]int, 2)
    src := []int{1, 2, 3, 4, 5}
    n := copy(dst, src)
    return n
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 2);
}

#[test]
fn test_append_variadic_spread() {
    let source = r#"
package main

func F() int {
    s1 := []int{1, 2}
    s2 := []int{3, 4, 5}
    s3 := append(s1, s2...)
    return len(s3)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 5);
}

#[test]
fn test_append_variadic_spread_values() {
    let source = r#"
package main

func F() int {
    s1 := []int{1, 2}
    s2 := []int{3, 4, 5}
    s3 := append(s1, s2...)
    return s3[0] + s3[1] + s3[2] + s3[3] + s3[4]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 15);
}

// =============================================================================
// 3. Map Operations and Guarantees
// =============================================================================

#[test]
fn test_map_zero_value_for_missing_int_key() {
    let source = r#"
package main

func F() int {
    m := make(map[string]int)
    return m["missing"]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_map_zero_value_for_missing_bool_key() {
    let source = r#"
package main

func F() bool {
    m := make(map[string]bool)
    return m["missing"]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i32>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_map_comma_ok_present() {
    let source = r#"
package main

func F() int {
    m := map[string]int{"key": 42}
    v, ok := m["key"]
    if ok {
        return v
    }
    return -1
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 42);
}

#[test]
fn test_map_comma_ok_missing() {
    let source = r#"
package main

func F() int {
    m := map[string]int{"key": 42}
    _, ok := m["other"]
    if ok {
        return 1
    }
    return 0
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_map_delete_nonexistent_key() {
    let source = r#"
package main

func F() int {
    m := map[string]int{"a": 1}
    delete(m, "nonexistent")
    return m["a"]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}

#[test]
fn test_map_len_after_insert_delete() {
    let source = r#"
package main

func F() int {
    m := make(map[string]int)
    m["a"] = 1
    m["b"] = 2
    m["c"] = 3
    delete(m, "b")
    return len(m)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 2);
}

#[test]
fn test_nil_map_read_returns_zero() {
    let source = r#"
package main

func F() int {
    var m map[string]int
    return m["anything"]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_map_nested_chained_access() {
    let source = r#"
package main

func F() int {
    m := make(map[string]map[string]int)
    inner := make(map[string]int)
    inner["b"] = 42
    m["a"] = inner
    return m["a"]["b"]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 42);
}

#[test]
fn test_map_literal_initialization() {
    let source = r#"
package main

func F() int {
    m := map[int]int{1: 10, 2: 20, 3: 30}
    return m[1] + m[2] + m[3]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 60);
}

#[test]
fn test_map_overwrite_value() {
    let source = r#"
package main

func F() int {
    m := map[string]int{"x": 1}
    m["x"] = 99
    return m["x"]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 99);
}

// =============================================================================
// 4. Array Value Semantics
// =============================================================================

#[test]
fn test_array_element_access() {
    let source = r#"
package main

func F() int {
    a := [3]int{1, 2, 3}
    return a[0] + a[1] + a[2]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 6);
}

#[test]
fn test_array_assignment_copies() {
    let source = r#"
package main

func F() int {
    a := [3]int{1, 2, 3}
    b := a
    b[0] = 99
    return a[0]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}

#[test]
fn test_array_passed_to_function_read() {
    let source = r#"
package main

func sum(arr [3]int) int {
    return arr[0] + arr[1] + arr[2]
}

func F() int {
    a := [3]int{10, 20, 30}
    return sum(a)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 60);
}

#[test]
fn test_array_passed_to_function_copies() {
    let source = r#"
package main

func mutate(arr [3]int) int {
    arr[0] = 99
    return arr[0]
}

func F() int {
    a := [3]int{1, 2, 3}
    mutate(a)
    return a[0]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}

#[test]
fn test_array_comparison_equal() {
    let source = r#"
package main

func F() bool {
    a := [3]int{1, 2, 3}
    b := [3]int{1, 2, 3}
    return a == b
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i32>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}

#[test]
fn test_array_comparison_not_equal() {
    let source = r#"
package main

func F() bool {
    a := [3]int{1, 2, 3}
    b := [3]int{1, 2, 4}
    return a != b
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i32>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}

#[test]
fn test_array_ellipsis_length() {
    let source = r#"
package main

func F() int {
    a := [...]int{10, 20, 30}
    return len(a)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 3);
}

#[test]
fn test_array_multidimensional() {
    let source = r#"
package main

func F() int {
    var a [2][3]int
    a[0][0] = 1
    a[0][1] = 2
    a[0][2] = 3
    a[1][0] = 4
    a[1][1] = 5
    a[1][2] = 6
    return a[0][0] + a[0][1] + a[0][2] + a[1][0] + a[1][1] + a[1][2]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 21);
}

#[test]
fn test_array_len_equals_cap() {
    let source = r#"
package main

func F() bool {
    a := [5]int{1, 2, 3, 4, 5}
    return len(a) == cap(a) && len(a) == 5
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i32>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}

// =============================================================================
// 5. Pointer Operations
// =============================================================================

#[test]
fn test_pointer_deref_roundtrip() {
    let source = r#"
package main

func F() int {
    x := 42
    p := &x
    return *p
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 42);
}

#[test]
fn test_pointer_read_through_deref() {
    let source = r#"
package main

func F() int {
    x := 10
    p := &x
    return *p + x
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 20);
}

#[test]
fn test_pointer_modification_through_deref() {
    let source = r#"
package main

func F() int {
    x := 10
    p := &x
    *p = 20
    return x
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 20);
}

#[test]
fn test_pointer_struct_auto_deref() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func F() int {
    p := &Point{X: 3, Y: 4}
    return p.X + p.Y
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 7);
}

#[test]
fn test_new_returns_zero_value() {
    let source = r#"
package main

func F() int {
    p := new(int)
    return *p
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_pointer_to_struct_field_write() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func F() int {
    p := &Point{}
    p.X = 10
    p.Y = 20
    return p.X + p.Y
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 30);
}

#[test]
fn test_pointer_pass_and_read() {
    let source = r#"
package main

func readPtr(p *int) int {
    return *p
}

func F() int {
    x := 42
    return readPtr(&x)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 42);
}

#[test]
fn test_pointer_shared_mutation() {
    let source = r#"
package main

func inc(p *int) {
    *p = *p + 1
}

func F() int {
    x := 0
    inc(&x)
    inc(&x)
    inc(&x)
    return x
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 3);
}

// =============================================================================
// 6. Composite Literals
// =============================================================================

#[test]
fn test_struct_literal_partial_fields_zero() {
    let source = r#"
package main

type Rec struct {
    A int
    B int
    C int
}

func F() int {
    r := Rec{A: 5}
    return r.A + r.B + r.C
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 5);
}

#[test]
fn test_slice_literal_len_cap() {
    let source = r#"
package main

func FL() int {
    s := []int{1, 2, 3}
    return len(s)
}

func FC() int {
    s := []int{1, 2, 3}
    return cap(s)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let fl = instance.get_typed_func::<(), i64>(&mut store, "FL").unwrap();
    let fc = instance.get_typed_func::<(), i64>(&mut store, "FC").unwrap();
    assert_eq!(fl.call(&mut store, ()).unwrap(), 3);
    assert_eq!(fc.call(&mut store, ()).unwrap(), 3);
}

#[test]
fn test_map_literal_access() {
    let source = r#"
package main

func F() int {
    m := map[string]int{"alpha": 1, "beta": 2}
    return m["alpha"] + m["beta"]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 3);
}

#[test]
fn test_address_of_composite_literal() {
    let source = r#"
package main

type Point struct {
    X int
    Y int
}

func F() int {
    p := &Point{X: 10, Y: 20}
    return p.X + p.Y
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 30);
}

#[test]
fn test_array_literal_with_ellipsis_values() {
    let source = r#"
package main

func F() int {
    a := [...]int{10, 20, 30}
    return a[0] + a[1] + a[2]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 60);
}

#[test]
fn test_nested_struct_composite_literal() {
    let source = r#"
package main

type Inner struct {
    V int
}

type Outer struct {
    In Inner
    N  int
}

func F() int {
    o := Outer{In: Inner{V: 5}, N: 10}
    return o.In.V + o.N
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 15);
}

// =============================================================================
// 7. Range Operations
// =============================================================================

#[test]
fn test_range_slice_sum() {
    let source = r#"
package main

func F() int {
    s := []int{1, 2, 3, 4, 5}
    sum := 0
    for _, v := range s {
        sum = sum + v
    }
    return sum
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 15);
}

#[test]
fn test_range_slice_index() {
    let source = r#"
package main

func F() int {
    s := []int{10, 20, 30}
    sum := 0
    for i := range s {
        sum = sum + i
    }
    return sum
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 3);
}

#[test]
fn test_range_array() {
    let source = r#"
package main

func F() int {
    a := [4]int{2, 4, 6, 8}
    sum := 0
    for _, v := range a {
        sum = sum + v
    }
    return sum
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 20);
}

#[test]
fn test_range_map_collects_all_values() {
    let source = r#"
package main

func F() int {
    m := map[int]int{1: 10, 2: 20, 3: 30}
    sum := 0
    for _, v := range m {
        sum = sum + v
    }
    return sum
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 60);
}

#[test]
fn test_range_nil_slice_zero_iterations() {
    let source = r#"
package main

func F() int {
    var s []int
    count := 0
    for range s {
        count = count + 1
    }
    return count
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_range_nil_map_zero_iterations() {
    let source = r#"
package main

func F() int {
    var m map[string]int
    count := 0
    for range m {
        count = count + 1
    }
    return count
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_range_blank_identifier_value() {
    let source = r#"
package main

func F() int {
    s := []int{5, 10, 15}
    sum := 0
    for _, v := range s {
        sum = sum + v
    }
    return sum
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 30);
}

#[test]
fn test_range_map_all_keys_visited() {
    let source = r#"
package main

func F() int {
    m := map[int]int{1: 0, 2: 0, 3: 0, 4: 0, 5: 0}
    sum := 0
    for k := range m {
        sum = sum + k
    }
    return sum
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 15);
}

// =============================================================================
// 8. Built-in Functions (len, cap, make, copy, append, delete)
// =============================================================================

#[test]
fn test_len_nil_map_returns_zero() {
    let source = r#"
package main

func F() int {
    var m map[string]int
    return len(m)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_make_slice_len_and_cap() {
    let source = r#"
package main

func FL() int {
    s := make([]int, 3, 10)
    return len(s)
}

func FC() int {
    s := make([]int, 3, 10)
    return cap(s)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let fl = instance.get_typed_func::<(), i64>(&mut store, "FL").unwrap();
    let fc = instance.get_typed_func::<(), i64>(&mut store, "FC").unwrap();
    assert_eq!(fl.call(&mut store, ()).unwrap(), 3);
    assert_eq!(fc.call(&mut store, ()).unwrap(), 10);
}

#[test]
fn test_make_map_with_capacity() {
    let source = r#"
package main

func F() int {
    m := make(map[string]int, 10)
    m["x"] = 5
    return m["x"]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 5);
}

#[test]
fn test_copy_correct_values() {
    let source = r#"
package main

func F() int {
    src := []int{10, 20, 30}
    dst := make([]int, 5)
    copy(dst, src)
    return dst[0] + dst[1] + dst[2]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 60);
}

#[test]
fn test_copy_shorter_dst() {
    let source = r#"
package main

func F() int {
    src := []int{10, 20, 30, 40}
    dst := make([]int, 2)
    n := copy(dst, src)
    return n
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 2);
}

#[test]
fn test_append_grows_slice() {
    let source = r#"
package main

func F() int {
    s := make([]int, 0, 2)
    s = append(s, 1)
    s = append(s, 2)
    s = append(s, 3)
    return len(s)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 3);
}

#[test]
fn test_delete_from_map() {
    let source = r#"
package main

func F() int {
    m := map[string]int{"a": 1, "b": 2, "c": 3}
    delete(m, "b")
    return len(m)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 2);
}

#[test]
fn test_delete_value_gone() {
    let source = r#"
package main

func F() int {
    m := map[string]int{"a": 1, "b": 2}
    delete(m, "a")
    return m["a"]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 0);
}

#[test]
fn test_make_slice_zero_len() {
    let source = r#"
package main

func F() int {
    s := make([]int, 0)
    s = append(s, 42)
    return s[0]
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 42);
}

#[test]
fn test_len_after_multiple_appends() {
    let source = r#"
package main

func F() int {
    var s []int
    for i := 0; i < 100; i++ {
        s = append(s, i)
    }
    return len(s)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i64>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 100);
}

#[test]
fn test_cap_grows_on_append() {
    let source = r#"
package main

func F() bool {
    s := make([]int, 0, 1)
    s = append(s, 1, 2, 3)
    return cap(s) >= len(s)
}
"#;
    let (mut store, instance) = compile_and_instantiate(source);
    let f = instance.get_typed_func::<(), i32>(&mut store, "F").unwrap();
    assert_eq!(f.call(&mut store, ()).unwrap(), 1);
}
