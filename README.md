# gno-rs

A virtual machine for Go written in Rust.

## Architecture

The project follows a classic three-stage pipeline:

```
Go Source (.go) → Parser → AST → Compiler → Bytecode → Stack-based VM → Output
```

### Source Layout

- **Parser**: `src/parser/` -- scanner, tokens, AST, parser
- **Compiler**: `src/vm/compiler/` -- opcodes, bytecode emission, symbol table
- **VM Runtime**: `src/vm/mod.rs` -- execution loop, stack, frames
- **Object System**: `src/vm/object/` -- tagged-pointer objects for all types
- **Builtins**: `src/vm/builtin.rs` -- print, len, make, append, etc.
- **Module System**: `src/gomod/` -- go.mod parsing, dependency management
- **Entry Point**: `src/main.rs` -- CLI (clap)

## What Is Done

### Parser / Lexer
- Full Go token set including all operators, keywords, literals (int, float, string, rune, imaginary)
- Automatic semicolon insertion
- Unicode identifier support
- Binary/octal/hex integer literals with `_` separators

### AST
- All major Go constructs represented: functions, methods, structs, interfaces, closures, for/if/switch/select/range, type assertions, type switches, composite literals, pointers, channels (syntax only), go/defer (syntax only)

### Compiler (118 opcodes)
- Arithmetic, comparison, logical operators
- Variable access: local, global, captured (closures)
- Control flow: jumps, calls, returns
- Collections: arrays, slices, maps (make, index, slice)
- Structs, interfaces, dynamic dispatch
- Type operations: upcast, downcast, type comparison, casts
- Optimized opcode variants (e.g. `AddLocalConst` combining local read + constant add)
- Dependency graph for package initialization order
- Closure compilation with captured variable tracking

### VM Runtime
- Stack-based execution with call frames
- 85+ opcodes handled in the main loop
- Pointer operations (ref, deref, writes)
- Iterators and range loops
- Variadic function calls
- Multiple return values
- Boehm GC allocator integrated (via `bdwgc-alloc`)

### Object System (tagged pointers)
- Primitives: int (all widths), uint (all widths), float32/64, bool, byte, rune, complex64/128, string
- Collections: array, slice, map, variadic
- Composite: struct, interface, alias, type values
- Functions/closures, references, iterators

### Builtins
- `print`, `println`, `len`, `make`, `cap`, `append`, `copy`, `delete`, `clear`, `byte`, `rune`, `int64`, `sprintf`, `gccollect`

### Package System
- `package` declarations, local imports, foreign imports
- `go.mod` parsing and module resolution
- CLI commands: `mod init`, `mod add`, `mod list`, `mod remove`

### Tests
- UI integration tests (run `.go` programs and check stdout)
- Unit tests for object types, scanner, parser, module resolution
- Fibonacci benchmark (`benches/fibonacci.rs`)

## TODO

### Phase 1: Solidify sequential execution

- [ ] Implement `defer` (add a deferred-call stack to the VM and compile `Statement::Defer`)
- [ ] Implement `panic()` / `recover()` builtins with stack unwinding
- [ ] Implement Go constants with `iota` support
- [ ] Complete type alias support (currently `unimplemented!` in `dep_graph.rs`)
- [ ] Implement `TypeOf` opcode in VM execution loop (currently `unimplemented!`)
- [ ] Fix all remaining `unimplemented!()` panics in the compiler and VM for sequential features
- [ ] Add UI tests for closures, structs, interfaces, type switches, slices, maps, pointers, switch/case

### Phase 2: Concurrency

- [ ] Design and implement a goroutine scheduler
- [ ] Implement channel objects (buffered and unbuffered), send/receive opcodes
- [ ] Implement `select` statement compilation and execution
- [ ] Add `go` statement compilation (spawn goroutine)

### Phase 3: Polish

- [ ] Remote dependency fetching
- [ ] `build` CLI command
- [ ] Proper GC integration
- [ ] Struct tags
- [ ] Better error messages and diagnostics
- [ ] More builtins (`string()`, `int()`, `float()` conversions)
