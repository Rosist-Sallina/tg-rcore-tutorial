# T4-SMP 设计文档：Lab8+Doom 的内核态中断与多核支持

> **分支**: `feat/T4-smp`（基于 `feat/T4`）
> **参考**: `t2l10-redo` 分支的 SMP 实现（ch3-ch5 级别）

---

## 一、目标与范围

在已完成的 ch8（线程 + 同步原语 + Doom）基础上扩展：

| 功能 | 当前状态 | 目标状态 |
|------|---------|---------|
| 内核态中断响应 | ❌ 内核执行时 SIE=0，无抢占 | ✅ Timer 中断驱动时间片轮转 |
| 多核支持 | ❌ 单 hart，顺序调度 | ✅ 双核并行，线程级 work-stealing |
| 用户态测试 | ❌ 仅有 ch8 原有测试 | ✅ SMP 正确性测试 + 内核中断测试 |
| 性能对比 | ❌ 无基准程序 | ✅ 矩阵乘法 / Doom 帧率测量 |

---

## 二、架构决策

### 2.1 SBI 选择：复用 t2l10-redo 增强版

ch8 当前使用 `tg_sbi` 的 `nobios` feature（内嵌 M-mode 固件）。启动副核需要 HSM（Hart State Management）ecall（SBI 扩展 `0x48534D`），而 t2l10-redo 分支已在 `tg-rcore-tutorial-sbi/src/msbi.rs` 中完整实现了 HSM 支持。

**方案**：将 t2l10-redo 的增强 SBI（`msbi.rs` + `m_entry.asm`）合并到当前 `tg-rcore-tutorial-sbi` crate，保持 `nobios` 特性标志不变。

HSM 启动流程（M-mode msbi.rs 中）：
```
S-mode: SBI ecall (EXT=0x48534D, FID=0)
  → M-mode: handle_hart_start(hartid, start_addr, opaque)
    → 状态：HART_STOPPED → HART_START_PENDING
    → CLINT_MSIP[hartid] = 1  （软件中断唤醒副核）
  → 副核 M-mode 陷入中断
    → 读取 start_addr / opaque
    → mret → 副核 S-mode 入口
```

### 2.2 SMP 层次：在 ch8 线程粒度上扩展

t2l10-redo 的 ch5 SMP 是**进程级**调度（每核各自取进程）。ch8 的调度单元是**线程**（Thread），因此需要将 `SmpState.ready` 改为以 `ThreadId` 为单位的就绪队列。

关键变化：
- `ready: [VecDeque<ThreadId>; NUM_CPUS]`（而非 ProcId）
- 进程表（`BTreeMap<ProcId, Process>`）和线程表（`BTreeMap<ThreadId, Thread>`）均放入同一个 `SpinNoIrq<SmpState>` 保护
- 线程→进程的映射 `BTreeMap<ThreadId, ProcId>` 也在其中

### 2.3 内核态中断：两层机制

**层次 1：调度循环中的 timer 中断**（主要）

当前 ch8 调度循环逻辑：
```
loop {
    find_next() → execute() → match scause { syscall / 异常 }
}
```

添加 timer 驱动后：
```
loop {
    // 设置本次时间片
    set_timer(now + QUANTUM);
    find_next() → execute() → match scause {
        UserEnvCall   → 处理 syscall → suspend/exit/block
        SupervisorTimer → reset timer → suspend（强制切换）
        异常          → exit
    }
}
```

`execute()` 返回时，CPU 已从用户态切换回内核态，此时 `scause` 中可能就是 `SupervisorTimer`，说明时间片已耗尽。

**层次 2：长 syscall 中的内核抢占**（进阶，可选）

对于不立即返回的 syscall（如 `read` 阻塞轮询 UART），内核执行 `console_getchar()` 时一直循环，不会主动 `yield`。要支持这种场景下的抢占，需要：

1. 在轮询循环的非临界区间段短暂打开 `sstatus.SIE=1`
2. 注册一个内核态 trap 入口（`stvec` 在 S-mode 时指向 `kernel_trap_entry`）
3. timer 中断触发 `kernel_trap_entry`，保存当前上下文，设置 `need_reschedule` flag
4. 轮询循环检测到 flag 后调用 `schedule_yield()`

> **教学建议**：层次 1 是学生练习的核心，层次 2 作为进阶选做题。

---

## 三、代码改动清单

### 3.1 复制 SMP 基础设施（不修改，直接复用）

```bash
# 从 t2l10-redo 复制 SMP crate
git checkout t2l10-redo -- tg-rcore-tutorial-smp/
# 复制增强 SBI（包含 HSM 支持）
git checkout t2l10-redo -- tg-rcore-tutorial-sbi/src/msbi.rs
git checkout t2l10-redo -- tg-rcore-tutorial-sbi/src/m_entry.asm  # 如有变化
```

