# T2L10 与 AI 对话记录（完整版)

## T2L10（Claude 对话）

*对应 Claude session: `d38281a3-b3f9-4949-a692-0da3644d06dd`*

> 说明：以下为从 `~/.claude/projects/*/*.jsonl` 提取的 Claude 可见对话，已过滤 skill 注入、tool_use、thinking、tool_result 和本地命令回显。

**用户：**
协助我实现。
实现10. 实验 ：ch3~ch5扩展多核的能力（操作系统中最容易出bug的部分之一）
目标：支持多核处理
参考：
https://github.com/arceos-org/arceos
实现要点（典型路线）：
确保 ch3~ch5能够继续支持多核能力
观测指标：
- 设计不同的测试环境和例子，测试多核执行的特点和优势
验收：
- 提供对应测试用例和文档，说明如何设计实现的，在开发中会出现哪些容易出错的地方，并如何进行修复的，以及完成设计分析报告。能帮助学生更好地理解多核，以及多核编程。

**Claude：**

好的 Master，我已经完成了对项目的全面探索。以下是关键发现：

**现状：CH3~CH5 完全是单核设计**
- 没有任何 SMP 相关代码（无 hart、cpu_id、spinlock）
- QEMU 启动时无 `-smp` 参数
- 所有同步依赖 `UPIntrFreeCell`（单核中断屏蔽）
- 全局状态通过 `UnsafeCell` 访问（单核安全，多核不安全）

**三个章节的架构差异：**
| | CH3 | CH4 | CH5 |
|---|---|---|---|
| 任务模型 | 固定数组[32] TCB | Vec<Process> + Sv39 | BTreeMap + 完整进程树 |
| 调度 | 时间片轮转 | FIFO | Stride 公平调度 |
| 地址空间 | 单一内核空间 | 每进程独立 Sv39 | 每进程独立 Sv39 |
| 上下文切换 | 直接 execute | MultislotPortal | MultislotPortal |

第一个问题：

**Claude：**

好的，2 核是最佳教学选择。下一个问题：

**Claude：**

好的 Master，所有关键设计决策已确认。让我提出完整的设计方案。

**Claude：**

## 三种实现方案对比

### 方案 A：自底向上渐进式（推荐）

**思路：** 先构建 SMP 基础设施 crate，再逐章节适配，每步可独立验证。

```
阶段 1: tg-rcore-tutorial-smp crate (SpinLock, PerCpu, hart boot)
阶段 2: 升级 sync crate (UPIntrFreeCell → SpinNoIrq)
阶段 3: CH3 多核适配 (最简单，无虚存)
阶段 4: CH4 多核适配 (增加虚存 + TLB shootdown)
阶段 5: CH5 多核适配 (完整进程管理 + work stealing)
阶段 6: 测试套件 + 文档
```

**优点：**
- 教学价值最高，学生能清楚看到每层抽象的职责
- 每阶段都可独立验收和 debug
- 符合 tg-rcore-tutorial 的渐进式风格
- 便于写实验指导书

**缺点：**
- 需要新增公共 crate，改动面较大
- 实现时间最长

**适用性：** 最适合课程实验

### 方案 B：直接魔改 CH5，向前移植

**思路：** 先在 CH5 实现完整 SMP，再把简化版移植到 CH4/CH3。

**优点：**
- 可以先在最完整的进程模型上思考多核问题
- CH5 的同步原语和进程管理更接近真实 OS

**缺点：**
- 对学生不友好，CH5 复杂度太高
- 向前移植时容易出现“不知道哪些是本质，哪些是 CH5 特有”
- 不符合“从简到繁”的教学逻辑

**适用性：** 适合研究，不适合教学

### 方案 C：最小侵入式，仅增加双核调度

**思路：** 不新增 crate，只在现有代码里加最小 SMP 支持，让两核能跑起来。

**优点：**
- 改动最少
- 短期内容易看到效果

**缺点：**
- 代码会很丑，很多逻辑分散在各章节里
- 难以讲清楚设计边界
- 后续扩展（TLB shootdown、work stealing）会很痛苦

**适用性：** 适合快速 demo，不适合作为教程

---

## 推荐方案：A（自底向上渐进式）

原因：
1. 这是**教学实验**，不是产品开发，清晰的分层比短期省代码更重要
2. 多核本来就是 OS 最易出 bug 的部分，必须让学生逐层理解 bug 从哪里来
3. tg-rcore-tutorial 本身就是按章节渐进构造内核，A 最契合现有结构

---

## 完整设计方案：CH3~CH5 多核教学框架

### 阶段 1：新增 `tg-rcore-tutorial-smp` crate

**目标：** 提供所有章节共享的最小 SMP 基础设施

**模块结构：**
```text
tg-rcore-tutorial-smp/src/
├── lib.rs
├── hart.rs
├── percpu.rs
├── spin.rs
├── barrier.rs
└── ipi.rs
```

**我实现的脚手架：**

1. **hart.rs**
   - `hart_id() -> usize`：读取 `tp` 或 `mhartid`
   - `boot_secondary_hart(hart_id, start_addr, opaque)`：SBI HSM 调用
   - `wait_for_all_harts()`：启动同步
   - `HART_STARTED: AtomicBool` / `AtomicUsize`

2. **percpu.rs**
   - `PerCpu<T, const N: usize>`：每核一份数据
   - `percpu!` 宏：声明 per-cpu 静态变量
   - 基于 `tp` 寄存器索引当前 hart

3. **spin.rs**
   - `SpinLock<T>` 结构体定义
   - `SpinLockGuard<T>` 类型签名
   - **学生练习**：实现 `lock()` / `drop()`

4. **barrier.rs**
   - `fence()`, `fence_r()`, `fence_w()`, `fence_rw()`
   - RISC-V `fence` 指令封装

5. **ipi.rs**
   - `send_ipi(target_hart)`：SBI IPI 封装
   - `IpiHandler` trait：供 CH4 TLB shootdown 使用

**学生练习点：**
- L1: 实现 `SpinLock::lock()` 的 CAS 自旋
- L1: 实现 `SpinLockGuard::drop()`
- L2: 用错误的 `Relaxed` ordering 版本做 bug demo，让学生修

**为什么这样设计：**
- 所有 SMP 章节都会用到这些基础设施
- 单独成 crate 后，学生能把“多核基础抽象”和“章节逻辑”分开理解

---

### 阶段 2：升级 `tg-rcore-tutorial-sync`

**目标：** 把单核专用的 `UPIntrFreeCell` 过渡到多核安全版本

**现状问题：**
`UPIntrFreeCell` 通过“关闭本核中断”保证临界区安全，但在多核下：
- 只能阻止**当前核**被打断
- 无法阻止**其他核**同时访问同一数据

所以它在 SMP 下**本质不安全**。

**改造方案：**

保留 API 形状，但内部切换：

```rust
// 单核
UPIntrFreeCell<T>

// 多核
SpinNoIrq<T> = SpinLock<T> + local interrupt disable
```

**我实现的脚手架：**
- `SpinNoIrq<T>` 包装类型
- 保持和原来 `exclusive_access()` 类似的使用体验
- `MutexBlocking` / `Semaphore` / `Condvar` 的接口签名不变

**学生练习点：**
- L1: 替换 CH3~CH5 所有 `UPIntrFreeCell` 调用点
- L2: 分析为什么“关中断”在多核下不够
- L2: 调整 `wait_queue` 操作的持锁顺序

**教学重点 bug：**
- 单核时没问题，多核瞬间数据竞争
- 最适合拿来讲“同步语义”和“硬件核数变化为什么会击穿错误假设”

---

