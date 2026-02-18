# govm

A Go virtual machine written in Rust. It parses, compiles, and executes Go source files on a stack-based bytecode VM.

## Quick Start

```bash
# Build from source
cargo build --release

# Run a Go source file
govm run path/to/main.go

# Compile to bytecode
govm build path/to/main.go              # produces main.govm
govm build path/to/main.go -o out.govm  # custom output path

# Run pre-compiled bytecode
govm run out.govm

# Module management
govm mod init [module-name]
govm mod add <dependency>
govm mod list
govm mod remove <dependency>
```

## Example

```go
package main

func main() {
    println("hello world")
}
```

```bash
$ govm run hello.go
hello world
```

## Architecture

```mermaid
flowchart LR
    Source["Go Source (.go)"] --> Scanner --> Tokens --> Parser --> AST
    AST --> Compiler --> Bytecode --> VM --> Output
```

| Stage | Source | Description |
|-------|--------|-------------|
| Scanner / Parser | `src/parser/` | Tokenizer, AST construction, automatic semicolon insertion |
| Compiler | `src/vm/compiler/` | Bytecode emission, symbol table, type checking (119 opcodes) |
| VM | `src/vm/mod.rs` | Stack-based execution loop, call frames, defer/panic/recover |
| Object System | `src/vm/object/` | Tagged-pointer values: ints, floats, strings, arrays, maps, structs, closures |
| Builtins | `src/vm/builtin.rs` | `print`, `len`, `make`, `append`, `copy`, `delete`, `cap`, `panic`, `recover` |
| Modules | `src/gomod/` | `go.mod` parsing, dependency resolution |

### Design Decisions

- **Memory management** -- Boehm GC via `bdwgc-alloc` as the global allocator.
- **Object representation** -- Tagged pointers with inline small-integer optimization (56-bit range encoded directly in the pointer, no heap allocation).
- **Concurrency** -- Tokio single-threaded runtime; goroutines are async tasks communicating over channels.

## Supported Go Features

- Variables, constants (including `iota`), type aliases
- Functions, closures, multiple return values, variadic arguments
- Structs, methods, interfaces, dynamic dispatch
- Arrays, slices, maps
- Pointers (ref / deref)
- Control flow: `if`/`else`, `for`, `range`, `switch`, `select`
- `defer`, `panic()`, `recover()`
- Goroutines and channels (buffered / unbuffered)
- Package imports (local and remote)

## Testing

```bash
cargo test
```

The test suite includes UI integration tests that compile and run `.go` programs under `tests/ui/`, verifying their stdout output.
