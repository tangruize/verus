# OpenVMM async / Verus：故障原因与补丁交接

日期：2026-10-25。[英文权威版本](OPENVMM_ASYNC_HANDOFF.md)；本文件是便于快速审阅的中文镜像。

**几句话说明原因：** Verus 已经支持基本的 `async fn`、后置条件和 `.await`。但 OpenVMM 还会在闭包里创建 `async move` 块，并通过 trait 使用异步方法；Rust 会把这些结构降低为不同的 coroutine、opaque type 和 trait object 表示，而 Verus 的部分前端路径还不认识它们。因此，这是特定组合的兼容性缺口，不是“Verus 完全不支持 async”。

**最重要的边界：** 本研究分支包含六项兼容修复，以及一个**局部、实验性的 async-block 补丁**，不代表 OpenVMM 的完整加载或快照路径已经验证成功。尤其是，把 async block 传给泛型 `F: Future` 接口仍然会失败，因此下面的 `UnitBuilder::spawn` 路径还没有打通。

## 1. 版本与证据来源

| 对象 | 固定版本 |
|---|---|
| 移植前新 clone 并 pull 的上游 Verus `main` | `98699c5926173feb87c589ece22532008f948bd7`，2026-10-25 |
| 本文档提交之前，实际完成验证的代码 tip | `026ffec07aa3fc2efc7e5a2fc5cb3ca8346497cf` |
| 公开 fork 与分支 | `tangruize/verus`，`openvmm/async-support-20261025` |
| 下文引用的 OpenVMM production 源码 | `70dde4c65b16de084744b0fa85aeb581f0b5c6cd` |
| 构建环境 | Linux x86-64，Rust `1.99.1`，Z3 `4.16.0` |
| 构建出的 verifier 标识 | `0.2026.10.25.026ffec`，release |

引用的三个 OpenVMM 文件已与公开固定版本逐个比较，Git blob hash 一致。下文 production 行号不是经过修改的 verification worktree 行号。历史实验还使用过标识为 `d85f757` 的编译器；该版本本身已经包含基本 async/await 示例，所以历史报告中“Verus 不支持 async”的笼统结论不准确。

## 2. 真实源码结构与报错

### A. 闭包返回 async block