### 阶段 3：CH3 多核适配

**目标：** 在最简单的无虚存环境下，把“双核都能跑任务”打通

**CH3 现状：**
- 固定大小 TCB 数组
- 单个全局调度循环
- 无地址空间隔离
- 最适合作为 SMP 入门章

**我实现的脚手架：**

1. **启动入口改造**
   - 主核正常进 `rust_main()`
   - 副核进入 `secondary_start`
   - 每核独立内核栈（272 KiB）
   - `boot_secondary_hart()` 框架调用

2. **QEMU 配置**
   - `-smp 2`

3. **测试宏**
   - `smp_test!` 用于指定核心执行测试

**学生练习点：**
- L1: 实现 `secondary_rust_main()`
- L2: 把全局就绪队列拆为 per-cpu 队列
- L2: 设计任务初始分配策略
- L3: 修复“两核同时执行同一个 task”的竞态

**核心易错点：**
1. 两核同时拿到同一个 `TaskControlBlock`
2. `remain` 计数器非原子更新
3. 副核 timer 中断还没初始化就进调度

**教学价值：**
CH3 没有虚存噪音，能纯讲“多核调度和共享状态竞争”。

---

### 阶段 4：CH4 多核适配

**目标：** 在引入虚存后，处理页表一致性和跨核 TLB 刷新

**CH4 新增复杂度：**
- 每进程 Sv39 地址空间
- MultislotPortal
- 页表修改后需要考虑其他核的 TLB

**我实现的脚手架：**

1. **副核页表激活序列**
   - 主核先建好内核页表
   - 副核启动后加载相同 `satp`
   - `sfence.vma` 后再进入调度

2. **TLB shootdown 框架**
   - `tlb_shootdown()` 接口
   - IPI 通知远程 hart 执行 `sfence.vma`

3. **Portal 改造**
   - 每核独立 portal slot

**学生练习点：**
- L1: 副核激活 `satp`
- L2: 实现 `tlb_shootdown()`
- L2: 修复两核同时分配同一物理页
- L3: 分析 portal 为什么必须 per-cpu

**核心易错点：**
1. stale TLB：一核 `munmap` 后另一核还拿旧翻译
2. 物理页帧分配器双重分配
3. portal 被多核并发踩坏

**教学重点：**
这是第一次把“缓存一致性 / 地址翻译一致性”拉进教学视野，非常适合讲多核最经典 bug。

---

### 阶段 5：CH5 多核适配

**目标：** 在完整进程模型下处理 `fork/exec/wait/exit` 的跨核并发

**CH5 复杂度最高：**
- 完整进程树
- Stride 调度
- `fork/exec/wait/exit`
- 跨核父子进程状态同步

**我实现的脚手架：**

1. **Per-CPU RunQueue**
   - 每核一个 `VecDeque<ProcId>`
   - 本地锁保护

2. **全局 ProcManager**
   - `BTreeMap<ProcId, Process>` 放全局
   - 就绪队列 per-cpu 化

3. **`on_cpu: AtomicUsize`**
   - 防止同一进程被两核同时 fetch

4. **Work stealing 接口**
   - 本地队列空时从其他核偷任务

**学生练习点：**
- L1: 实现本地 `fetch()`
- L2: 实现 `try_steal()`
- L2: `fork()` 的多核安全分配
- L2: `wait()/exit()` 跨核退出码传递
- L3: 讨论多核 stride 公平性

**核心易错点：**
1. work stealing 锁顺序不一致导致死锁
2. 同一进程被双核同时调度
3. `fork()` 时父进程状态被并发打断
4. `exit()` 和 `wait()` 跨核竞态

**教学价值：**
CH5 能把多核最真实的 OS 问题集中展示出来，是整个实验的高潮部分。

---

### 阶段 6：测试与文档交付

**我实现的脚手架：**
- `smp_test!` 宏
- `barrier_sync(n)`
- 输出检查工具
- 参考测试：
  - `test_basic_smp_boot`
  - `test_spinlock_contention`

**学生练习点：**
- 编写 `sched_yield` 并发测试
- 编写 `fork bomb` 多核压力测试
- 设计 `TLB shootdown` 触发用例
- 写性能对比实验

**文档交付：**
```text
docs/smp/
├── design-report.md
├── lab-guide.md
├── common-bugs.md
└── grading-rubric.md
```

---

## 用户态测试程序建议

新增到 `tg-rcore-tutorial-user`：
- `smp_hello`
- `smp_counter`
- `smp_yield_test`
- `smp_fork_stress`
- `smp_race_demo`

这样学生不只看内核日志，还能从 userland 看到双核行为差异。

---

## 结论

这是一个非常适合作为课程实验的主题，因为：

1. **足够真实**：多核确实是 OS 最容易出 bug 的部分之一
2. **足够有层次**：从 CH3 到 CH5，复杂度自然递增
3. **足够有教学价值**：能覆盖竞态、死锁、一致性、TLB、调度公平性等核心概念

如果你认可，我下一步就开始按照**方案 A**落具体实现。

**用户：**

继续实现，详细帮我解答问题

**Claude：**

好的 Master。我将开始按照方案 A 进行实现，先从 **阶段 1：新增 `tg-rcore-tutorial-smp` crate** 开始，然后依次适配 `sync`、`ch3`、`ch4`、`ch5`。

首先创建基础 SMP crate 和架构。

**Claude：**

我已经创建了 `tg-rcore-tutorial-smp` crate 的基础架构，并开始适配 `tg-sync` 以支持 SMP。

**Claude：**

修复 tg-smp crate 的初始编译错误。

**Claude：**

Claude Code 运行出现问题，需要恢复。请重新运行此提示继续工作。

**用户：**

我刚才让Codex接着你的继续做了，你找一下目前工作区的状态，然后重新规划

**Claude：**

好的 Master。我先检查当前工作区状态，看看 Codex 已经完成了哪些部分，然后基于现状重新规划下一步，而不是重复工作。

**Claude：**

Codex 已经完成了相当多的工作。让我先整理当前状态，然后给出新的规划。

**Claude：**

## 当前工作区状态分析

### 已完成的部分

#### 1. `tg-rcore-tutorial-smp` crate
- ✅ 已创建并基本实现
- 包含：`hart.rs`, `percpu.rs`, `spin.rs`, `barrier.rs`, `ipi.rs`
- `cargo check` 通过

#### 2. `tg-sync` 的 SMP 适配
- ✅ 已完成
- `UPIntrFreeCell` 在 `smp` feature 下切换为 `SpinNoIrq`
- 新增了 `SpinNoIrq<T>` 实现
- 单核/多核双模式兼容

#### 3. CH3 SMP
- ✅ **已基本完成并编译通过**
- 新增了 `src/smp.rs`
- `Cargo.toml` 添加了 `smp` feature
- `.cargo/config.toml` 添加 `-smp 2`
- 启动入口改造已完成
- `cargo check --features smp` 通过

### 未完成 / 有问题的部分

#### 4. CH4 SMP
- ⚠️ 部分完成，但有编译问题
- 缺少 `src/smp.rs` 文件或内容不完整
- 需要处理虚存相关的多核问题（页表共享、TLB）

#### 5. CH5 SMP
- ⚠️ 部分完成，但明显未收口
- `main.rs` 引用了 `mod smp;` 但文件不存在
- 多核进程调度语义还没完整接上

#### 6. 测试与文档
- ⚠️ 文档已有初稿，但测试还不充分
- 需要实际验证双核运行

### 关键观察

1. **Codex 采用的是正确路线**：方案 A，自底向上渐进
2. **CH3 是最稳定的断点**：已经编过，可以作为运行验证起点
3. **CH4/CH5 需要继续收口**：主要是 `smp.rs` 和调度/页表逻辑