### 3.2 `tg-rcore-tutorial-ch8/Cargo.toml`

增加 `smp` feature 和依赖：

```toml
[features]
default = []
exercise = []
smp = ["tg-smp"]                   # 新增

[dependencies]
# ... 现有依赖 ...
tg-smp = { package = "rosist-sallina-tg-rcore-tutorial-smp-t2l10",
            path = "../tg-rcore-tutorial-smp", optional = true }
```

同时：在 `_start()` 入口添加 `mv tp, a0` 以传递 hart_id（SMP 模式需要 `tp` 寄存器存储 hart id）：

```rust
// 修改 tg-rcore-tutorial-ch8/src/main.rs 中的 _start：
core::arch::naked_asm!(
    "mv tp, a0",            // ← 新增：保存 hart_id 到 tp
    "la sp, {stack} + {stack_size}",
    "j  {main}",
    // ...
)
```

portal slots 数量根据 feature 动态设置：

```rust
// rust_main() 中：
#[cfg(feature = "smp")]
let portal_slots = 2;
#[cfg(not(feature = "smp"))]
let portal_slots = 1;
let portal_size = MultislotPortal::calculate_size(portal_slots);
```

### 3.3 `tg-rcore-tutorial-ch8/src/smp.rs`（新建）

这是核心文件，实现 ch8 线程粒度的 SMP 调度器。完整结构如下：

