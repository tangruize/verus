# OpenVMM async / Verus: failure modes and patch handoff

Date: 2026-10-25. [Chinese review copy](OPENVMM_ASYNC_HANDOFF.zh-CN.md).

**Short explanation:** Verus already supports basic `async fn`, postconditions, and `.await`. OpenVMM also creates `async move` blocks inside closures and uses async methods through traits; Rust lowers these into different coroutine, opaque-type, and trait-object representations, some of which the relevant Verus frontend paths reject. These are specific compatibility gaps, not evidence that Verus cannot handle any async code.

**Important boundary:** This research branch contains six compatibility fixes and one **partial, experimental async-block port**. It does **not** establish verification of the complete OpenVMM load/snapshot path. In particular, passing an async block to a generic `F: Future` interface still fails, so the `UnitBuilder::spawn` path below is not yet unblocked.

## 1. Revisions and provenance

| Component | Pinned revision or version |
|---|---|
| Upstream Verus `main`, cloned and pulled before porting | `98699c5926173feb87c589ece22532008f948bd7`, 2026-10-25 |
| Validated code tip, before this documentation commit | `026ffec07aa3fc2efc7e5a2fc5cb3ca8346497cf` |
| Public fork and branch | `tangruize/verus`, `openvmm/async-support-20261025` |
| OpenVMM production source quoted below | `70dde4c65b16de084744b0fa85aeb581f0b5c6cd` |
| Build environment | Linux x86-64, Rust `1.99.1`, Z3 `4.16.0` |
| Built verifier identity | `0.2026.10.25.026ffec`, release |

The three quoted OpenVMM files were compared with the public pinned revision; their Git blob hashes match. Production line numbers below are not the line numbers of the modified verification worktree. Historical experiments also used a compiler identifying itself as `d85f757`; even that revision already contained basic async/await examples, so the old blanket statement "Verus does not support async" was too broad.

## 2. Actual source shapes and diagnostics

### A. A closure returns an async block