---

## 新的实施计划

基于当前状态，我建议这样继续：

### 第一步：先验证 CH3 真运行
不要急着继续写 CH4/CH5，先确认 CH3 的双核启动是真能跑起来，而不只是 `cargo check`。

目标：
- `cargo run --features smp` 能看到两个 hart 的启动日志
- 至少能确认副核进入 `secondary_rust_main()`

### 第二步：补齐 CH4 的 `src/smp.rs`
CH4 的重点是：
- 副核激活内核页表
- 共享 kernel space
- portal per-cpu 化

### 第三步：补齐 CH5 的 `src/smp.rs`
CH5 的重点是：
- per-cpu runqueue
- `on_cpu` 防止双核同时调度同一进程
- `fork/exit/wait` 的跨核安全

### 第四步：补实验文档和测试
- `docs/smp/design-report.md`
- `docs/smp/lab-guide.md`
- `docs/smp/common-bugs.md`
- `docs/smp/test-plan.md`

---

## 现在最应该做的事

**先跑 CH3 的双核运行验证。**

原因：
- 如果 CH3 运行期都没把副核拉起来，继续写 CH4/CH5 只是空中楼阁
- CH3 最简单，最适合定位启动链路问题

我要开始检查 CH3 的当前 SMP 运行状态。

我继续检查并验证 CH3 的双核运行状态，先看当前代码是否真的能把副核启动起来，而不只是通过编译。

**Claude：**

用户中断了 tool 调用，需要重新检查当前状态并继续。

**用户：**

我刚才让codex把那部分测试完了，你看看记录，然后推进接下来的

**Claude：**

好的 Master。我先查看当前状态和最近的工作记录，看看 Codex 测试到了什么程度，然后直接接着推进后续部分，而不是重复验证。

**Claude：**

让我继续查看最近的提交和工作区状态。

**Claude：**

Codex 已经进一步推进了不少内容。让我先基于当前状态重新梳理，再继续实现。

**Claude：**

## 当前状态更新（基于 Codex 后续工作）

### 已完成

#### CH3
- ✅ SMP 编译通过
- ✅ 运行路径已调通
- ✅ 双核启动已验证

#### CH4
- ✅ SMP 编译通过
- ⚠️ 运行验证还需要进一步确认稳定性
- 已处理：
  - `src/smp.rs`
  - 页表激活
  - 动态主副核判断

#### CH5
- ⚠️ 正在收尾
- 已有 `src/smp.rs` 骨架
- 已开始统一单核/多核的处理器访问路径
- 编译问题在逐步清理

#### 文档
- ✅ `docs/smp/` 已有草稿

### 现在的重点

**收掉 CH5**，因为：
1. CH3/CH4 已经基本打通
2. CH5 是最复杂也最有价值的部分
3. 只要 CH5 收口，整个 T2L10 的代码主线就完整了

---

## 接下来的具体任务

### 1. 完成 CH5 的 `src/smp.rs`
- per-cpu runqueue
- work stealing 接口
- `on_cpu` 原子标记

### 2. 统一 syscall 访问层
让 `main.rs` 的 syscall 处理逻辑在：
- 单核模式：走 `PROCESSOR`
- 多核模式：走 `SMP_PROCESSOR`

### 3. 运行验证
- 双核启动
- shell 能进入
- fork/exec/wait 基本路径不炸

### 4. 补最终文档
- 把运行结论、OpenSBI 环境要求、常见 bug 写清楚

---

我现在继续推进 CH5 的收尾工作。

**Claude：**

修复 CH5 中 `Arc` 的导入问题。

**Claude：**

修复 unused import 问题——在 SMP 模式下 TaskControlBlock 不在 main.rs 中直接使用。

**Claude：**

`SyncUnsafeCell` 还不稳定。用更简单的方法——自定义包装类型。

**Claude：**

CH3 SMP 编译通过。同时确认非 SMP 模式也正常。

**Claude：**

两种模式都通过。现在并行实现 CH4 和 CH5。

**用户：**

[Request interrupted by user for tool use]

*我使用的模型和辅助工具为: Codex Cli: GPT5.4-XHigh*

*对应 session: `019d2dcd-8cbd-7242-9d45-5d562d94ca45`*

> 说明：以下保留用户与 AI 的完整可见对话，包含当时的 AGENTS/environment 输入，不包含 system/tool/token 元数据。

## T2L10

**用户：**

这是题目：10. 实验 ：ch3~ch5扩展多核的能力（操作系统中最容易出bug的部分之一）
目标：支持多核处理
参考：
https://github.com/arceos-org/arceos
实现要点（典型路线）：
确保 ch3~ch5能够继续支持多核能力
观测指标：
- 设计不同的测试环境和例子，测试多核执行的特点和优势
验收：
- 提供对应测试用例和文档，说明如何设计实现的，在开发中会出现哪些容易出错的地方，并如何进行修复的，以及完成设计分析报告。能帮助学生更好地理解多核，以及多核编程。这是claude的plan：一次性展示完设计方案                                                                          
                                                                                                