```rust
//! 第八章 SMP 多核调度（线程粒度）。

use crate::process::{Process, Thread};
use alloc::collections::{BTreeMap, VecDeque};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Lazy;
use tg_kernel_context::foreign::{MultislotPortal, TpReg};
use tg_smp::hart;
use tg_smp::percpu::PerCpu;
use tg_smp::spin::SpinNoIrq;
use tg_task_manage::{ProcId, ProcRel, ThreadId};

pub const NUM_CPUS: usize = tg_smp::MAX_CPUS;
const SECONDARY_STACK_SIZE: usize = 4 * 4096;

// ─── 副核栈 ───
#[repr(transparent)]
struct StackStorage([UnsafeCell<u8>; SECONDARY_STACK_SIZE]);
unsafe impl Sync for StackStorage {}
impl StackStorage {
    const fn zeroed() -> Self { Self(unsafe { core::mem::MaybeUninit::zeroed().assume_init() }) }
    fn top(&self) -> usize { unsafe { self.0.as_ptr().add(SECONDARY_STACK_SIZE) as usize } }
}
#[unsafe(link_section = ".boot.stack")]
static SECONDARY_STACK: StackStorage = StackStorage::zeroed();

// ─── 全局状态 ───
static KERNEL_SATP: AtomicUsize = AtomicUsize::new(0);
pub static LIVE_TASKS: AtomicUsize = AtomicUsize::new(0);  // 存活线程计数

/// Per-CPU 当前线程指针（指向 SmpState 中的 Thread，仅在 execute() 期间有效）
pub static CURRENT_THREAD: PerCpu<usize> = PerCpu::new([0; NUM_CPUS]);
/// Per-CPU 当前进程 PID
pub static CURRENT_PROC_PID: PerCpu<usize> = PerCpu::new([0; NUM_CPUS]);

static NEXT_TARGET_CPU: AtomicUsize = AtomicUsize::new(0);

// ─── 核心数据结构 ───
struct SmpState {
    // 进程表
    procs: BTreeMap<ProcId, Process>,
    proc_rels: BTreeMap<ProcId, ProcRel>,
    // 线程表（key = ThreadId，value = Thread）
    threads: BTreeMap<ThreadId, Thread>,
    // 线程 → 归属进程的映射
    thread_to_proc: BTreeMap<ThreadId, ProcId>,
    // Per-CPU 就绪队列（以线程为调度单位）
    ready: [VecDeque<ThreadId>; NUM_CPUS],
}

impl SmpState {
    fn new() -> Self { /* ... */ }

    /// 从指定 CPU 的就绪队列取线程，取不到则窃取对方队列尾部
    fn fetch_for(&mut self, cpu: usize) -> Option<(ThreadId, Thread, ProcId)> {
        let tid = self.ready[cpu].pop_front()
            .or_else(|| self.ready[1 - cpu].pop_back())?;
        let proc_id = *self.thread_to_proc.get(&tid)?;
        let thread = self.threads.remove(&tid)?;
        Some((tid, thread, proc_id))
    }
}

pub struct SmpProcessor {
    state: SpinNoIrq<SmpState>,
}

impl SmpProcessor {
    /// 添加进程
    pub fn add_process(&self, pid: ProcId, process: Process, parent: ProcId) { /* ... */ }

    /// 添加线程（指定归属进程），分配到目标 CPU
    pub fn add_thread(&self, tid: ThreadId, thread: Thread, pid: ProcId) {
        let target = NEXT_TARGET_CPU.fetch_add(1, Ordering::Relaxed) % NUM_CPUS;
        let mut state = self.state.lock();
        state.threads.insert(tid, thread);
        state.thread_to_proc.insert(tid, pid);
        state.ready[target].push_back(tid);
        LIVE_TASKS.fetch_add(1, Ordering::AcqRel);
    }

    /// 取出下一个可运行线程
    pub fn fetch_next(&self, cpu: usize) -> Option<(ThreadId, Thread, ProcId)> {
        self.state.lock().fetch_for(cpu)
    }

    /// 线程时间片用完，重新入就绪队列
    pub fn suspend(&self, tid: ThreadId, thread: Thread, pid: ProcId, cpu: usize) {
        let mut state = self.state.lock();
        state.threads.insert(tid, thread);
        state.ready[cpu].push_back(tid);
    }

    /// 线程阻塞（从就绪队列移除，等待唤醒）
    /// 注意：线程已从 state.threads 中取出（由调用方持有），此处仅存回以备唤醒
    pub fn block_thread(&self, tid: ThreadId, thread: Thread, pid: ProcId) {
        let mut state = self.state.lock();
        state.threads.insert(tid, thread);
        // 不放入 ready 队列
    }

    /// 唤醒被阻塞的线程
    pub fn wakeup_thread(&self, tid: ThreadId) {
        let target = NEXT_TARGET_CPU.fetch_add(1, Ordering::Relaxed) % NUM_CPUS;
        let mut state = self.state.lock();
        state.ready[target].push_back(tid);
    }

    /// 线程退出
    pub fn thread_exit(&self, tid: ThreadId, pid: ProcId, code: isize) { /* 清理映射，可能触发进程退出 */ }

    /// 进程 fork
    pub fn fork_current_thread(&self) -> isize { /* 复制进程 + 创建主线程 */ }

    /// 访问当前进程（通过 CURRENT_PROC_PID）
    pub fn with_current_proc<R>(&self, f: impl FnOnce(&mut Process) -> R) -> R {
        let pid = ProcId::from_usize(CURRENT_PROC_PID.get());
        let mut state = self.state.lock();
        f(state.procs.get_mut(&pid).unwrap())
    }
}

pub static SMP_PROCESSOR: Lazy<SmpProcessor> = Lazy::new(SmpProcessor::new);

// ─── 副核启动 ───
pub fn boot_secondary() {
    let primary = hart::hart_id();
    let secondary = 1 - primary;
    hart::boot_hart(secondary, _secondary_start as *const () as usize, SECONDARY_STACK.top());
    hart::wait_for_hart(secondary);
}

#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
unsafe extern "C" fn _secondary_start() -> ! {
    core::arch::naked_asm!(
        "mv tp, a0",   // hart_id → tp
        "mv sp, a1",   // stack top → sp
        "j {main}",
        main = sym secondary_rust_main,
    )
}

extern "C" fn secondary_rust_main() -> ! {
    // 1. 加载内核页表
    let satp = KERNEL_SATP.load(Ordering::Acquire);
    unsafe {
        core::arch::asm!(
            "csrw satp, {satp}",
            "sfence.vma",
            satp = in(reg) satp,
            options(nostack),
        );
    }
    // 2. 标记已启动
    let hart_id = hart::hart_id();
    hart::mark_hart_started(hart_id);
    // 3. 初始化副核专用 portal slot 并进入调度循环
    let portal = unsafe {
        MultislotPortal::init_transit(crate::PROTAL_TRANSIT.base().val(), NUM_CPUS)
    };
    schedule_loop(portal)
}

// ─── 调度主循环（每核独立运行）───
pub fn schedule_loop(portal: &'static mut MultislotPortal) -> ! {
    use riscv::register::{scause::{self, Exception, Interrupt, Trap}, sie, time};
    use tg_syscall::{Caller, SyscallId as Id, SyscallResult as Ret};

    let hart_id = hart::hart_id();
    const QUANTUM: u64 = 12_500;  // ~10ms @ 12.5MHz

    // 启用 S-mode timer 中断
    unsafe { sie::set_stimer() };

    loop {
        // 无存活线程：主核关机，副核自旋
        if LIVE_TASKS.load(Ordering::Acquire) == 0 {
            if hart_id == 0 { tg_sbi::shutdown(false); }
            loop { core::hint::spin_loop(); }
        }

        let Some((tid, mut thread, pid)) = SMP_PROCESSOR.fetch_next(hart_id) else {
            core::hint::spin_loop();
            continue;
        };

        // 记录当前线程/进程
        CURRENT_THREAD.set(&mut thread as *mut _ as usize);
        CURRENT_PROC_PID.set(pid.get_usize());

        // 设置时间片并切换到用户态
        tg_sbi::set_timer(time::read64() + QUANTUM);
        unsafe { thread.context.execute(portal, TpReg) };

        // ─── Trap 分发 ───
        match scause::read().cause() {

            // ① Timer：时间片耗尽，强制切换
            Trap::Interrupt(Interrupt::SupervisorTimer) => {
                tg_sbi::set_timer(u64::MAX);  // 清除 pending
                SMP_PROCESSOR.suspend(tid, thread, pid, hart_id);
            }

            // ② Syscall
            Trap::Exception(Exception::UserEnvCall) => {
                let ctx = &mut thread.context.context;
                ctx.move_next();
                let id: Id = ctx.a(7).into();
                let args = [ctx.a(0), ctx.a(1), ctx.a(2), ctx.a(3), ctx.a(4), ctx.a(5)];
                let ret = tg_syscall::handle(Caller { entity: 0, flow: 0 }, id, args);

                // 信号检查
                let signal_result = SMP_PROCESSOR.with_current_proc(|proc| {
                    proc.signal.handle_signals(ctx)
                });
                use tg_signal::SignalResult;
                match signal_result {
                    SignalResult::ProcessKilled(code) => {
                        SMP_PROCESSOR.thread_exit(tid, pid, code as _);
                    }
                    _ => match ret {
                        Ret::Done(ret_val) => match id {
                            Id::EXIT => {
                                SMP_PROCESSOR.thread_exit(tid, pid, ret_val);
                            }
                            // 同步原语阻塞
                            Id::SEMAPHORE_DOWN | Id::MUTEX_LOCK | Id::CONDVAR_WAIT => {
                                *ctx.a_mut(0) = ret_val as _;
                                if ret_val == -1 {
                                    SMP_PROCESSOR.block_thread(tid, thread, pid);
                                } else {
                                    SMP_PROCESSOR.suspend(tid, thread, pid, hart_id);
                                }
                            }
                            _ => {
                                *ctx.a_mut(0) = ret_val as _;
                                SMP_PROCESSOR.suspend(tid, thread, pid, hart_id);
                            }
                        },
                        Ret::Unsupported(_) => {
                            SMP_PROCESSOR.thread_exit(tid, pid, -2);
                        }
                    }
                }
            }

            // ③ 其他异常：杀死线程
            e => {
                tg_console::log::error!("[hart {hart_id}] tid {} trap {e:?}", tid.get_usize());
                SMP_PROCESSOR.thread_exit(tid, pid, -3);
            }
        }

        // 清除当前指针（防止悬垂）
        CURRENT_THREAD.set(0);
    }
}
```

