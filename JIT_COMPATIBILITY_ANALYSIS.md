# JIT Compatibility Analysis for govm Bytecode

## Architecture Summary

The govm VM is a **stack-based interpreter** with:
- 126 opcodes, variable-width encoding (1–5 bytes per instruction)
- Tagged pointer object representation (`Object(*mut u8)` with 8 tag bits)
- Dynamic typing at the bytecode level (type resolved via tag bits at runtime)
- Async execution loop (Tokio-based, for goroutines/channels)
- Boehm GC for memory management

Both **Cranelift** and **LLVM** use register-based SSA (Static Single Assignment) IRs. The question is how cleanly the bytecode maps to them.

---

## Tier 1: Straightforward to Lower (no real blockers)

These opcodes have direct or near-direct equivalents in both Cranelift IR and LLVM IR:

| Category | Opcodes | Notes |
|---|---|---|
| Arithmetic | `Add`, `Subtract`, `Multiply`, `Divide`, `Modulo`, `Negate` | Maps to `iadd`, `isub`, `imul`, `sdiv`, `srem` etc. — **once types are known** |
| Comparisons | `Gt`, `Gte`, `Lt`, `Lte`, `Eq`, `Neq` | Maps to `icmp`/`fcmp` variants |
| Boolean logic | `And`, `Or`, `Not` | Maps to `band`, `bor`, `bnot` |
| Control flow | `Jump`, `JumpIfFalse` | Maps to basic blocks + `br`/`brif` |
| Constants | `Const`, `True`, `False`, `Null` | Immediate values |
| Locals | `GetLocal`, `SetLocal`, `CopyLL`, `SwapLL` | Stack slots or SSA variables |
| Globals | `GetGlobal`, `SetGlobal` | Global value loads/stores |
| Type casts | `CastToFloat32`, `CastToFloat64` | `fcvt` instructions |
| Stack ops | `Pop` | No-op in SSA (just don't use the value) |
| Fused ops | `AddLocalConst`, `GtLocalConst`, etc. | Decompose into load + op — actually beneficial for SSA |

The **stack-to-SSA conversion** is the fundamental transformation needed. This is a well-solved problem: you simulate the stack abstractly per basic block, each stack position becomes an SSA value, and at control-flow merge points you insert phi nodes. Both Cranelift (`Block` parameters) and LLVM (`phi` instructions) support this natively.

---

## Tier 2: Moderate Effort (needs runtime support but feasible)

| Feature | Opcodes | Challenge |
|---|---|---|
| Function calls | `Call`, `ReturnValue`, `Return` | Need to define a calling convention. Both backends support custom call ABIs. The frame management (bp/ip saving) maps to standard call frame management. |
| Closures | `GetCaptured`, `SetCaptured` | The closure environment (`Vec<Object>`) becomes an extra pointer argument to the JIT-compiled function. Access is a struct field load. Standard approach. |
| Builtins | `CallBuiltin` | Emit a native call to a Rust runtime function. Both Cranelift and LLVM trivially support calling external functions. |
| Data structures | `MakeArray`, `MakeSlice`, `Map`, `Struct`, `IndexGet`, `IndexSet` | These should be **runtime calls** (not inlined into JIT code). The JIT emits `call @runtime_make_array(...)` etc. |
| Pointers | `Ref`, `Deref`, `LocalPtrWrite`, `GlobalPtrWrite` | Map to load/store through indirection. Both IRs handle this natively. |
| Interfaces | `Upcast`, `Downcast`, `DynamicDispatch` | Virtual dispatch tables. Emit an indirect call through a vtable pointer. Standard pattern in both backends. |
| Iteration | `Range`, `IntoIter` | Runtime calls that return iterator objects. |

---

## Tier 3: Hard Problems (architectural friction)

These are the areas where the current design creates genuine tension with JIT compilation:

### 1. The Async Execution Model (biggest obstacle)

The entire execution loop is `async`. Channel operations use `.await` (e.g., `ch.sender.send(value).await`).

**Problem:** JIT-generated native code cannot `.await`. Cranelift and LLVM produce synchronous machine code. You cannot yield to the Tokio runtime from JIT-compiled functions.

**Solutions (pick one):**

- **A) Hybrid approach:** Only JIT-compile "pure" functions (no channel ops, no goroutine spawning). Keep the interpreter for async operations. When JIT code needs to call a function that uses channels, it calls back into the interpreter. This is the most pragmatic option.
- **B) Blocking channels in JIT code:** Replace async channels with OS-level blocking primitives (e.g., `crossbeam-channel`) for JIT-compiled code, and use OS threads (or stackful coroutines) instead of Tokio tasks for goroutines. This changes the concurrency model significantly.
- **C) Yield points / on-stack replacement (OSR):** At channel operations, save the JIT execution state and return to the interpreter/scheduler. When the channel operation completes, re-enter JIT code. This is complex but is how some production VMs handle it (e.g., Go's own runtime, GraalVM).

### 2. Dynamic Typing & Tagged Pointers

Every arithmetic operation in the VM does a runtime type check via `match left.tag()`. Without **type specialization**, JIT-compiling this would just produce native code that does the same tag-check branches as the interpreter. You'd remove the dispatch loop overhead (~20–30% speedup), but miss the real win of JIT: eliminating type checks entirely.

**Solutions:**

- **Type inference at compile time:** Since the compiler already knows Go types, propagate them into the bytecode so the JIT knows "this Add is always i64 + i64."
- **Speculative specialization:** JIT for the common type, add a guard (type check), and deoptimize (fall back to interpreter) if the guard fails. This is what V8/SpiderMonkey do.
- **Typed opcodes:** Instead of one `Add` that handles 12+ types, emit `AddInt64`, `AddFloat64`, etc. from the compiler. This is the simplest option given there are already `AddLocalConst` fused ops.

### 3. Defer / Panic / Recover

This is essentially Go's exception handling model. In JIT code:

- **LLVM** has mature support for this via landing pads and personality functions (same mechanism used for C++ exceptions and Go's own panic/recover in the real Go compiler).
- **Cranelift** has *limited* exception support. It doesn't natively support landing pads, but defer can be modeled as "register a cleanup function on a side stack" (which is basically what the VM already does with `self.deferred`). The JIT code would emit calls to runtime helpers: `runtime_push_defer(func, args)` and `runtime_execute_deferred()` at return points.

### 4. Goroutine Spawning

`GoSpawn` would become a runtime call from JIT code: `runtime_go_spawn(ip, args, num_locals)`. The spawned goroutine can still run in the interpreter (or in JIT if the target function is also compiled). This is manageable.

### 5. GC Compatibility

The VM uses Boehm GC, which is a conservative collector — it scans the stack for anything that looks like a pointer. This actually works **in favor of JIT**: Boehm GC will conservatively scan the native stack frames produced by JIT code and find GC-managed pointers. No special GC maps or safepoints are needed (unlike precise GC). This is one less thing to worry about.

---

## Cranelift vs. LLVM: Which to Choose?

| Dimension | Cranelift | LLVM |
|---|---|---|
| **Compilation speed** | Very fast (designed for JIT) | Slower (designed for AOT) |
| **Code quality** | Good, not great | Excellent optimization |
| **Rust integration** | Native Rust, first-class crate | Via `inkwell` or `llvm-sys` bindings |
| **Exception handling** | Limited (no landing pads) | Full support (landing pads, personality functions) |
| **Maturity** | Younger, actively developed | Very mature |
| **Complexity** | Simpler API | Complex API, larger dependency |
| **Binary size** | Small | Large (links LLVM libs) |
| **Best for** | Fast startup, moderate optimization | Maximum throughput |

### Recommendation: Cranelift

1. It's pure Rust — no C++ dependency, no LLVM build complexity.
2. Its fast compilation is critical for JIT (you don't want a 100ms pause to compile a function).
3. The `cranelift-jit` crate gives you an in-process JIT with minimal setup.
4. Defer/panic/recover can be modeled as runtime calls rather than needing LLVM's exception machinery.
5. It's used by Wasmtime (production-quality) and is actively maintained by the Bytecode Alliance.

LLVM would only make sense if maximum optimization of hot loops (e.g., numeric computation) is needed, and you're willing to accept much slower JIT compilation time and the build complexity.

---

## Recommended Architecture

```
┌─────────────────────────────────────────────┐
│              Go Source Code                  │
└──────────────────┬──────────────────────────┘
                   │ (existing compiler)
                   ▼
┌─────────────────────────────────────────────┐
│         Typed Bytecode (enhanced)            │
│  - Add type annotations to opcodes          │
│  - Mark "pure" vs "async" functions         │
└──────────┬──────────────────┬───────────────┘
           │                  │
     pure functions     async functions
           │                  │
           ▼                  ▼
┌──────────────────┐  ┌──────────────────┐
│  Cranelift JIT   │  │   Interpreter    │
│  (native code)   │  │   (as today)     │
└──────────────────┘  └──────────────────┘
           │                  │
           └──────┬───────────┘
                  ▼
         ┌────────────────┐
         │    Runtime     │
         │  - GC (Boehm)  │
         │  - Channels    │
         │  - Goroutines  │
         │  - Builtins    │
         └────────────────┘
```

---

## Concrete Steps to Get There

1. **Add type information to bytecode.** Since the compiler already knows Go types, annotate arithmetic/comparison opcodes with concrete types. This is the single highest-impact change — without it, JIT gains are marginal (~20–30% from removing dispatch overhead) rather than transformative (2–10x from eliminating type checks).

2. **Implement stack-to-SSA translation.** For each function's bytecode, simulate the stack abstractly and produce Cranelift IR. The `cranelift-frontend` crate's `FunctionBuilder` makes this straightforward.

3. **Start with the "easy" opcodes.** Get arithmetic, comparisons, locals, globals, jumps, and simple function calls working first.

4. **Model complex operations as runtime calls.** `MakeArray`, `IndexGet`, `Defer`, `GoSpawn`, `ChanSend`, etc. all become `call` instructions to Rust runtime functions exposed via `cranelift-module`.

5. **Use a tiered approach.** Interpret everything first, profile which functions are hot, then JIT-compile only those. Functions using channels/select stay interpreted (or get the hybrid treatment).

---

## Verdict

**Yes, the bytecode is compatible with JIT compilation via Cranelift (or LLVM), but with caveats:**

- The **arithmetic, control flow, locals/globals, function calls, and closures** map cleanly and would give real speedups.
- The **async execution model** (channels, select, goroutine spawning) is the primary architectural obstacle. A hybrid interpreter+JIT approach is the pragmatic solution.
- **Type specialization** is essential to unlock the full performance benefit of JIT. Without it, JIT only removes dispatch overhead. With it, tight native loops can be generated for numeric code.
- **Defer/panic/recover** is manageable via runtime calls, especially with Cranelift.
- **Boehm GC** is actually a good fit for JIT since it's conservative and doesn't need stack maps.

The overall effort would be moderate-to-large — adding a new `src/vm/jit/` module with the Cranelift translation layer, plus modifications to the compiler to emit type information. The interpreter would remain as a fallback for complex async operations.