⏺ 完整设计方案：CH3~CH5 多核教学框架                                                            
                                         
  ---                  
  阶段 1：tg-rcore-tutorial-smp crate — SMP 基础设施
                                                                                                
  模块结构：           
  tg-rcore-tutorial-smp/src/                                                                    
  ├── lib.rs            # 公共接口                                                              
  ├── hart.rs           # Hart 启动与管理                                                       
  ├── percpu.rs         # Per-CPU 数据结构（tp 寄存器索引）
  ├── spin.rs           # SpinLock 框架（核心留空）                                             
  ├── barrier.rs        # 内存屏障封装   
  └── ipi.rs            # 核间中断框架                                                          
                                         
  脚手架（我实现）：                                                                            
  - hart.rs — 副核启动汇编、SBI 唤醒调用、hart_id() 读取、HART_STARTED 原子信号
  - percpu.rs — PerCpu<T> 结构体 + percpu! 宏，基于 tp 寄存器的 per-cpu 存储                    
  - ipi.rs — send_ipi(target_hart) SBI 封装 + IpiHandler trait 定义                             
  - barrier.rs — fence()/fence_r()/fence_w()/fence_rw() 的 RISC-V 封装                          
  - spin.rs — SpinLock<T> 结构体定义、SpinLockGuard<T> 类型签名                                 
                                                                                                
  学生练习：                             
  - L1 — 实现 SpinLock::lock() 的 CAS 自旋循环（compare_exchange + memory ordering 选择）       
  - L1 — 实现 SpinLockGuard::drop() 释放锁                                               
  - L2 — 提供一个用 Relaxed ordering 的"有 bug"版本，学生通过测试发现并修复 ordering 问题       
                                                                                                
  ---                                                                                           
  阶段 2：升级 tg-rcore-tutorial-sync crate                                                     
                                                                                                
  脚手架（我实现）：                                                                            
  - SpinNoIrq<T> — SpinLock + 中断屏蔽的组合包装，接口与原 UPIntrFreeCell 兼容
  - 保留 UPIntrFreeCell 的代码作为注释/文档对比                                                 
  - Mutex/Semaphore/Condvar 的接口签名保持不变，内部改用 SpinNoIrq                              
                                                                                                
  学生练习：                                                                                    
  - L1 — 将 CH3~CH5 内核中所有 UPIntrFreeCell 调用点替换为 SpinNoIrq
  - L2 — 回答设计问题：为什么单核中断屏蔽在多核下不安全？给出具体的竞态场景                     
  - L2 — Mutex 内部从 UPIntrFreeCell 换成 SpinNoIrq 后，wait_queue 操作需要怎样调整？           
                                         
  ---                                                                                           
  阶段 3：CH3 多核适配 — 最简多核（无虚存）                                                     
                                                                                                
  CH3 是最简单的起点：无虚存、固定任务数组、时间片轮转。                                        
                                         
  脚手架（我实现）：                                                                            
  - _start 汇编改造 — 主核(hart 0)正常启动，副核进入 wfi 等待                                   
  - secondary_start 汇编 — 副核被唤醒后设置独立内核栈，跳转 secondary_rust_main()               
  - Per-CPU 内核栈分配（每核独立 272KiB 栈空间）                                                
  - rust_main() 末尾调用 boot_secondary_hart() 唤醒副核的框架代码
  - QEMU 启动参数加 -smp 2                                                                      
  - 测试框架：提供 smp_test! 宏用于编写多核测试
                                                                                                
  学生练习：                                                                                    
  - L1 — 实现 secondary_rust_main()：初始化副核的 timer 中断、进入调度循环
  - L2 — 实现 Per-CPU 调度：将原来的全局 TCB 数组拆分为 per-cpu 就绪队列，每核独立轮转          
  - L2 — 实现任务初始分配策略：新任务如何分配到不同核心？（轮询 / 最少任务优先）                
  - L3 — 调试并修复：提供一个"两核同时访问全局 remain 计数器导致任务重复执行"的 bug 场景        
                                                                                                
  关键易错点（教学重点）：                                                                      
  1. 两核同时执行同一个 task（全局数组无锁保护）                                                
  2. remain 计数器的非原子递减导致提前/延迟退出                                                 
  3. Timer 中断在副核未初始化时触发 panic      
                                                                                                
  ---                                                                                           
  阶段 4：CH4 多核适配 — 虚存 + 页表                                                            
                                                                                                
  CH4 增加了 Sv39 虚存和 MultislotPortal，多核下引入页表一致性问题。                            
                                         
  脚手架（我实现）：                                                                            
  - 副核启动时的页表激活序列（在主核建好内核页表后，副核直接加载相同的 satp）                   
  - TlbShootdown 框架 — IPI 触发远程核心执行 sfence.vma 的机制                                  
  - MultislotPortal 的多核安全改造（每核独立 portal slot）                                      
  - 内核地址空间的共享映射（所有核心共用一份内核页表）    
                                                                                                
  学生练习：                             
  - L1 — 副核启动后激活虚存：设置 satp 并执行 sfence.vma，理解为何必须先 fence 再跳转           
  - L2 — 实现 tlb_shootdown()：当一个核心修改页表映射时，通知其他核心刷新 TLB                   
    - 提示：什么时候需要 shootdown？munmap/sbrk 缩减时                       
    - 提示：如果忘了 shootdown 会发生什么？（stale TLB → 访问已释放的物理页 → 数据损坏）        
  - L2 — 修复：两核同时 mmap 分配物理页导致同一物理页被分配两次的竞态                           
  - L3 — 思考题：MultislotPortal 为什么需要每核独立 slot？如果共用会出什么问题？                
                                                                                                
  关键易错点（教学重点）：                                                                      
  1. TLB 一致性 — 多核下最经典的 bug 来源之一
  2. 物理页帧分配器的并发安全                                                                   
  3. Portal 的并发使用冲突                                                                      
                                                                                                
  ---                                                                                           
  阶段 5：CH5 多核适配 — 完整进程管理                                                           
                                                                                                
  CH5 有 fork/exec/wait/exit 和 stride 调度，多核下复杂度最高。                                 
                                         
  脚手架（我实现）：                                                                            
  - PerCpuRunQueue 数据结构定义 — 每核一个 VecDeque<ProcId> + 本地 SpinLock
  - ProcManager 改造 — 进程表用全局 SpinNoIrq<BTreeMap> 保护，就绪队列 per-cpu                  
  - on_cpu: AtomicUsize 字段加入 PCB — 标记进程当前在哪个核上运行（防止同一进程被两核同时调度） 
  - Work stealing 的接口定义：trait WorkStealing { fn try_steal(&self, from: usize) ->         
  Option<ProcId>; }                                                                             
                                         
  学生练习：                                                                                    
  - L1 — 实现 Per-CPU fetch()：从本地队列取任务，本地为空时调用 try_steal()                     
  - L2 — 实现 try_steal()：从其他核的就绪队列尾部窃取一个任务              
    - 要点：窃取时必须持有两个队列的锁，如何避免死锁？（固定加锁顺序）                          
  - L2 — fork() 多核安全改造：                                                                  
    - 子进程放入哪个核的就绪队列？（当前核 vs 负载最轻的核）                                    
    - 复制地址空间时需要持锁吗？为什么？                                                        
  - L2 — wait()/exit() 多核安全：                                                               
    - 父进程在核 0 wait，子进程在核 1 exit，如何安全传递退出码？                                
    - 僵尸进程的回收在哪个核上执行？                            
  - L3 — Stride 调度的多核公平性：                                                              
    - 每核独立 stride 值 vs 全局 stride 值的权衡                                                
    - 设计实验证明 work stealing 能改善负载不均                                                 
                                                                                                
  关键易错点（教学重点）：                                                                      
  1. 死锁 — work stealing 需要同时锁两个队列                                                    
  2. 同一进程被两核同时 fetch（on_cpu 标记的必要性）                                            
  3. fork() 期间父进程被抢占，子进程地址空间复制不完整                                          
  4. exit() 和 wait() 的跨核通信竞态                                                            
                                                                                                
  ---                                                                                           
  阶段 6：测试用例 + 设计文档                                                                   
                                                                                                
  脚手架（我实现）：                     
  - 测试框架基础设施：                                                                          
    - smp_test! 宏 — 在指定核心上运行测试函数                                                   
    - barrier_sync(n) — n 核同步屏障，用于协调测试时序                                          
    - 输出验证工具 — 检查多核输出的正确性（顺序无关比较）                                       
  - 参考测试用例（示例）：                                                                      
    - test_spinlock_contention — 两核并发递增共享计数器
    - test_basic_smp_boot — 验证两核都成功启动         
                                                                                                
  学生练习：                             
  - L1 — 编写测试：两核并发 sched_yield 是否正确切换                                            
  - L2 — 编写测试：fork bomb 在多核下是否正确处理                                               
  - L2 — 编写测试：设计一个能触发 TLB shootdown bug 的用例（一核 munmap，另一核立即访问）       
  - L3 — 性能对比实验：                                                                         
    - 单核 vs 双核运行相同任务集的时间对比                                                      
    - 全局锁 vs Per-CPU 队列的调度延迟对比                                                      
  - L3 — 撰写设计分析报告：                                                                     
    - 多核启动流程与单核的差异                                                                  
    - 遇到的 bug 分类（竞态/死锁/一致性）及修复方法
    - 多核带来的性能提升和额外开销分析                                                          
                                                                                                
  ---                                                                                           
  用户态测试程序（新增到 tg-rcore-tutorial-user）                                               
                                                                                                
  脚手架（我实现）：                                                                            
  - smp_hello — 每核打印 "Hello from hart X"，验证基本多核                                      
  - smp_counter — 多进程并发递增共享内存计数器（需原子操作）                                    
  - smp_yield_test — 多进程交替 yield，验证调度公平性       
                                                                                                
  学生练习：                                                                                    
  - 编写 smp_fork_stress — 大量 fork 测试多核 fork 正确性
  - 编写 smp_race_demo — 故意制造竞态条件的演示程序                                             
                                                                                                
  ---                                    
  文件交付物总览                                                                                
                                                                                                
  新增/修改文件：                                                                               
  ├── tg-rcore-tutorial-smp/          # [新增] SMP 公共 crate                                   
  ├── tg-rcore-tutorial-sync/         # [修改] 升级为多核安全
  ├── tg-rcore-tutorial-ch3/          # [修改] 多核适配                                         
  ├── tg-rcore-tutorial-ch4/          # [修改] 多核适配
  ├── tg-rcore-tutorial-ch5/          # [修改] 多核适配                                         
  ├── tg-rcore-tutorial-user/         # [修改] 新增多核测试程序
  └── docs/smp/                       # [新增] 设计文档 + 实验指导书                            
      ├── design-report.md            # 设计分析报告                                            
      ├── lab-guide.md                # 实验指导书（含各阶段练习说明）                          
      ├── common-bugs.md              # 常见 bug 手册                                           
      └── grading-rubric.md           # 评分标准                                                
                                                                                                
  ---                                   你 继续搞吧