### 3.4 `tg-rcore-tutorial-ch8/src/main.rs` 的修改点

```rust
// 1. 条件编译：SMP 模式和单核模式使用不同的调度模块
#[cfg(not(feature = "smp"))]
mod processor;
#[cfg(feature = "smp")]
mod smp;

// 2. _start 中增加 "mv tp, a0"（传递 hart_id）

// 3. rust_main 中：
//    - portal_slots 根据 feature 动态决定
//    - SMP 模式：初始化 SMP_PROCESSOR，boot_secondary()，进入 schedule_loop()
//    - 单核模式：与 feat/T4 完全相同

// 4. 单核调度循环增加 timer 分支（不需要 SMP 也应该有 preemption）：
// 在 match scause 中增加：
Trap::Interrupt(Interrupt::SupervisorTimer) => {
    tg_sbi::set_timer(u64::MAX);
    unsafe { (*processor).make_current_suspend() };
}
// 并在 execute() 前设置 timer：
tg_sbi::set_timer(time::read64() + QUANTUM);
unsafe { sie::set_stimer() };
```

### 3.5 `tg-rcore-tutorial-ch8/src/processor.rs` 的修改点

单核模式需要增加 `need_reschedule` 标志（为内核抢占预留接口）：

```rust
// 在 Processor 中增加：
pub fn set_need_reschedule(&self) { /* 原子标志 */ }
pub fn check_and_clear_reschedule(&self) -> bool { /* 原子检查+清除 */ }
```

### 3.6 SyscallContext 的 SMP 适配

`SyscallContext` 中访问 `PROCESSOR.get_mut()` 的代码需要在 SMP 模式下改为访问 `SMP_PROCESSOR`：

