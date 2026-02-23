# Future Tasks: Go Spec Feature Gaps

This document tracks Go language specification features that are missing or incomplete in the govm WASM compiler. Concurrency (goroutines, channels, select) and unsafe are intentionally excluded from scope.

---

## Language Features

### `goto` Statement

- **Status:** Not supported
- **Impact:** Low
- **Details:** Explicitly rejected with an error in `src/wasm/compiler/statements.rs` (line 2734). WASM uses structured control flow, so `goto` would require a relooper/stackifier transformation. Rarely used in idiomatic Go.

### Range Over Function Iterators (Go 1.22+)

- **Status:** Not supported
- **Impact:** Medium
- **Details:** Explicitly rejected with an error in `src/wasm/compiler/statements.rs` (line 1956). This is a newer feature (Go 1.22/1.23) becoming increasingly popular with `iter` package patterns.

### Named Type Conversions (Edge Cases)

- **Status:** Partially broken
- **Impact:** Medium
- **Details:** Works with methods (`MyInt(10)`, `int(a.Add(b))`) but standalone conversion edge cases may fail. Two tests are commented out in `tests/wasm_integration.rs`: `test_named_type_conversion` (line 19064) and `test_named_type_round_trip_conversion` (line 24230). Both test patterns like `type MyInt int; var x MyInt = MyInt(42); int(x)`.

---

## Standard Library: Fully Stubbed Packages

The following packages are listed as "allowed" in `src/wasm/stdlib/mod.rs` but have no Go source files. All function calls return "not yet implemented" errors (see `src/wasm/compiler/calls.rs`, lines 2200-2211).

### `strings`

- **Status:** Stub only
- **Impact:** High

### `strconv`

- **Status:** Stub only
- **Impact:** High

### `fmt` (Sprintf, Errorf, etc.)

- **Status:** Stub only
- **Impact:** High
- **Details:** `Sprintf` and `Errorf` return a specific "not yet available" error. All other functions also error.

### `sort`

- **Status:** Stub only
- **Impact:** Medium

### `bytes`

- **Status:** Stub only
- **Impact:** Medium

### `encoding/json`

- **Status:** Stub only
- **Impact:** High

### `unicode`

- **Status:** Stub only
- **Impact:** Low
- **Details:** Only `unicode/utf8` has Go source and likely works.

---

## Standard Library: Partially Implemented Packages

### `errors.As` and `errors.Join`

- **Status:** Not implemented
- **Impact:** Medium
- **Details:**
  - `errors.New`, `errors.Is`, `errors.Unwrap` are implemented.
  - `errors.As` is commented out in `src/wasm/stdlib/errors/wrap.go` (line 31) because it requires reflection.
  - `errors.Join` is fully commented out in `src/wasm/stdlib/errors/join.go`.

### `time`

- **Status:** Source exists, likely broken
- **Impact:** Medium
- **Details:** Has Go source files (`time.go`, `format.go`, `zoneinfo.go`) but all ~30 time-related tests are commented out. Missing or broken: Duration methods, formatting, parsing, timezone handling, `time.Now()` (requires a host function).

### `math.Trunc` and `math.Round`

- **Status:** Possibly broken
- **Impact:** Low
- **Details:** Core math functions (`Abs`, `Sqrt`, `Floor`, `Ceil`, `Min`, `Max`) are inlined as WASM instructions. Pure Go implementations exist for transcendental functions. However, tests for `math.Trunc` and `math.Round` are commented out, suggesting possible issues. Architecture-specific stubs panic but are gated behind `haveArch*` constants set to `false`, so the Go fallbacks should be used instead.

---

## Type System

### Missing `UntypedFloat` in `GoType` Enum

- **Status:** Missing variant
- **Impact:** Low
- **Details:** The `GoType` enum in `src/wasm/compiler/mod.rs` (line 207) has `UntypedInt` but no `UntypedFloat`. Untyped float constants likely default to `Float64`, but this could cause subtle issues in constant expression evaluation where the Go spec requires distinguishing untyped int from untyped float (e.g., `const x = 1.0` should be untyped float, `const x = 1` should be untyped int).

---

## Summary

| Category | Feature | Status | Impact |
|----------|---------|--------|--------|
| Language | `goto` | Not supported | Low |
| Language | Range over function iterators | Not supported | Medium |
| Language | Named type conversion (basic types) | Partially broken | Medium |
| Stdlib | `strings` | Stub only | High |
| Stdlib | `strconv` | Stub only | High |
| Stdlib | `fmt` (Sprintf, Errorf, etc.) | Stub only | High |
| Stdlib | `sort` | Stub only | Medium |
| Stdlib | `bytes` | Stub only | Medium |
| Stdlib | `encoding/json` | Stub only | High |
| Stdlib | `unicode` | Stub only | Low |
| Stdlib | `errors.As`, `errors.Join` | Not implemented | Medium |
| Stdlib | `time` | Source exists, likely broken | Medium |
| Stdlib | `math.Trunc`, `math.Round` | Possibly broken | Low |
| Types | `UntypedFloat` in GoType | Missing variant | Low |