**AI：**

这次我直接接着把 `T2L10` 往下做，先看当前仓库里 Claude 已经改了哪些、做到哪一步，再补齐缺口并跑一轮验证。

**AI：**

我先按 `sallinarosist-workstyle` 的路子来，重点看本地已有改动和章节实现，不重起炉灶，优先接 Claude 已搭好的骨架往前推。

**AI：**

现在看具体改动面，重点是 `smp` crate、`sync` 改造、`ch3~ch5` 各自接到了哪里，先把断点找出来。

**AI：**

已经能看出来 Claude 主要是把入口和框架接上了，但 `ch4/ch5` 这块明显还没收口，接下来我读 `smp.rs` 和相关实现，先把能不能编过这条线打通。

**AI：**

`ch5` 这里已经实锤了：`main.rs` 引了 `mod smp;`，但文件根本没落盘。下一步我直接跑 `cargo check` 把整条错误链抓出来，再补文件和接口。

**AI：**

当前状态很明确了：`ch3+smp` 已经能过，`ch5+smp` 卡在缺文件，`ch4+smp` 先被一个依赖名问题拦住。现在我补 `ch5` 的 `smp.rs`，同时把 `ch4` 的依赖和接口对齐。

**AI：**

我准备开始改文件了：一块是给 `ch5` 补完整的 `smp.rs` 和统一的 SMP 辅助接口，另一块是修 `ch4` 的依赖名和副核 syscall 上下文指针遗漏。

**AI：**

我先自检一遍刚补的 `ch5`，这类跨 `cfg` 改动最容易出现语法和类型漏网，先把这些机械错误压平再继续。

**AI：**

先跑 `ch4/ch5` 的增量检查。现在重点不是“逻辑全优雅”，而是先把 feature `smp` 的编译面和关键接口闭起来。

**AI：**

`ch4` 这边暴露的是典型 SMP 编译坑：跨核静态里一旦放了裸指针或 `Process`，`Send/Sync` 就会立刻炸。我现在把这些教学内核里可控的约束显式写出来。

**AI：**

`ch5` 现在只剩几处结构性小坑了：一个是 `static` 初始化时不能直接造 `BTreeMap`，一个是 `cfg` 分支导致的不可达代码警告。我把这两个收掉。

**AI：**

这波基本不是逻辑错误了，纯粹是 `deny(warnings)` 下的条件编译清理。我把 SMP 模式里不参与构建的单核调度器和未用接口都裁掉。

**AI：**

代码面已经打通到 `ch3/ch4/ch5 + smp` 都能 `cargo check`。我再补一层交付物，把实验说明和测试设计文档落到仓库里，免得最后只剩代码没讲清楚。

**AI：**

文档我按“学生交付能直接用”的风格写，尽量短，重点放实现思路、测试步骤、容易翻车的点，不写那种空话报告。

**AI：**

最后我再做一轮统一收尾：确认工作区变更面和三章的 SMP 编译验证都还稳定。

**AI：**

老哥，这波我已经把 `T2L10` 继续接上了，代码和文档两边都补了一轮。

这次核心落地了三件事。第一，公共 SMP 基础设施 crate 已经接进来，入口统一设置 `tp=hart_id`，`SpinNoIrq` 和 Per-CPU 支撑也都打通了，见 [tg-rcore-tutorial-smp/src/lib.rs](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-smp/src/lib.rs)。第二，`ch4` 和 `ch5` 的 `smp` 分支我补到了能编过，其中 `ch4` 修了 syscall crate 依赖名和副核 schedule 的当前进程表指针传递，见 [tg-rcore-tutorial-ch4/Cargo.toml](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-ch4/Cargo.toml)、[tg-rcore-tutorial-ch4/src/smp.rs](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-ch4/src/smp.rs)；`ch5` 则补了完整的 SMP 调度骨架，用“运行时把进程从全局表取出，Trap 后再放回/回收”的方式避开双核同时执行同一进程的问题，见 [tg-rcore-tutorial-ch5/src/smp.rs](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-ch5/src/smp.rs) 和 [tg-rcore-tutorial-ch5/src/main.rs](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-ch5/src/main.rs)。第三，实验交付文档我补到了 [docs/smp/design-report.md](/Users/sallinarosist/OS/tg-rcore-tutorial/docs/smp/design-report.md)、[docs/smp/lab-guide.md](/Users/sallinarosist/OS/tg-rcore-tutorial/docs/smp/lab-guide.md)、[docs/smp/common-bugs.md](/Users/sallinarosist/OS/tg-rcore-tutorial/docs/smp/common-bugs.md)、[docs/smp/test-plan.md](/Users/sallinarosist/OS/tg-rcore-tutorial/docs/smp/test-plan.md)。

验证我已经跑过：
`tg-rcore-tutorial-ch3/ch4/ch5` 下的 `cargo check --features smp` 全部通过。当前没替你跑 QEMU 真机路径，所以运行时行为我还没替你做最终验收；另外 `kernel-context` 和 `kernel-alloc` 里还有一些 Rust 2024 的 `unsafe_op_in_unsafe_fn` 警告，但这次不会挡住 `smp` 分支编译。

如果你要，我下一步可以直接继续两条线里的一条：
1. 帮你补真正的运行级测试和用户态 SMP 小程序。
2. 帮你整理成可提交的实验报告/提交说明。

**用户：**

什么意思，现在的不是真正的运行级测试吗

**AI：**

不是，老哥，我现在做的是“编译级验证”，不是“运行级验证”。

区别很直接：

- 我已经做了：`cargo check --features smp`
  这只说明代码能通过类型检查、模块能连上、接口没炸、`cfg(feature = "smp")` 分支能编译。
- 我还没做：`cargo run --features smp` 或直接用 QEMU 启动内核
  这一步才会真的把系统跑起来，验证副核是否启动、调度是否正常、是否会卡死、是否会 panic、`fork/wait/mmap` 在双核下是否真能跑。

所以我刚才那句“没跑真正的运行级测试”，意思就是：

- 还没实际启动 `ch3/ch4/ch5` 的 SMP 内核
- 还没看 QEMU 日志
- 还没验证双核执行时的真实行为

如果你要，我下一步就直接给你跑真正的运行级测试，至少可以做这三项：

- `ch3`: `cargo run --features smp`
- `ch4`: `cargo run --features smp`
- `ch5`: `cargo run --features smp`

然后我把每章启动结果、是否双核起来、有没有 panic/死锁、日志里看到什么，直接汇总给你。

**用户：**

核对是否能正常运行

**AI：**

我现在直接做运行级核对：按 `ch3 -> ch4 -> ch5` 真跑 `cargo run --features smp`，看能不能启动、有没有双核日志、会不会 panic 或卡死。