```rust
// 通过条件编译隔离：
#[cfg(not(feature = "smp"))]
fn current_proc() -> &'static mut Process {
    PROCESSOR.get_mut().get_current_proc().unwrap()
}

#[cfg(feature = "smp")]
fn current_proc() -> &'static mut Process {
    // 通过 CURRENT_PROC_PID 找到当前进程
    // 需要持有 SmpState 锁
}
```

**关键点**：同步原语的 `wakeup`（`re_enque`）需要改为 `SMP_PROCESSOR.wakeup_thread(tid)`。

---

## 四、用户态测试用例设计

### 4.1 SMP 功能验证

#### `smp_hello.rs` — 验证多核调度基本可用
```rust
// 创建 4 个线程，每个打印自己的 TID 和 hart_id
// 预期输出：每个线程都能运行，hart_id 有 0 和 1 两种（双核分担）
fn main() {
    for _ in 0..4 {
        thread_create(worker, 0);
    }
    // waittid 等待所有线程退出
}
fn worker(_: usize) -> i32 {
    let tid = gettid();
    let hart = get_hart_id();  // 新增 syscall SYS_GETHARTID
    println!("tid={} hart={}", tid, hart);
    0
}
```

预期输出验证点：
- 所有 4 个线程都完成
- hart_id 同时出现 0 和 1（证明真正并行）

#### `smp_mutex_test.rs` — 验证跨核互斥锁正确性
```rust
// 两个线程在不同核上并发对共享计数器做 100_000 次 +1
// 不用锁预期出现竞争导致结果小于 200_000
// 用锁后预期精确等于 200_000
static COUNTER: AtomicI32 = AtomicI32::new(0);

fn with_mutex_test() {
    let mutex = mutex_create(true);
    for _ in 0..2 {
        thread_create(|_| {
            for _ in 0..100_000 {
                mutex_lock(mutex);
                COUNTER.fetch_add(1, Relaxed);
                mutex_unlock(mutex);
            }
            0
        }, 0);
    }
    // 等待所有线程
    assert_eq!(COUNTER.load(Relaxed), 200_000, "mutex failed!");
    println!("smp_mutex_test OK: counter={}", COUNTER.load(Relaxed));
}
```

#### `smp_semaphore_test.rs` — 生产者消费者
```rust
// 生产者线程（hart 0）写 100 个消息到共享缓冲区
// 消费者线程（hart 1）读取并验证
// 使用信号量同步
// 预期：消费者收到所有 100 条消息，顺序正确
```

#### `smp_condvar_test.rs` — 条件变量跨核唤醒
```rust
// 线程 A 等待条件变量
// 线程 B（可能在另一核）触发信号
// 验证 A 能被正确唤醒
```

### 4.2 内核中断（Preemption）验证

#### `preemption_test.rs` — 不主动让权的线程能被抢占
```rust
// 线程 A：死循环计数，不调用 sched_yield()
// 线程 B：每隔 100_000 次循环打印进度
// 如果内核态中断生效，B 能被调度到，能打印输出
// 如果没有内核中断（单核 + 无 preemption），B 永远不会运行
fn greedy_counter(limit: usize, id: usize) {
    let mut count = 0usize;
    loop {
        count = count.wrapping_add(1);
        if count % 1_000_000 == 0 {
            println!("thread {} count {}", id, count);
        }
        if count >= limit { break; }
    }
}

fn main() {
    thread_create(|_| { greedy_counter(5_000_000, 0); 0 }, 0);
    thread_create(|_| { greedy_counter(5_000_000, 1); 0 }, 0);
    // 预期：两个线程的输出交替出现（抢占有效）
    // 错误：只有 thread 0 的输出（无抢占，thread 1 饿死）
}
```

#### `timer_accuracy_test.rs` — 验证时钟精度
```rust
// 调用 clock_gettime 记录开始时间
// 在一个 for 循环中多次 sched_yield() + gettime
// 验证时间单调递增且间隔合理（约 10ms per quantum）
```

### 4.3 新增 Syscall：`SYS_GETHARTID`

| 属性 | 值 |
|------|-----|
| syscall 号 | `syscall_id = 500`（在 tg_syscall 中新增） |
| 功能 | 返回当前线程所在的 hart id |
| 实现 | `tg_smp::hart::hart_id()` |
| 用途 | 测试用例验证线程确实在不同核上运行 |

```rust
// tg-rcore-tutorial-syscall/src/lib.rs 中新增：
pub enum SyscallId {
    // ... 现有 ...
    GET_HARTID = 500,
}

// 内核实现（impls 中）：
fn get_hart_id(&self, _caller: Caller) -> isize {
    #[cfg(feature = "smp")]
    { tg_smp::hart::hart_id() as isize }
    #[cfg(not(feature = "smp"))]
    { 0 }
}

// 用户库（tg-rcore-tutorial-user/src/lib.rs 中）：
pub fn get_hart_id() -> usize {
    syscall(SyscallId::GET_HARTID, [0, 0, 0, 0, 0, 0]) as usize
}
```