Production：[`openvmm/openvmm_core/src/worker/dispatch.rs:1966–1980`](https://github.com/microsoft/openvmm/blob/70dde4c65b16de084744b0fa85aeb581f0b5c6cd/openvmm/openvmm_core/src/worker/dispatch.rs#L1966-L1980)：

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

归档的 load-body 实验报错：

```text
The verifier does not yet support the following Rust feature: generator types
```

**原因：** 这里产生的是编译器生成的 coroutine 类型。支持普通 `async fn` 的 opaque 返回类型，并不意味着已经实现“闭包里面的 coroutine block”对应的前端翻译路径。

**证据范围：** `20261012T171603.962126Z-255411` 这次运行先在诊断用翻译中临时移除了两个更早的闭包转换阻塞点，才在 verification worktree 的 `dispatch.rs:1986–1994` 到达此错误。临时修改随后已恢复。它不是有效的完整函数体证明，也没有产生 verification counts。旧报告把这个调用写成 `VmTimeKeeper::spawn`；准确地说，它是 state-unit builder 的 `UnitBuilder::spawn`。

这个区别很重要，因为实际 builder 明确要求一个 Future。[`vmm_core/state_unit/src/lib.rs:979–987`](https://github.com/microsoft/openvmm/blob/70dde4c65b16de084744b0fa85aeb581f0b5c6cd/vmm_core/state_unit/src/lib.rs#L979-L987) 的签名是：

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

实验性补丁可以接收并检查 async block 的块体，但尚未把该 coroutine 的 `Future` 实现提供给 Verus。第 4 节给出一个仍然失败的最小例子，说明不能声称这条完整 spawn 路径已经修好。

### B. 原生 async trait 方法的返回值是 opaque future

Production：[`vmm_core/state_unit/src/lib.rs:110–148`](https://github.com/microsoft/openvmm/blob/70dde4c65b16de084744b0fa85aeb581f0b5c6cd/vmm_core/state_unit/src/lib.rs#L110-L148)。下面摘录省略了其他 trait 方法：

```rust
pub trait StateUnit: InspectMut {
    async fn advance_time(&mut self, _duration: std::time::Duration) -> anyhow::Result<()> {
        Ok(())
    }
}
```

在未打补丁的当前基线上，最小复现报错：

```text
The verifier does not yet support the following Rust feature: Verus does not yet support Opaque types in trait def
```

**原因：** 原生 async trait 方法通过关联的 opaque 返回表示暴露隐藏的 future。旧补丁仅允许 async 方法进入该路径，在新 main 上还不够：新版提取内部返回类型的代码也需要先解析这个 projection，再寻找 `Future::Output`。`fd59818a` 同时处理了这两条路径，没有顺便放开任意非 async 的 opaque trait 方法。

### C. `async_trait(?Send)` 生成带 Output projection 的动态 Future

Production：[`openvmm/openvmm_core/src/partition.rs:433–440`](https://github.com/microsoft/openvmm/blob/70dde4c65b16de084744b0fa85aeb581f0b5c6cd/openvmm/openvmm_core/src/partition.rs#L433-L440)：

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

相关展开类型的形状是 `Pin<Box<dyn Future<Output = ()> + 'async_trait>>`。这里只展示类型结构，不是完整宏展开。

未修复前的报错如下；其中 `that` 是原始诊断文本的用词：

```text
The verifier does not yet support the following Rust feature: dyn with more that one trait
```

**原因：** 源码虽然只有一个 principal trait，但 Rust 会分别表示 `Future` 和关联类型等式 `Output = ()`，因此出现多个 predicate。旧前端直接拒绝额外的 predicate。`47ee0391` 保留了动态类型里的 Output、关联类型约束以及 SMT 构造器的参数个数；移植到当前 main 时，还补齐了导入 trait 的兼容性查询和 pruning 中关联类型声明的保留。

这支持的是动态类型及其输出类型关系，不是 `Pin`、poll、executor 或 cancellation 的完整规格。`dyn Future<Output = T> + Send` 和 `dyn Future<Output = T> + Send + Sync` 仍不属于本补丁接受的组合。另一个针对普通 `dyn Trait + Send + Sync` 的补丁，并不意味着这两种 Future 组合也已支持。

### D. 其他会阻止验证到达 async obligation 的问题

| 触发结构 | 观察到的失败 | 修复方向 |
|---|---|---|
| 函数内 `#[derive(Debug)]` 的 enum；derive 产生嵌套 `impl` item | `The verifier does not yet support the following Rust feature: internal item statements` | 接收这些派生实现，同时继续检查外层函数体。 |
| 宏展开产生函数内不可变 `static` | `Internal Verus Error: The thir_body query is running for item ... which may require erasure, but the VerusErasureCtxt has not been initialized.` 此处省略 item 标识。 | 在 THIR 查询之前准备 erasure 信息、翻译 static item，并且只给宏生成的 static 使用隐式 exec mode；初始化表达式仍然验证。 |
| `#![no_std]`、仅开启 `vstd` 的 alloc feature，并选择 async 函数做 focused 验证 | `vir/src/prune.rs:466:46` panic：`no entry found for key` | 只访问实际存在的 async helper，并让基于 `core::future` 的 vstd 模块在无 std 时可用。 |
| 普通 principal trait object 同时携带 `Send + Sync` | `dyn with more that one trait` | 精确识别这一组合，不丢掉任意 predicate；动态方法契约仍然检查。 |

static 的例子对应 OpenVMM tracing 宏会使用的展开结构，例如 production `dispatch.rs:1682` 的 `tracing::info_span!`。回归使用一个小型宏生成的 static，没有导入整个 tracing 依赖树，因此不能据此宣称所有 tracing 宏展开均已验证成功。

Production `dispatch.rs:3389–3422` 的 VP 构造还组合了 iterator closure、`vp.run(runner, &chipset)` 和另一个 `async move`。这也说明，消除一个前端报错不等于验证了完整操作。

## 3. 上游已经修复的问题：这里不重复打补丁

| 历史问题 | 上游修复 | 本分支状态 |
|---|---|---|
| async 返回值未命名时，本应使用输出类型的后置条件保留了 Future 自身的类型 | [verus-lang/verus#3015](https://github.com/verus-lang/verus/pull/3015)，`ce82df44ac2b4a76c22c01c22baf056c876bdec3`；2026-10-07 19:31:16 UTC 合并，即 UTC+8 的 2026-10-08 03:31:16 | pull 下来的 main 已包含，没有重复补丁。 |
| dyn coercion / VIR lowering 后成为恒等转换的 cast 仍被拒绝 | [verus-lang/verus#2951](https://github.com/verus-lang/verus/pull/2951)，`1814c7d5472e4640371aeb8e976dd88008941b69`；2026-09-18 | 实现及回归均已在上游。 |

对于你特别问的 unnamed-return bug：**是的，上游已经修过。** Verus 把 async 后置条件表述为“这个 future 被 await 完成后成立的事实”。旧的未命名返回路径在需要结果类型的位置使用了 future 自身的类型，导致这些事实无法与 `.await` 提供的事实匹配。上游重构区分了 `outer_ret`（future）和 `inner_ret`（输出），不再依赖可选的命名返回 binder 来恢复输出类型。

## 4. 按功能拆分的提交，以及 async block 剩余缺口

以下按 pulled main 之后的顺序排列。每个提交可以独立审阅；实际使用以整个补丁栈的顺序为准。

| Commit | 功能 |
|---|---|
| `f1a74a76` | 函数内 item 的派生实现。 |
| `fd59818a` | 原生 async trait 方法的 opaque future 返回，包括新版 inner-return 提取路径。 |
| `51323089` | 宏生成的函数内不可变 static，并保留 initializer 验证。 |
| `e1efcd0e` | async helper pruning 与 alloc-only/no_std 的 Future 规格；包含实际 `.await` 调用的 Cargo focus 回归。 |
| `8cc25ca8` | 普通 principal trait object 上精确的 `Send + Sync` 组合，并保留方法契约检查。 |
| `47ee0391` | `dyn Future<Output = T>` 类型与 projection，导入 trait 的检查，以及关联类型声明保留。 |
| `026ffec0` | **实验性、局部支持：** async block 构造和块体检查，包括简单的直接 `.await`；泛型 Future-bound 调用仍会被拒绝。 |

只想使用前六项兼容修复时，选择 `47ee0391`，而不是分支 tip。第七项刻意保持为独立提交。

即使使用全部补丁，下面的最小例子**仍然失败**：

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

失败点是 `retain_future(...)`：

```text
Verus does not recognize this trait bound: <{async block@...} as std::future::Future>
```

这里省略了自动生成的源码位置。`get_impl_paths_for_clauses` 仍未处理编译器为 coroutine 生成的 `Future` 实现；旧方案复用了 closure 的块体检查，没有提供完整的 coroutine/Future 编码。简单地把 `Future` 加到“不必处理”的白名单里，并不能建立缺失的证明关系，本分支没有这样做。

回归还确认：简单的 `(async { 7usize }).await` 可以被接收；在块内、以及这次 await 之后写 `assert(false)`，都会产生真实的证明失败。但这**不能证明一般的 async-block 后置条件传递已经建立**。原生 async 函数契约、动态 Future 的 Output 类型关系，与这里尚未解决的缺口是不同的事情。

要保持生产源码语义并继续打通 OpenVMM，还需要建模 coroutine 的 Future bound 和输出关系，把块的完成与契约连接起来，再回到真实的 spawn/coercion 和 executor 接口。不能把 production spawn 改成未验证的 helper，或从验证翻译中移除后，宣称证明了原函数体。

## 5. 获取与构建

以下 Linux 命令只使用 checkout 内的构建产物，不覆盖全局安装的 Verus/Cargo 可执行文件。平台依赖可参考[上游构建说明](BUILD.md)。

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

固定 checkout 要求 Rust `1.99.1`，solver 必须是 Z3 `4.16.0`。修改 verifier 源码后，需要显式重建 `rust_verify`；只重编测试 harness，不会重建它启动的 verifier。

在 `source` 下运行定向检查：

```bash
cargo test --locked --release -p rust_verify_test \
  --test async_functions --test consts --test expr_stmts \
  --test traits_dyn --test external_traits --test exec_closures \
  --test mut_refs_closures --test opaque_types \
  --no-fail-fast -- --test-threads=4

cargo test --locked --release -p rust_verify_test --test cargo \
  verified_async_function_filter -- --exact --nocapture
```

这些结果不需要跳过证明或关闭 lifetime 检查。

## 6. 验证范围与结论边界

上述代码 tip 已重新构建，vstd 结果为 **2102 verified，0 errors**。定向结果如下：

| Suite | 通过 | 原有 ignored 测试 |
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

其中包含实际 assertion/postcondition 失败的负对照，不是仅仅因为语法不支持而失败。Cargo 用例使用 `#![no_std]`、仅 alloc 的 vstd、focused function selection，并包含实际的 `.await` 表达式。async-block Future-bound 的拒绝明确记录为剩余限制，不算该操作获得支持。

这些是兼容性与回归证据，不是 soundness audit，也不是 OpenVMM 快照恢复、调度、polling、cancellation 或完整 `InitializedVm::load` 函数体的证明。此次移植没有修改任何运行中实验的安装。分支只包含源码补丁、定向回归和交接文档，不包含实验会话、二进制、solver 压缩包或演示材料。

移植及文档使用了 GitHub Copilot 辅助，功能提交中有明确披露。这是研究 fork，不是已经得到上游认可的正式发布。