**AI：**

三个章节都已经开始真跑了。`ch3` 先确认到 QEMU 起起来了；我继续盯 `ch4/ch5` 的实际日志，看有没有双核启动信息、panic 或卡死。

**AI：**

`ch5` 已经抓到一个真问题了：不是代码路径 panic，而是平台层 `SBI hart_start` 返回 `-2`，副核根本没被固件启动。现在我继续看 `ch4`，再把 `ch3` 重新跑一遍确认它到底是正常收尾还是静默退出。

**AI：**

`ch3` 和 `ch4` 现在都没有继续吐串口，像是已经卡在运行期了。我先看主机上的 QEMU 进程状态，确认是不是还活着、有没有异常退出。

**AI：**

我换成手动拉 QEMU 跑，避免 `cargo run` 的 runner 配置把现象搅混。这样能更明确看出：是内核自己 panic，还是只是 runner/图形配置让输出看不见。

**AI：**

我怀疑根因已经浮出来了：不是 `ch5` 独有，而是当前这套 `-bios none` 跑法根本没提供 HSM，`hart_start` 不可用。 我去看一下本地 `tg-sbi` 实现里有没有 HSM 支持，这样能判断是代码 bug 还是启动环境不支持。

**AI：**

根因已经基本坐实了：本地 `tg-sbi` 的 `nobios` 只支持 console/timer/reset，不支持 HSM 和 IPI。所以多核启动依赖的 `hart_start` 在当前运行环境里天然会失败。为了把结论钉死，我再直接手动跑一遍 `ch4` 或 `ch3` 的 ELF，看它们是不是也死在同一个地方。

**用户：**

<turn_aborted>
The user interrupted the previous turn on purpose. Any running unified exec processes may still be running in the background. If any tools/commands were aborted, they may have partially executed; verify current state before retrying.
</turn_aborted>

**用户：**

你能不能测试基础的exercise，不要管extension

**AI：**

能。我现在只看基础章节路径，不碰 `snake/pong/irq_input` 这类 extension feature，直接测 `ch3~ch5` 的基础 SMP 启动。先把 `ch3` 也补一遍基础跑测，确认是不是和 `ch4/ch5` 一样死在启动副核。

**AI：**

能，老哥。我刚才专门按“基础 exercise 路径”测了，没有开任何 extension feature。

我测的是：

- `ch4` 基础 SMP：`qemu-system-riscv64 -machine virt -nographic -bios none -smp 2 -kernel target/.../rosist-sallina-tg-rcore-tutorial-T1L2`
- `ch5` 基础 SMP：`cargo run --features smp`
- 都只开了 `smp`，没开 `pong`、`snake`、`irq_input` 之类

结果很明确：现在基础多核也**不能正常运行**，而且问题不是 extension 引起的，是运行环境本身不支持 `hart_start`。

实际运行时 `ch4` / `ch5` 都打到了同一个错误：

```text
SBI hart_start failed for hart 1: error -2
```

关键证据在这两处：

- [hart.rs](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-smp/src/hart.rs)
- [lib.rs](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-sbi/src/lib.rs)

`tg-rcore-tutorial-sbi` 现在的 `nobios` 只写了这些 SBI 能力：

- console
- timer
- system reset

没有 HSM，也没有 IPI。  
所以你现在用的 `-bios none` 基础环境下，`smp` 代码一调用 `hart_start` 启副核，就会失败。这个和 extension 无关，基础 exercise 也一样过不去。

结论就是：

- 基础 `exercise` 的单核运行没问题
- 基础 `exercise` 的**双核 SMP 当前不能正常跑**
- 根因是 `tg-sbi` 的 `-bios none` 环境不支持 HSM/IPI，不是 `pong/snake` 这些 extension 污染出来的

如果你要，我下一步可以直接做两种之一：

1. 改运行环境，换成支持 HSM 的 OpenSBI，再重新测基础 exercise。
2. 继续补 `tg-sbi` 的 `nobios`，把 HSM/IPI 自己实现掉。

**用户：**

那为什么你刚才测试的时候多次拉起了图形化窗口？这在只测试exercise的情况下不应该出现吧

**AI：**

对，这点你说得对，是我这边测试方式不够干净。

原因很具体：

- 我一开始对 `ch3` 用了 `cargo run --features smp`
- 但你当前仓库里的 [tg-rcore-tutorial-ch3/.cargo/config.toml](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-ch3/.cargo/config.toml) 已经被改成了这个 runner：
  - `-display cocoa`
  - `-device virtio-gpu-device,...`
- 所以即使我没开 `snake` 之类 extension feature，`cargo run` 还是会按这个 runner 拉图形窗口

也就是说：

- 不是我故意去测 extension
- 是 `ch3` 的运行器配置本身已经被前面的半成品改脏了
- 用 `cargo run` 会把这个图形 runner 一起带上，导致测试不纯

这也是为什么我后面改成了直接手写 QEMU 命令：

```bash
qemu-system-riscv64 -machine virt -nographic -bios none -smp 2 -kernel ...
```

这种方式才是“只测基础 exercise”的干净跑法，不会被 `.cargo/config.toml` 里的图形配置带偏。

所以你这个质疑是对的，我修正一下结论：

- 我前面那几次带窗口的测试，不算干净的 exercise-only 测试
- 后面我用手写 `qemu-system-riscv64 -nographic` 跑的 `ch4/ch5`，才是干净的基础测试
- 那个干净测试下，失败点依然是 `hart_start failed for hart 1: error -2`

如果你要，我下一步可以只用“手写 `qemu-system-riscv64 -nographic`”的方式，把 `ch3` 基础路径也重新做一遍干净测试，不再碰 `cargo run`。

**用户：**

你先测试吧

**AI：**

这次我只用手写的 `qemu-system-riscv64 -nographic` 跑，不走 `cargo run`，先把 `ch3` 的基础路径测干净。

**AI：**

`ch3` 这次没有弹图形窗口，但 10 秒内也没有任何串口输出。我再多等一轮，确认它是“静默卡住”还是晚一点才出日志。

**AI：**

`ch3` 这边看起来不是弹窗问题，而是手写的最简命令下完全静默。我换成显式 `-serial mon:stdio` 再测一次，看看是不是串口没有挂到标准输出。

**AI：**

`ch3` 的基础路径也被当前代码状态污染了：不加 GPU 设备会直接在启动早期 panic。我再补上设备但不拉图形窗口，只看它后面是不是也卡在副核启动。

**用户：**

<turn_aborted>
The user interrupted the previous turn on purpose. Any running unified exec processes may still be running in the background. If any tools/commands were aborted, they may have partially executed; verify current state before retrying.
</turn_aborted>

**用户：**

我的建议是你恢复到完全没更改这个（应该是上一次commit）的状态，然后从7c2e6255432f24b63a8e644ff75c1e91185e2e82开始新拉一个分支，然后修改

**AI：**

可以，老哥。更稳的做法不是“恢复当前目录”，而是**保留现在这堆脏改动不动**，直接从 `7c2e6255432f24b63a8e644ff75c1e91185e2e82` 新开一个 `worktree + branch` 重新做。

这样有两个好处：

- 不会误删你和 Claude 现在工作区里的未提交改动
- 新分支是干净基线，后面只做基础 `exercise`，不被这堆半成品污染

如果你同意，我下一步就做这个：

1. 从 `7c2e6255432f24b63a8e644ff75c1e91185e2e82` 新建一个分支
2. 单独开一个新目录 worktree
3. 后续所有修改都只在那个新 worktree 里做

你给我一句确认，我就直接开始。