---

## 五、性能对比应用

### 5.1 矩阵乘法基准（`matrix_bench.rs`）

展示双核并行带来的计算加速：

```
设计参数：
  矩阵大小：128×128（足够大以看到差异，足够小以快速完成）
  单线程版本：主线程完成全部计算
  双线程版本：线程 0 计算前 64 行，线程 1 计算后 64 行
  同步：用信号量确保两个线程都完成后主线程验证结果

预期输出：
  [matrix_bench] single-thread: 1250ms
  [matrix_bench] dual-thread:    680ms
  [matrix_bench] speedup: 1.84x
  [matrix_bench] result: CORRECT
```

关键实现要点：
- 结果矩阵放在共享内存（进程地址空间内）
- 每个线程只写自己负责的行，无竞争，无需锁
- 验证阶段在主线程串行执行

### 5.2 生产者-消费者吞吐量基准（`producer_consumer_bench.rs`）

展示双核 I/O 密集型场景的吞吐量提升：

```
配置 1（单核 baseline）：
  1 个生产者线程 + 1 个消费者线程
  共享 ring buffer（大小 256），信号量同步

配置 2（双核 parallel）：
  2 个生产者 + 2 个消费者
  同一 ring buffer，mutex 保护入队/出队

测量：在固定时间（1秒）内处理的消息总数

预期输出：
  [bench] single-producer-consumer: 45,000 msg/s
  [bench] dual-producer-consumer:   82,000 msg/s
  [bench] throughput gain: 1.82x
```

### 5.3 Doom 帧率对比（`doom_benchmark`）

在 Doom stub 的基础上设计帧率测量：

```
场景：
  - 主渲染线程：每帧计算 320×200 像素的渐变色（模拟渲染负载）
  - 输入处理线程：轮询 input_event()（独立线程，不阻塞渲染）

测量：
  - 单核模式：渲染 + 输入在同一核轮流执行
    → 渲染被输入轮询打断，帧率不稳定
  - 双核模式：渲染在 hart 0，输入在 hart 1
    → 渲染线程不被打断，帧率更稳定

指标：
  帧时间方差（variance of frame time）
  双核 variance 明显低于单核
```

---

## 六、常见 Bug 与防坑指南

### Bug 1：副核使用了错误的 portal slot

**现象**：副核执行用户程序时卡死或 page fault。

**原因**：`MultislotPortal::execute()` 使用 slot index 区分各核的 portal 状态。如果两个核共用 slot 0，会相互覆盖。

**修复**：确保 `portal_slots = NUM_CPUS = 2`，并且 `execute()` 调用时传入正确的 `TpReg`（通过 tp 寄存器传递 hart_id 作为 slot index）。

### Bug 2：SmpState 锁在 syscall 中死锁

**现象**：双核下某个 syscall 执行后内核卡死。

**原因**：`SyscallContext` 在持有 `SmpState` 锁的情况下调用了另一个需要锁的函数（如 `wakeup_thread` 内部再次尝试 `state.lock()`）。

**修复**：`SmpState` 锁的持有时间要尽可能短——取出数据、释放锁、再处理数据。避免在锁内调用任何可能再次需要锁的函数。

### Bug 3：`CURRENT_THREAD` 指针在线程被 move 后悬垂

**现象**：syscall 中访问 `current_thread` 指向的内容出错（数据损坏）。

**原因**：`thread` 是局部变量，`execute()` 后传给 `SmpState.suspend()` 时被 move 进 BTreeMap，而 `CURRENT_THREAD` 仍指向旧栈地址。

**修复**：在 `execute()` 返回后、修改线程数据前，将 `CURRENT_THREAD` 用来读取寄存器值（如读出 a7、a0~a5）；读完后立即清除指针。不要在 `suspend()`/`exit()` 之后再解引用 `CURRENT_THREAD`。

### Bug 4：单核模式的 `timer` 中断处理与 SMP 模式不一致

**现象**：`--features smp` 时时间片工作正常，不加时却不抢占。

**原因**：只在 `smp.rs` 中设置了 `sie::set_stimer()`，单核路径的 `main.rs` 调度循环忘记设置。

**修复**：两条路径都要在循环开始前 `unsafe { sie::set_stimer(); }` 并在每次 `execute()` 前 `set_timer(now + QUANTUM)`。

### Bug 5：`SpinNoIrq` 嵌套死锁

**现象**：中断处理中尝试获取已被持有的 `SpinNoIrq` 锁时内核无响应。

