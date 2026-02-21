# Performance Analysis & Optimization Roadmap

## Current Architecture

govm compiles Go source code to WebAssembly and executes it via Wasmtime (which JIT-compiles WASM to native machine code). The compilation pipeline is:

```
Go Source → Parse (AST) → Symbol Resolution → WASM Bytecode → Wasmtime JIT → Native Code
```

There are currently no optimization passes between symbol resolution and WASM emission. The compiler does single-pass code generation.

---

## Performance Comparison

### vs. Compiled Go (`go build`, native code) — ~2-5x slower

Native Go via the `gc` compiler produces highly optimized machine code with register allocation, inlining, escape analysis, bounds-check elimination, a concurrent GC, and an SSA-based optimization pipeline.

| Factor | Impact |
|---|---|
| No optimization passes (no inlining, constant folding, DCE) | ~1.5-2x slowdown on compute-heavy code |
| WASM linear memory (bounds-checked loads/stores) | ~1.2-1.5x overhead for memory-intensive code |
| No source-level register allocation (Wasmtime's Cranelift works with less info) | ~1.1-1.3x |
| Host function call overhead (crosses WASM-host boundary) | Negligible unless in tight loops |

The bump allocator is actually an advantage for short-lived UDFs — zero GC pauses. But it cannot reclaim memory mid-execution, so long-running or allocation-heavy code will OOM.

**Realistic estimate:** 2-4x slower for pure compute. 1.5-2x for short UDFs doing simple transforms.

### vs. Go Interpreter (e.g., Yaegi) — ~10-50x faster

Tree-walking interpreters reinterpret the AST on every execution, have virtual dispatch per node, and box every intermediate value. Wasmtime JIT eliminates all of this — arithmetic becomes native CPU instructions, control flow becomes native jumps.

**Startup caveat:** Compilation + JIT warmup is slower than an interpreter's first invocation. Module caching amortizes this.

### vs. Custom Bytecode VM (e.g., Tengo) — ~3-10x faster

Even well-written bytecode VMs with computed-goto dispatch spend ~5-15ns per instruction on dispatch overhead vs. ~0.3ns for JIT-compiled native code. The gap is largest for tight loops and numeric computation.

### vs. Optimized WASM (TinyGo/LLVM → WASM) — ~1.5-3x slower

An LLVM-backed compiler emits optimized WASM with inlining, constant propagation, loop optimizations, and better stack usage. Our single-pass compiler produces correct but naive WASM with redundant loads/stores and no inlined functions.

### Summary

| Compared to | Expected Ratio | Primary Cause |
|---|---|---|
| Native Go (`go build`) | 2-5x slower | No optimizations, WASM sandbox overhead |
| Go interpreter (Yaegi) | 10-50x faster | JIT-compiled native code vs. AST walking |
| Custom bytecode VM | 3-10x faster | JIT eliminates dispatch loop |
| Optimized WASM (TinyGo/LLVM) | 1.5-3x slower | No optimization passes |

---

## Optimization Roadmap

### Phase 1: Intermediate Representation

Before any optimization is possible, we need a proper IR between the AST and WASM emission.

#### 1.1 — SSA (Static Single Assignment) IR

Introduce an SSA-based intermediate representation. Every variable is assigned exactly once, making dataflow analysis trivial.

**What it enables:**
- Constant propagation and folding become simple forward passes
- Dead code elimination falls out of use-def chains
- Common subexpression elimination becomes hash-consing of SSA values
- Register-like allocation maps cleanly to WASM locals

**Structure:**

```
Go AST
  │
  ▼
SSA IR  (basic blocks, phi nodes, typed values)
  │
  ▼
Optimization Passes  (in-place rewrites on SSA)
  │
  ▼
WASM Emission  (lower SSA to WASM stack machine)
```

**SSA construction approach:**
- Build a control flow graph (CFG) from the AST
- Each basic block contains a linear sequence of SSA instructions
- Use phi nodes at join points (or block parameters, which are simpler for WASM lowering)
- Every value has a unique name and a single definition site

**Estimated impact:** This is the prerequisite for all other optimizations. On its own it may slightly regress performance (extra compilation time, possible naive lowering), but it unlocks everything below.

#### 1.2 — CFG Construction

Build an explicit control flow graph during SSA construction:
- Basic blocks with predecessor/successor edges
- Dominator tree computation (needed for SSA phi placement)
- Loop detection (needed for loop optimizations)

### Phase 2: Core Optimization Passes

Each pass operates on the SSA IR and can be run in sequence. Order matters — run them in the listed order for best results.

#### 2.1 — Constant Folding & Propagation

Replace operations on known constants with their results at compile time.

**Examples:**
- `x := 2 + 3` → `x := 5`
- `if true { ... } else { ... }` → eliminate the dead branch
- `len("hello")` → `5`
- Propagate constants through phi nodes when all inputs are the same value

**Estimated impact:** 5-15% improvement on typical code. Larger on code with configuration constants.

#### 2.2 — Dead Code Elimination (DCE)

Remove SSA values with no uses. With SSA, this is a single backward pass:
- Mark all side-effecting instructions (calls, stores, returns) as live
- Mark all transitive inputs of live instructions as live
- Delete everything else

**Estimated impact:** 5-10%. Removes code left behind by constant propagation and inlining.

#### 2.3 — Function Inlining

Replace call sites with the body of the callee, substituting arguments.

**Strategy:**
- Inline leaf functions (functions that don't call other functions) under a size threshold (~30 SSA instructions)
- Inline single-call-site functions regardless of size
- Inline hot stdlib functions (e.g., `len`, `cap`, simple `strings` utilities)
- Do NOT inline recursive functions
- Run constant propagation and DCE again after inlining (inlining often exposes new constant folding opportunities)

**Estimated impact:** 20-40% on real workloads. This is the single highest-impact optimization. Function call overhead in WASM is significant because it involves stack frame setup, local allocation, and indirect jumps.

#### 2.4 — Common Subexpression Elimination (CSE)

If two SSA values compute the same operation on the same inputs, reuse the first result.

**Implementation:** Hash-cons SSA values within each basic block (local CSE), then extend to dominated blocks (global CSE) using the dominator tree.

**Estimated impact:** 3-8%. Most impactful on code with repeated field accesses or index computations.

#### 2.5 — Copy Propagation

Replace uses of `x := y` with direct uses of `y`, then remove the copy.

**Estimated impact:** 2-5%. Mostly cleans up artifacts from SSA construction and inlining.

### Phase 3: Memory & Allocation Optimizations

#### 3.1 — Escape Analysis

Determine which allocations can stay on the stack (WASM locals) instead of being heap-allocated via `alloc()`.

**Rules for stack allocation:**
- The value's address is never taken (no `&x` that escapes the function)
- The value is never stored into a heap-allocated structure
- The value is never passed to a function that might retain it
- The value does not outlive the function (not returned, not captured by a closure that escapes)

**Implementation:**
- Build an escape graph: nodes are allocations, edges are "flows to" relationships
- If an allocation only flows to local uses, it can be stack-allocated
- The `stack_alloc_target` hint in the codebase should drive this

**Estimated impact:** 10-30% for struct-heavy code. Eliminates heap bumps and reduces memory pressure. Particularly important since there's no GC to reclaim short-lived heap allocations.

#### 3.2 — Bounds Check Elimination

Remove redundant bounds checks on slice/array access when the index is provably in range.

**Cases to handle:**
- `for i := 0; i < len(s); i++ { s[i] }` — the loop condition guarantees bounds
- Sequential access `s[0]; s[1]; s[2]` after a length check
- After a prior bounds check on the same slice with a >= index

**Estimated impact:** 5-15% on slice-heavy code (sorting, string processing).

#### 3.3 — WASM Local Reuse

After SSA lowering, reuse WASM local slots for SSA values with non-overlapping lifetimes. This is essentially register allocation for the WASM stack machine.

**Why it matters:** Wasmtime's Cranelift does its own regalloc, but giving it fewer locals with better lifetimes produces better native code. Excessive locals cause register spills.

**Implementation:** Compute live ranges of WASM locals, then use a linear-scan or graph-coloring allocator to merge compatible locals.

**Estimated impact:** 5-10%. Helps Cranelift produce tighter native code.

### Phase 4: WASM-Specific Optimizations

#### 4.1 — Instruction Selection

Lower SSA operations to the best available WASM instruction sequences:
- Use `i32.clz`, `i32.ctz`, `i32.popcnt` for bit operations instead of loops
- Use `select` instruction instead of `if-else` for conditional moves
- Use `memory.copy` and `memory.fill` for bulk operations
- Fuse load-operate-store patterns where possible

**Estimated impact:** 3-8%.

#### 4.2 — Stack Scheduling

WASM is a stack machine. The order in which values are pushed matters — poor ordering causes redundant `local.get`/`local.set` pairs.

**Implementation:** Schedule SSA value computations so that producers are immediately followed by their consumers, minimizing temporary locals.

**Estimated impact:** 5-10%.

#### 4.3 — Block Structure Optimization

WASM's structured control flow (block/loop/if) can be laid out to minimize branches:
- Place the fall-through case of an `if` as the more likely branch
- Merge adjacent blocks with unconditional jumps
- Convert simple `if-else` chains into `br_table` when switching on integers

**Estimated impact:** 2-5%.

### Phase 5: Runtime Optimizations

#### 5.1 — Module Caching

Cache the compiled Wasmtime module (not just the WASM bytes) so that repeated invocations skip both Go compilation and JIT warmup.

**Implementation:**

```
Cache Key: hash(go_source + compiler_version + config)
      │
      ▼
Cache Hit? ──yes──► Instantiate cached Module (microseconds)
      │
      no
      │
      ▼
Compile Go → WASM → Wasmtime Module → Cache → Instantiate
```

**Options:**
- **In-memory LRU cache** — `Module` objects keyed by source hash. Fastest, lost on restart.
- **On-disk precompilation** — Use `Module::serialize()` / `Module::deserialize()` to persist compiled native code. Survives restarts.
- **Two-tier** — In-memory for hot UDFs, on-disk for warm UDFs.

**Estimated impact:** 10-100x improvement on invocation latency for cached UDFs. Compilation time dominates for small UDFs — a cached module instantiates in microseconds vs. milliseconds for full compilation.

#### 5.2 — Wasmtime Configuration Tuning

Tune Wasmtime's settings for the UDF workload:
- `Config::cranelift_opt_level(OptLevel::Speed)` — enable Cranelift's aggressive optimizations
- `Config::cranelift_nan_canonicalization(false)` — skip NaN canonicalization if not needed
- `Config::epoch_interruption(true)` — use epoch-based interruption instead of fuel for lower overhead
- `Config::memory_init_cow(true)` — copy-on-write memory initialization for faster instantiation
- Consider `Config::cranelift_opt_level(OptLevel::SpeedAndSize)` if code size matters

**Estimated impact:** 10-30% from Cranelift opt level alone.

#### 5.3 — Memory Pool / Instance Reuse

Instead of creating a new Wasmtime instance per invocation:
- Pre-allocate a pool of instances with memory already initialized
- Reset heap pointer and globals between invocations (the `reset()` function already exists)
- Reuse the instance without re-instantiation

**Estimated impact:** 2-5x improvement on instantiation latency. Most impactful when UDFs are invoked millions of times (e.g., per-row in a query).

#### 5.4 — Arena/Region-Based Allocation

Enhance the bump allocator with region support:
- Allow marking a region boundary and freeing everything after it
- Useful for intermediate allocations within a single UDF (e.g., temporary strings during JSON encoding)
- Still no full GC, but enables partial memory reclamation

**Estimated impact:** Extends the range of UDFs that can run without OOM. Indirect performance benefit from reduced memory pressure.

---

## Suggested Implementation Order

Prioritized by impact-to-effort ratio:

| Priority | Item | Effort | Impact |
|---|---|---|---|
| 1 | Module caching (5.1) | Low | Very High |
| 2 | Wasmtime config tuning (5.2) | Low | Medium-High |
| 3 | Instance reuse pool (5.3) | Medium | High |
| 4 | SSA IR (1.1-1.2) | High | Prerequisite |
| 5 | Function inlining (2.3) | Medium | Very High |
| 6 | Constant folding (2.1) | Low | Medium |
| 7 | Dead code elimination (2.2) | Low | Medium |
| 8 | Escape analysis (3.1) | High | High |
| 9 | Bounds check elimination (3.2) | Medium | Medium |
| 10 | CSE (2.4) | Medium | Low-Medium |
| 11 | WASM local reuse (3.3) | Medium | Medium |
| 12 | Stack scheduling (4.2) | Medium | Medium |
| 13 | Instruction selection (4.1) | Low-Medium | Low-Medium |
| 14 | Block structure optimization (4.3) | Low | Low |
| 15 | Arena allocation (5.4) | Medium | Situational |

**Phase 1-2 target:** Close the gap with native Go to ~1.5-3x (from ~2-5x).
**Phase 3-4 target:** Approach ~1.2-2x for typical UDF workloads.
**Phase 5 target:** Invocation latency under 100μs for cached UDFs.

---

## Measuring Progress

### Benchmarks to Build

1. **Microbenchmarks** — tight loops, arithmetic, string operations, slice manipulation
2. **Allocation benchmarks** — struct creation, map operations, JSON encoding
3. **Real UDF benchmarks** — representative user-defined functions (row transforms, aggregations, validation logic)
4. **Startup benchmarks** — time from Go source to first result (cold) and cached invocation (warm)

### Metrics to Track

- **Throughput**: operations/second for each benchmark
- **Latency**: cold-start (compile + JIT + execute) and warm-start (cached execute)
- **Memory**: peak linear memory usage, allocation count
- **Compile time**: Go → WASM compilation time
- **WASM size**: generated module size in bytes (proxy for code quality)

Compare against:
- Same Go code compiled with `go build` and run natively
- Same Go code compiled with TinyGo targeting WASM
- Equivalent logic in a Go interpreter (Yaegi)