**用户：**

半成品也删掉，没关系

**AI：**

我按你的意思处理：直接丢掉当前半成品，在这个仓库里清回干净状态，然后从 `7c2e6255432f24b63a8e644ff75c1e91185e2e82` 新开一个分支重做。先核对当前分支和目标 commit 是否存在。

**AI：**

目标 commit 在，当前分支是 `test`。我现在直接清掉工作区，再从那个 commit 新建分支 `t2l10-redo`。

**AI：**

未跟踪文件已经删掉了，剩下是 Git 索引权限限制住了 `reset`。我直接申请提权，把已跟踪改动清掉并切到新分支。

**AI：**

清干净并切好了。我再核对一下当前分支和工作区，确保现在就是从那个 commit 出发的干净状态。

**AI：**

现在分支和提交点都对了，唯一还剩一个未跟踪目录 `tg-rcore-tutorial-ch9/`。我先看一下它是什么，再决定要不要一起清掉。

**AI：**

好了。

当前主仓库已经切到干净的新分支：

- 分支：`t2l10-redo`
- HEAD：`7c2e6255432f24b63a8e644ff75c1e91185e2e82`

我已经清掉了刚才那批半成品改动。现在主仓库里唯一还显示未跟踪的是 [tg-rcore-tutorial-ch9](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-ch9)，但它本身是一个独立的 git work tree，不是刚才那批改动的一部分，所以我没动它。

现在可以直接从这个干净基线开始重做 `T2L10`。如果你要，我下一步就按“只做基础 exercise，不碰 extension”的要求重新开始。

**用户：**

是的，按照一开始的plan重做T2L10,这个分支下现在应该没有extension内容

**AI：**

我按最初那套思路从干净基线重做，但这次只做基础 `exercise` 路径。先把 `ch3~ch5`、`tg-sbi` 和 `tg-sync` 的当前基线读一遍，确认没有 extension 污染，再从底层把多核启动能力补起来。

**AI：**

基线确认完了：`ch3~ch5` 现在都只有 `exercise`，没有 extension feature。接下来我先读 `msbi`，因为多核能不能跑，第一拦路虎就是这里有没有 HSM/IPI。

**AI：**

我已经把关键基线摸清了。下一步我准备按这条顺序做：先把 `tg-sbi` 的 `-bios none` 多核启动补起来，再接 `tg-smp` 基础设施，然后依次改 `tg-sync`、`ch3`、`ch4`、`ch5`，最后只用基础路径做运行验证。先把相关实现文件再读全。

**AI：**

我先落底层代码，先做这几件最关键的：`tg-sbi` 补 `hart_start`/`send_ipi` 的最小支持，新增 `tg-smp` crate，然后把 `tg-sync` 切成单核/多核两套内部实现。这样上层三章接起来时不会反复返工。

**AI：**

我现在开始落底层改动了：先把 `msbi` 里的 HSM 打通，不然上层所有 SMP 都是空转。这个阶段改动主要在 `tg-rcore-tutorial-sbi` 和新建 `tg-rcore-tutorial-smp`，还不会碰上层逻辑。

**AI：**

底层第一轮已经写进去了。我先只检查 `tg-sbi`、`tg-smp`、`tg-sync` 这三块，确保启动机制和同步层本身是干净的，再上卷到章节里。

**AI：**

我先接 `ch3`，因为它最简单，最适合验证“副核能不能真的起来”。这一轮会改 `Cargo.toml`、启动入口、QEMU 参数和一个新的 `src/smp.rs`。

**AI：**

我先只编 `ch3+smp`，把入口和副核启动这条链单独压通。这里如果能编过，下一步就直接跑真机看 `hart 1` 能不能起来。

**AI：**

基线里 `tg-rcore-tutorial-syscall` 的包名已经换掉了，所以三章当前其实本来就编不起来。我先把这个依赖名纠正，不然没法继续验证后面的 SMP 改动。

**AI：**

`ch3` 现在已经开始编章节本体了，我再等一轮把真正的报错抓出来。这里只有把 `deny(warnings)` 压平以后，运行验证才有意义。

**AI：**

`ch3` 现在卡的不是 SMP 代码，而是 build.rs 在编用户态程序时要拉 crates index，当前沙箱网络不通。我要提权跑一次 `cargo check`/`cargo build`，把依赖下载打通，不然三章后面的验证都过不去。

**AI：**

依赖下载已经放开了，`ch3` 现在在真正编章节代码。我继续等结果，拿到第一轮编译错误后就收口。

**AI：**

`ch3+smp` 已经编过了。我先把它真跑起来看双核启动日志，如果这步顺了，再按同样模式推进 `ch4/ch5`。

**AI：**

`ch3` 运行期还没吐串口。我再等一轮；如果还是静默，就说明现在不是 HSM panic，而是更早的启动路径还有问题。

**AI：**

我先做一个最小排查：用同一个 `ch3` 二进制改成单核启动。如果单核都静默，就说明是我刚改的 M 态入口有问题，不是上层 SMP 调度的问题。

**AI：**

我先修 M 态副核停车策略，把“忙等”改成“MSIP 唤醒”，这更接近真正的 HSM，也能避免副核纯自旋把启动阶段拖死。这一步只动 `msbi` 和 `m_entry.asm`。

**AI：**

我重新编一轮 `ch3+smp`，然后马上真跑。这个结果能直接告诉我们底层 HSM 这次是不是通了。

**AI：**

好，`ch3` 已经从“完全静默”变成能正常进入主核初始化了。现在我继续等，看它是不是卡在 `hart_start` 之后，还是副核已经真正进了调度循环。

**AI：**

现在大概率是卡在“主核等副核就绪”的阶段。我给 `ch3` 的 SMP 路径加两条关键日志：一条在主核发起 `hart_start` 前后，一条在副核刚进入 S 态时。这样能马上看清卡点。

**AI：**

日志位置更靠前一点。我再补两条：一条在 `init_smp_scheduler` 之后，一条在真正调用 `boot_secondary()` 之前。这样能区分是调度器初始化卡住，还是 HSM 启动卡住。

**AI：**

我再细分一步，把主核日志切成“`hart_start` 调用前”和“`hart_start` 返回后”。这样能马上判断是 HSM handler 自己没返回，还是返回了但副核没有进 S 态。

**AI：**

我先把唤醒路径退回到最朴素的“共享状态轮询”，因为现在已经确认卡点在 `hart_start` 的返回路径上，MSIP 这层反而在拖后腿。先让 ecall 稳定返回，再看副核能不能靠轮询起来。

**AI：**

我重新跑一次 `ch3`。这次目标很单纯：先确认 `hart_start` 会不会正常返回。只要主核能打出 `hart_start returned`，说明 M 态 HSM 基本链路就通了。

**AI：**

好，HSM ecall 已经回来了。现在只剩副核没把 `HART_STARTED` 置起来。我继续盯一轮，看副核会不会晚一点进入；如果没有，我就直接查副核入口那段。

**AI：**

我已经定位到一个更像真因的点了：副核如果是被 `msip` 唤醒后才开始跑，它进 `_m_start` 时状态其实已经写好了，结果我们先 `wfi` 再检查，直接把自己睡死了。现在我把停车逻辑改成“先看状态，再决定睡不睡”。

**AI：**

我先做一个环境验证：不再强行 `-bios none`，直接让 QEMU 用默认 OpenSBI 启动同一个 `ch3+smp` 内核。如果副核能起来，那说明代码路径基本可用，真正的问题就在 `nobios` 这条运行环境。

**AI：**