**原因**：`SpinNoIrq::lock()` 在获取锁前先 `clear_sie()`，但如果当前核已经持有该锁（在 syscall 处理中），发生 timer 中断后又尝试 `lock()` 就会自旋死锁。

**修复**：确保 timer 中断处理路径（在 `schedule_loop` 的 `SupervisorTimer` 分支）不需要持有 `SmpState` 锁——只调用 `suspend()`（会短暂获取锁），且此时外层没有持有者。

---

## 七、构建与验证

### 7.1 构建命令

```bash
cd tg-rcore-tutorial-ch8

# 单核 + 内核抢占（最小改动）
cargo build --release

# 双核 SMP
cargo build --release --features smp

# QEMU 运行（单核）
cargo run --release

# QEMU 运行（双核）
QEMU_EXTRA="-smp 2" cargo run --release --features smp
```

### 7.2 验证步骤

| 步骤 | 命令 | 预期 |
|------|------|------|
| 1. 单核不破坏现有功能 | `cargo run` | 与 feat/T4 输出相同 |
| 2. 内核 timer 抢占 | 运行 `preemption_test` | 两个贪心线程交替输出 |
| 3. 双核启动 | `--features smp`，看日志 | `[hart 0] booting hart 1` + `[hart 1] ready` |
| 4. SMP 并行 | 运行 `smp_hello` | hart_id 同时出现 0 和 1 |
| 5. 锁正确性 | 运行 `smp_mutex_test` | `counter=200000 OK` |
| 6. 性能提升 | 运行 `matrix_bench` | dual-thread 加速 >1.5x |

### 7.3 QEMU 配置（`Makefile` 或 `.cargo/config.toml`）

```makefile
# 默认单核
run:
    qemu-system-riscv64 \
        -machine virt \
        -nographic \
        -bios none \
        -kernel target/riscv64gc-unknown-none-elf/release/$(CRATE)

# SMP 双核
run-smp:
    qemu-system-riscv64 \
        -machine virt \
        -nographic \
        -bios none \
        -smp 2 \
        -kernel target/riscv64gc-unknown-none-elf/release/$(CRATE)
```

---

## 八、实施顺序建议

1. **第一步**（最小可验证）：单核 + timer 中断抢占
   - 修改 `main.rs` 调度循环加入 `SupervisorTimer` 分支
   - 运行 `preemption_test` 验证抢占生效
   - 预计代码改动：< 30 行

2. **第二步**：移植 SMP 基础设施
   - 复制 `tg-rcore-tutorial-smp` crate
   - 复制增强 SBI（HSM 支持）
   - 修改 `Cargo.toml` 添加 `smp` feature
   - 预计代码改动：纯复制，零风险

3. **第三步**：实现 `smp.rs` 调度器
   - 参考 ch5 的 `smp.rs`，扩展为线程粒度
   - 实现 `SmpProcessor` 的 CRUD 操作
   - 实现 `boot_secondary` + `schedule_loop`

4. **第四步**：适配 `SyscallContext`
   - 最复杂的部分，需要将所有 `PROCESSOR.get_mut()` 换为 SMP 路径
   - 重点：同步原语的唤醒路径必须用 `SMP_PROCESSOR.wakeup_thread()`

5. **第五步**：用户态测试 + 基准程序
   - 从简单到复杂：`smp_hello` → `smp_mutex_test` → `matrix_bench`

---

## 九、教学价值说明

| 功能 | 教学价值 | 难度 |
|------|---------|------|
| Timer 中断抢占 | 理解内核为何需要"抢占"：不依赖用户合作 | ★★☆ |
| Per-CPU 就绪队列 | 理解 work-stealing 调度器的核心思想 | ★★★ |
| SpinNoIrq | 理解为何多核锁必须同时禁中断 | ★★★ |
| Portal 多槽位 | 理解地址空间切换在多核下的特殊挑战 | ★★★★ |
| 同步原语跨核唤醒 | 理解 Mutex/Semaphore 在多核下的复杂性 | ★★★★★ |

---

## 十、2026-04-01 调试记录

本轮联调主要定位出两类实质性问题，并做了最小修复与插桩，便于后续 agent 继续追踪。

### 1. 单核线程饥饿

- 现象：`threads` / `threads_arg` 在单核模式下会卡住，`waittid` 长时间收不到部分线程的退出码。
- 根因：`smp.rs` 即使在非 `smp` feature 下也按 2 个 CPU 分发线程，一部分线程被放进 `ready[1]`，但单核模式没有 hart1 去消费，只有 `ready[0]` 彻底空掉时才可能被偷取，导致严重饥饿。
- 修复：增加 `ACTIVE_CPUS` 常量；非 `smp` 模式固定为 1，`add_thread` / `wakeup_thread` / `fetch_for` 都按 `ACTIVE_CPUS` 工作。
- 结果：`threads_arg`、`threads` 单独运行都已通过。