Production: [`openvmm/openvmm_core/src/worker/dispatch.rs:1966-1980`](https://github.com/microsoft/openvmm/blob/70dde4c65b16de084744b0fa85aeb581f0b5c6cd/openvmm/openvmm_core/src/worker/dispatch.rs#L1966-L1980):

```rust
let vmtime = state_units
    .add("vmtime")
    .advances_time()
    .spawn(driver_source.simple(), {
        |recv| {
            let mut vmtime = vmtime_keeper;
            async move {
                vmm_core::vmtime_unit::run_vmtime(&mut vmtime, recv).await;
                vmtime
            }
        }
    })
    .unwrap();
```

The diagnostic observed in the archived load-body experiment was:

```text
The verifier does not yet support the following Rust feature: generator types
```

**Cause:** This expression has a compiler-generated coroutine type. Accepting the opaque return of an ordinary `async fn` does not automatically implement the frontend path for a coroutine block nested inside a closure.

**Evidence qualification:** Run `20261012T171603.962126Z-255411` reached this error at verification-worktree `dispatch.rs:1986-1994` only after two earlier closure-coercion surfaces were temporarily omitted from a diagnostic translation. Those omissions were reverted. The run was not a valid full-body proof and produced no verification counts. The earlier report called this `VmTimeKeeper::spawn`; the actual call is the state-unit builder's `UnitBuilder::spawn`.

The distinction matters because the real builder requires a future. Its signature, at [`vmm_core/state_unit/src/lib.rs:979-987`](https://github.com/microsoft/openvmm/blob/70dde4c65b16de084744b0fa85aeb581f0b5c6cd/vmm_core/state_unit/src/lib.rs#L979-L987), is:

```rust
pub fn spawn<F, Fut>(
    self,
    spawner: impl Spawn,
    f: F,
) -> Result<SpawnedUnit<Fut::Output>, NameInUse>
where
    F: FnOnce(Receiver<StateRequest>) -> Fut,
    Fut: 'static + Send + Future,
    Fut::Output: 'static + Send,
```

The experimental patch accepts and checks block bodies, but does not yet supply the coroutine's `Future` implementation to Verus. Section 4 gives a minimized counterexample to any claim that this whole spawn path is fixed.

### B. A native async trait method has an opaque future return

Production: [`vmm_core/state_unit/src/lib.rs:110-148`](https://github.com/microsoft/openvmm/blob/70dde4c65b16de084744b0fa85aeb581f0b5c6cd/vmm_core/state_unit/src/lib.rs#L110-L148). This excerpt omits the other trait methods:

```rust
pub trait StateUnit: InspectMut {
    async fn advance_time(&mut self, _duration: std::time::Duration) -> anyhow::Result<()> {
        Ok(())
    }
}
```

The minimized reproduction on the unpatched current baseline reported:

```text
The verifier does not yet support the following Rust feature: Verus does not yet support Opaque types in trait def
```

**Cause:** A native async trait method exposes its hidden future through an associated opaque return representation. The old patch's exception for async methods was insufficient on current main: the new inner-return extraction also needed to resolve this projection before looking for `Future::Output`. Commit `fd59818a` handles both paths, without enabling arbitrary non-async opaque trait methods.

### C. `async_trait(?Send)` produces a projected dynamic future

Production: [`openvmm/openvmm_core/src/partition.rs:433-440`](https://github.com/microsoft/openvmm/blob/70dde4c65b16de084744b0fa85aeb581f0b5c6cd/openvmm/openvmm_core/src/partition.rs#L433-L440):

```rust
#[async_trait(?Send)]
pub trait HvliteVp {
    async fn run(
        &mut self,
        runner: VpRunner,
        chipset: &vmm_core::vmotherboard_adapter::AdaptedChipset,
    );
}
```

The relevant expansion shape is `Pin<Box<dyn Future<Output = ()> + 'async_trait>>`. This is a type-shape sketch, not a complete macro expansion.

The unpatched frontend reported the following diagnostic; "that" is the original diagnostic's wording:

```text
The verifier does not yet support the following Rust feature: dyn with more that one trait
```

**Cause:** Although the source has one principal trait, Rust represents both `Future` and the associated-type equation `Output = ()` as predicates. The frontend rejected the extra predicate. Commit `47ee0391` preserves the output type in the dynamic type, its associated-type bounds, and the corresponding SMT constructor arity. Porting to current main also required consulting imported trait compatibility metadata and retaining associated declarations during pruning.

This supports the projected dynamic type and its output-type relation; it is not a specification of `Pin`, polling, executors, or cancellation. `dyn Future<Output = T> + Send` and `dyn Future<Output = T> + Send + Sync` remain outside this patch's accepted combinations. The separate `dyn Trait + Send + Sync` fix does not imply those combinations are supported.

### D. Nearby failures that can prevent reaching the async obligation

| Trigger | Observed failure | Repair |
|---|---|---|
| Function-local enum with `#[derive(Debug)]`; the derive produces nested `impl` items | `The verifier does not yet support the following Rust feature: internal item statements` | Accept the derived implementation items while still checking the enclosing body. |
| A macro expands to an immutable function-local `static` | `Internal Verus Error: The thir_body query is running for item ... which may require erasure, but the VerusErasureCtxt has not been initialized.` Item identity is elided here. | Arrange erasure information before the static's THIR query, translate the static item, and use the implicit exec mode only for macro-generated statics. Initializers remain checked. |
| Focused async verification with `#![no_std]` and allocation-only `vstd` | Panic at `vir/src/prune.rs:466:46`: `no entry found for key` | Reach async helpers only when present, and make the `core::future`-based vstd module available without `std`. |
| An ordinary principal trait object carrying both `Send` and `Sync` | `dyn with more that one trait` | Recognize exactly that combination rather than dropping arbitrary predicates. Dynamic method contracts remain checked. |

The static case models the expansion surface used by OpenVMM tracing macros, for example `tracing::info_span!` at production `dispatch.rs:1682`. The regression uses a small macro-generated static rather than importing the entire tracing dependency tree; it is not a claim that every tracing expansion now verifies.

The VP construction at production `dispatch.rs:3389-3422` further composes iterator closures, `vp.run(runner, &chipset)`, and another `async move` block. It illustrates why solving one frontend rejection is not equivalent to verifying the complete operation.

## 3. Problems already fixed upstream, not duplicated here

| Historical problem | Upstream fix | Status on this branch |
|---|---|---|
| An unnamed async return retained the future's type where postconditions needed its output type | [verus-lang/verus#3015](https://github.com/verus-lang/verus/pull/3015), `ce82df44ac2b4a76c22c01c22baf056c876bdec3`; merged 2026-10-07 19:31:16 UTC, or 2026-10-08 03:31:16 UTC+8 | Already in the pulled main; no duplicate patch. |
| A cast became an identity after dynamic coercion/VIR lowering | [verus-lang/verus#2951](https://github.com/verus-lang/verus/pull/2951), `1814c7d5472e4640371aeb8e976dd88008941b69`; 2026-09-18 | Already upstream, including its regression. |

For the unnamed-return bug specifically: **yes, it was fixed upstream**. Verus states async postconditions as facts about the future once it has been awaited. The old unnamed-return path used the future's own type where the completed result's type was required, so those facts did not match the facts supplied by `.await`. The upstream refactor separates `outer_ret` (future) from `inner_ret` (output) and recovers the latter independently of an optional named-return binder.

## 4. Functional commit stack and the remaining async-block gap

Commits are ordered from the pulled upstream baseline. They can be reviewed independently; apply dependent commits in this order.

| Commit | Functionality |
|---|---|
| `f1a74a76` | Derived implementations on function-local items. |
| `fd59818a` | Opaque future returns from native async trait methods, including current main's inner-return extraction. |
| `51323089` | Immutable macro-generated function-local statics, including initializer verification. |
| `e1efcd0e` | Async helper pruning and allocation-only/no_std Future specifications; a real Cargo focus regression containing `.await`. |
| `8cc25ca8` | Principal trait objects carrying exactly `Send + Sync`, with method-contract checks preserved. |
| `47ee0391` | `dyn Future<Output = T>` type/projection support, imported-trait checks, and associated-type retention. |
| `026ffec0` | **Experimental and partial:** async-block construction and block-body checking, including a simple direct `.await`; generic Future-bound calls still rejected. |

To use only the first six compatibility fixes, select commit `47ee0391` instead of the branch tip. The seventh commit is intentionally separate.

This minimized example **still fails** with the full stack:

```rust
use core::future::Future;
use vstd::prelude::*;

verus! {
    fn retain_future<F: Future<Output = usize>>(future: F) -> F {
        future
    }

    async fn test() {
        let future = retain_future(async { 7usize });
        let _value: usize = future.await;
    }
}
```

The error is at `retain_future(...)`:

```text
Verus does not recognize this trait bound: <{async block@...} as std::future::Future>
```

Here the generated source location is elided. The compiler-generated `Future` implementation is still unhandled by `get_impl_paths_for_clauses`, and the legacy port reuses closure body checking rather than providing a complete coroutine/Future encoding. Merely adding `Future` to a do-nothing allowlist would not establish the missing proof relationships; this branch does not do that.

The regression also establishes that a simple `(async { 7usize }).await` is accepted, and that `assert(false)` both inside a block and after that await is rejected as an actual proof failure. That is **not** evidence of general async-block postcondition transport. Native async-function contracts and projected dynamic output types are distinct from this remaining gap.

For a source-preserving completion of the OpenVMM path, the remaining work includes modeling the coroutine's Future bound and output relation, connecting block completion to its contracts, and then revisiting the actual spawn/coercion and executor-facing interfaces. Do not replace the production spawn with an unverified helper or omit it from the verification translation and call the result a proof of the original body.

## 5. Obtain and build the branch

These Linux commands use checkout-local build products rather than replacing globally installed Verus/Cargo executables. See [upstream build instructions](BUILD.md) for platform-specific prerequisites.

```bash
git clone --single-branch --branch openvmm/async-support-20261025 \
  https://github.com/tangruize/verus.git verus-openvmm
cd verus-openvmm/source

rustup toolchain install
./tools/get-z3.sh
./z3 --version

export CARGO_BUILD_JOBS=4
export VERUS_Z3_PATH="$PWD/z3"
cargo build --locked --release \
  -p rust_verify -p verus -p cargo-verus \
  -p verus_builtin -p verus_builtin_macros -p verus_state_machines_macros
export PATH="$PWD/target-verus/release:$PATH"
cargo verus build --manifest-path vstd/Cargo.toml --release
./target-verus/release/verus --version
```

The pinned checkout requests Rust `1.99.1`; the solver must be Z3 `4.16.0`. When rebuilding after verifier source changes, explicitly rebuild `rust_verify`; recompiling the test harness alone does not rebuild the verifier it launches.

Run the focused checks from `source`:

```bash
cargo test --locked --release -p rust_verify_test \
  --test async_functions --test consts --test expr_stmts \
  --test traits_dyn --test external_traits --test exec_closures \
  --test mut_refs_closures --test opaque_types \
  --no-fail-fast -- --test-threads=4

cargo test --locked --release -p rust_verify_test --test cargo \
  verified_async_function_filter -- --exact --nocapture
```

No proof-skipping or lifetime-check-disabling flags are required for these results.

## 6. Validation and interpretation

The code tip above rebuilt successfully, and vstd reported **2102 verified, 0 errors**. The focused results were:

| Suite | Passed | Existing ignored tests |
|---|---:|---:|
| `async_functions` | 28 | 0 |
| `consts` | 59 | 2 |
| `exec_closures` | 111 | 1 |
| `expr_stmts` | 4 | 2 |
| `external_traits` | 43 | 0 |
| `mut_refs_closures` | 17 | 0 |
| `opaque_types` | 49 | 0 |
| `traits_dyn` | 40 | 0 |
| Cargo `verified_async_function_filter` | 1 | 0 |

These include genuine assertion/postcondition failures as negative controls, not merely failures caused by an unsupported syntax error. The Cargo case uses `#![no_std]`, `vstd` with only its `alloc` feature, focused function selection, and an actual await. The async-block Future-bound rejection is explicitly recorded as a remaining limitation, not counted as successful support for that operation.

These results are compatibility/regression evidence, not a soundness audit or a proof of OpenVMM snapshot restoration, scheduling, polling, cancellation, or the complete `InitializedVm::load` body. No live experiment installation was changed by this port. The branch contains source patches, focused regressions, and this handoff, not experiment transcripts, binaries, solver archives, or presentation material.

The port and handoff were prepared with GitHub Copilot assistance, disclosed in the functional commits. This is a research fork, not an upstream-approved release.