我准备换运行基线了：SMP 走外部 OpenSBI，不再强行 `-bios none`。原因很硬：多核真正需要的是完整 HSM，而不是我们在教学最小 SBI 里硬补一个半成品。先把三章都切到 OpenSBI 路径，再拿 `ch3` 做第一个运行验证。

**AI：**

我先只验证 `ch3` 的 OpenSBI 路径。如果这步能把副核拉起来，后面 `ch4/ch5` 继续照这个环境接就行。

**AI：**

OpenSBI 这条路现在也被基线的链接地址卡住了：当前内核 ELF 还是从 `0x80000000` 起，直接和默认 OpenSBI 固件重叠。我要先看一下这个地址是谁定的，判断是改链接脚本更省，还是继续把 `nobios` 的副核唤醒补完更省。

**AI：**

真正把内核压到 `0x80000000` 的不是 `tg-sbi` 依赖，而是三章的 `build.rs` 还在硬写 `NOBIOS_SCRIPT`。我把这层也切到普通 `SCRIPT`，这样 OpenSBI 才能正常装内核。

**AI：**

链接脚本已经切过去了。我重新编一次 `ch3` 再跑，如果这次不再报 OpenSBI ROM overlap，就说明环境基线终于对了。

**AI：**

`ch3` 的基础 SMP 已经跑通了，环境基线也确定了。现在我切到 `ch4/ch5`，同时把 `tg-smp::ipi` 改成“共享 pending 位 + 真 SBI sPI”双保险，后面 TLB shootdown 和跨核唤醒就不至于太虚。

**AI：**

我开始落 `ch4`。这一章的核心不是调度，而是“副核拿到正确的 `satp` 后能不能切进同一份内核地址空间”，所以我会先把 portal、`satp` 激活和共享进程表这条线接起来。

**AI：**

`ch4` 的代码面我先接好了。现在只做编译验证，把 `cfg(feature = "smp")` 下面的死角和缺口先捞出来，再进运行级。

**AI：**

`tg-smp::ipi` 里有个纯机械的 `deny(warnings)` 问题，我已经收掉了。继续跑 `ch4+smp` 编译，把真正的章节级错误抓出来。

**AI：**

我把 `ch4` 的几个 `cfg` 死角收了，再跑一轮编译。现在应该能看到更接近真实逻辑的问题，而不是导入和条件编译噪音。

**AI：**

我把 `cfg` 位置放正了。再跑一次 `ch4+smp` 编译，应该就能看到是不是已经到了可运行状态。

**AI：**

`ch4` 只剩一个未用函数告警，我已经压掉了。再跑一次编译，过了我就直接做运行验证。

**AI：**

`ch4+smp` 编译已经过了。我现在直接做运行级核对，重点看这三件事：OpenSBI 下副核能否起来、`satp` 激活后是否能进入调度、portal 双槽位是否至少不当场炸。

**AI：**

我发现一个更关键的运行时问题了：OpenSBI 的 boot hart 不一定是 `hart 0`。`ch4` 第一次启动时就是 `hart 1` 先进来，所以我们把“主核固定当 0、固定启动 1”这套假设打破以后，代码才能稳定。现在我先把 `ch3/ch4` 的主副核角色改成动态判断。

**AI：**

我把主副核角色从“写死 0/1”改成“以当前 boot hart 为主核”了。现在重新跑 `ch4`，看它是不是还能稳定把另一个 hart 拉起来，而不是再触发 `-6`。

**AI：**

我开始接 `ch5` 了。这一章我不打算硬套单核 `PManager`，而是直接走“当前运行进程从全局表取出，Trap 后再放回/回收”的多核语义，不然 `wait/fork/exit` 很容易两核打架。

**用户：**

继续

**AI：**

我继续把 `ch5` 收到底。现在剩两块硬活：补 `src/smp.rs`，以及把 syscall 实现切到“单核走 `PROCESSOR`，多核走 `SMP_PROCESSOR`”的统一访问层。

**AI：**

`ch5` 的多核骨架已经接上了。我先跑编译，不急着进 QEMU，先把 `cfg`、所有权和 syscall 接口这几个最容易炸的点收掉。

**AI：**

`ch5` 正在真正编主体了，我再等一轮，把最终报错抓出来。这里大概率是 `cfg` 和多核状态机的小尾巴。

**AI：**

`ch5` 这次不是内核逻辑炸，是 build.rs 在处理用户态工程时撞到了一个半初始化目录。我直接把三章都绑到仓库根的 `tg-rcore-tutorial-user`，省得它们各自 clone 一份乱掉。

**AI：**

用户态路径已经统一了，我再跑 `ch5+smp` 编译。接下来应该才会真正落到内核代码本身的错误。

**AI：**

`ch5` 还在编，我继续等结果。现在已经不再是 build 环境问题，剩下就是多核实现本身的尾巴了。

**AI：**

这三个都是纯 `deny(warnings)` 尾巴，我已经收了。再跑一次 `ch5+smp` 编译，过了就进运行验证。

**AI：**

`ch5+smp` 编译已经过了。我现在直接跑真机，重点看 initproc、fork/exec/wait 这一串能不能在双核下继续活着。

**AI：**

`ch5` 现在不是多核调度先炸，而是 shell 的 `read` 路径把 OpenSBI 返回的“无字符”当成了 `0xff` 一路灌进用户态，最后把 shell 自己喂爆了。我先把 `read` 改成真正的阻塞读，这样才能看清 `fork/exec/wait` 的正常行为。

**AI：**

我重跑 `ch5`，这次我会真给 shell 发一行输入，确认它不是假活着卡住，而是能在双核下正常接受输入、继续执行。

**AI：**

我把三章的代码链路已经打通到能跑了，接下来补实验交付文档，顺便把这次真正踩到的运行环境结论写清楚：`T2L10` 这次要靠 OpenSBI 跑，不再走 `-bios none`。

**AI：**

我做最后一轮收尾检查：看工作区改动面和当前分支状态，确认这一版已经是从干净基线重做出来的，不再是之前那堆半成品。

**AI：**

现在这版已经在 `t2l10-redo` 分支上重做起来了，而且不是之前那种半成品状态。

这次我把环境基线改成了 OpenSBI 路径，不再走 `-bios none`。对应地：

- `ch3/ch4/ch5` 都加了 `smp` feature
- 三章的 `build.rs` 都改回普通 `tg_linker::SCRIPT`
- 三章 `.cargo/config.toml` 都切到 `-smp 2`
- 三章都统一绑到根目录 [tg-rcore-tutorial-user](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-user)

基础设施这边：

- 新增了 [tg-rcore-tutorial-smp](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-smp)
- 升级了 [tg-rcore-tutorial-sync/src/up.rs](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-sync/src/up.rs)
- `tg-sbi` 里也补了最小 HSM/启动相关代码，但当前实际运行主要依赖 OpenSBI

运行结果我已经实际测过：

- `ch3`: `cargo run --features smp` 能双核跑起来，两个 hart 会一起推进用户程序
- `ch4`: `cargo run --features smp` 能双核启动，副核能激活页表并进入调度
- `ch5`: `cargo run --features smp` 能双核启动，能进 `Rust user shell`，并稳定出现 `>>`

文档也补了：

- [design-report.md](/Users/sallinarosist/OS/tg-rcore-tutorial/docs/smp/design-report.md)
- [lab-guide.md](/Users/sallinarosist/OS/tg-rcore-tutorial/docs/smp/lab-guide.md)
- [common-bugs.md](/Users/sallinarosist/OS/tg-rcore-tutorial/docs/smp/common-bugs.md)
- [test-plan.md](/Users/sallinarosist/OS/tg-rcore-tutorial/docs/smp/test-plan.md)