### 2. SMP 副核启动卡死

- 现象：QEMU `-smp 2` 时卡在 `[hart 0] booting hart 1`。
- 第一层根因：副核在 M 态 `park_secondary` 使用 `wfi`，但没有完整的机器态软中断使能，导致不会被 CLINT MSIP 可靠唤醒。
- 第二层根因：`MSBI_HART_STATE` 被置位后，副核可能先看到状态，再去读 `MSBI_HART_START_ADDR` / `MSBI_HART_OPAQUE`，存在启动参数发布顺序窗口。
- 修复：
  - 去掉 `park_secondary` 中的 `wfi`，改为忙等。
  - 副核离开 park 前，额外等待 `start_addr` 和 `opaque` 都变为非零。
- 调试标记：
  - `H`：主核 `hart_start` ecall 已进入 `msbi::handle_hart_start`
  - `P`：副核已离开 `park_secondary`
  - `S`：已进入 `_secondary_start`
- 当前结果：可看到 `HPS`，随后打印 `[hart 1] secondary started` 和 `[hart 0] hart 1 ready`，说明副核启动链已打通。

### 3. 当前保留的调试信息

- `tg-rcore-tutorial-ch8/src/main.rs`
  - `thread_create`
  - `waittid`
- `tg-rcore-tutorial-ch8/src/smp.rs`
  - `add_thread`
  - `thread_exit`
  - `secondary_rust_main`
- `tg-rcore-tutorial-sbi/src/msbi.rs`
  - `handle_hart_start` 输出 `H`
- `tg-rcore-tutorial-sbi/src/m_entry.asm`
  - `park_secondary` 输出 `P`
- `tg-rcore-tutorial-ch8/src/smp.rs`
  - `_secondary_start` 输出 `S`

### 4. 已验证的最小用例

- 单核：
  - `threads_arg` 通过
  - `threads` 通过
- 双核：
  - `smp_hello` 通过，输出显示 hart0 / hart1 都拿到了线程

### 5. 后续收尾修复

- 修正 `wait()` 对 `ProcRel` 返回值的解释：
  - `None` 表示没有子进程，返回 `-1`
  - `ProcId(-2)` 表示仍在运行，返回 `-2`
- 修正阻塞 syscall 唤醒竞态：
  - 新增 `wake_returns`，解决“跨核唤醒先于 block_thread 入表”时返回值丢失的问题
- 将 `tg-sync` 内部状态从 `UPIntrFreeCell<RefCell<_>>` 切换为 `spin::Mutex<_>`：
  - `MutexBlocking`
  - `Semaphore`
  - `Condvar`
- 补齐 `input_try_getchar()`，使 `doom_benchmark` 不再因为未实现 syscall 崩溃
- 修正 `doom_benchmark` 的 debug 模式整数溢出

### 6. 最终验收结论

- 单核 `ch8b_usertest` 已完整跑通，最终打印 `Basic usertests passed!`
- 双核 `t4_usertest` 已跑通：
  - `smp_hello`
  - `preemption_test`
  - `smp_mutex_test`
  - `matrix_bench`
  - `doom_benchmark`

### 7. 2026-04-03 owner 校验补充

- 目标：禁止“非 owner 线程执行 `mutex_unlock`” silently 破坏锁状态。
- 设计落点：仅放在 `ch8` syscall 层，不修改 `tg-sync::Mutex` trait。
- 实现方式：
  - `Process::mutex_owner` 改为始终维护，不再只在 deadlock detect 打开时有效。
  - `mutex_lock` 成功后无条件调用 `record_mutex_lock()`。
  - `condvar_wait` / `mutex_unlock` 释放锁后无条件调用 `record_mutex_unlock()`，保持 owner 状态准确。
  - `mutex_unlock` syscall 先检查 `current_tid` 是否等于 `mutex_owner[mutex_id]`，若不是则直接返回 `-1`。
- 新增回归：
  - `mutex_owner_test`
  - 场景：线程 A 持锁，线程 B 非 owner 解锁应返回 `-1`，之后 A 仍可正常解锁，B 再次加锁成功。
- 本轮验证：
  - `tg-rcore-tutorial-user` 交叉编译通过，包含 `mutex_owner_test`
  - `tg-rcore-tutorial-ch8` 在 `TG_SKIP_USER_APPS=1` 和 `TG_SKIP_USER_APPS=1 --features smp` 下均可通过 `cargo check`
  - 完整 `ch8` 镜像构建当前会被既有 `build.rs` / `easy-fs` 打包问题阻塞，这不是 owner 校验引入的新问题
