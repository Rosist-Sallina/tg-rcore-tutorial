# 任务四：原理掌握与实践能力考核总结报告

## 一、与 AI 合作的实现过程

机考题目要求在已完成的 ch8（线程 + 同步原语 + DOOM）基础上，扩展内核态中断响应和多核处理两项功能。使用工具为 Codex CLI（GPT-5.4-XHigh），分支为 `feat/T4-smp`。

### 整体架构决策

一开始有两个路线选择：从 ch1 开始逐步实现，或者直接在 ch8 上扩展。选择了直接在 ch8 上做，理由是 ch8 已经有完整的线程和同步原语，在这个基础上做 SMP 更能体现"内核能力的叠加"，而从 ch1 开始做更像在重复已经做过的事情。

因为T2L10已经基本完成了可以复用的SMP支持，也基本上不用再从零实现一套方案。

因此 AI 对这个选择给出了很明确的实施方向：参考仓库里 `t2l10-redo` 分支上已经完成的 ch3~ch5 级别的 SMP 实现，把其中的 SBI 启动链路（HSM 支持）和 SMP crate 复用到 ch8，而不是从零写一套。

### 内核态 Timer 中断（时间片抢占）

这部分是"内核态响应中断"的具体实现。在 ch8 原有的调度循环里，执行用户程序时没有任何强制切换机制——线程只有主动 yield 或者 syscall 结束才会让出 CPU。加入 timer 中断之后，调度循环变成这样的结构：

进入 execute 前先设置时间片（`set_timer(now + QUANTUM)`），execute 返回时检查 scause，如果是 `SupervisorTimer` 就说明时间片到了，强制把当前线程挂回就绪队列并切换到下一个。

具体做法是在 S 模式调度循环中打开 `sie.stimer`，并在 scause 分支里增加 `SupervisorTimer` 处理。内核本身运行时保持 `SIE=0`，只在"等待用户程序 execute"期间才能接收 timer 中断，避免内核执行路径被中断打断导致崩溃。

AI 指出：`QUANTUM` 取 12500（与 ch3 保持一致），对应 QEMU 的 `timebase_frequency = 12500000` 下大约 1ms 的时间片。

### 多核支持（SBI HSM + 线程级 SMP 调度）

SBI 这部分：t2l10-redo 分支上的 `msbi.rs` 和 `m_entry.asm` 已经基本实现了 HSM（Hart State Management）的 M-mode 支持，把这两个文件合并进 `tg-rcore-tutorial-sbi`，就获得了"S 模式通过 SBI ecall 启动副核"的能力。HSM 启动流程是：S 模式发 SBI ecall → M 模式 `handle_hart_start` 把目标 hart 状态标记为 `START_PENDING` 并通过 CLINT 软件中断唤醒它 → 副核 M 模式陷入中断 → 读取 start_addr/opaque → mret 进入副核 S 模式入口。

调度器这部分：ch5 级别的 SMP 以进程为调度单元，但 ch8 已经把调度单元细化到了线程（Thread），所以需要把就绪队列改成 `[VecDeque<ThreadId>; NUM_CPUS]` 的形式，每个 hart 维护自己的就绪队列，并实现基本的 work-stealing（自己队列空时从其他核偷线程）。

所有进程和线程状态放在一个 `SpinNoIrq<SmpState>` 保护的全局结构里，保证跨核访问的正确性。进程对象改成 `BTreeMap<ProcId, Box<Process>>`，用 Box 固定地址，避免并发增删时裸指针悬垂。

### 遇到的主要问题

第一个问题是 Rust 的类型约束。`Process` 结构体本身没有实现 `Send`，但 SMP 下需要跨核传递它的引用，编译器报错。解法是添加 `unsafe impl Send for Process`，并在注释里说明这是"已知在全局锁保护下才会跨核访问，所以是安全的"。

第二个问题是 condvar 的 `wait_with_mutex()` 实现有一个缺陷：当时的实现只做了解锁再重新加锁，没有真正把线程挂进 `wait_queue`，导致 `condvar_signal()` 失去作用。这个问题是 AI 在查看代码时发现的，修复是让 `wait_with_mutex` 把当前线程挂进等待队列，然后释放锁并切换调度，等 signal 把它唤醒时再重新加锁。

第三个问题是 `sys_openat` 里文件名解析有 bug：当时的实现按字节数组长度读取文件名，会把末尾的 `\0` 也带进去，导致 `filetest_simple` 打开文件时找不到。这个 bug 在单核测试里一直存在，只是在 SMP 调试时才被 AI 发现。修复是改成"逐字节读到 `\0` 为止"。

第四个问题是 M-mode timer 的多核问题。`handle_timer()` 原来只写了 `CLINT_MTIMECMP`（hart 0 的比较寄存器），在 SMP 下副核的 timer 中断无法正常触发。修复是改成 `CLINT_MTIMECMP + 8 * mhartid`，让每个 hart 写自己的 mtimecmp 寄存器。

### 用户态测试设计

新增了几个测试和基准程序：
- `preemption_test`：两个线程各自空转不 yield，单核下验证 timer 抢占生效（如果没有抢占，第二个线程永远跑不到）
- `smp_hello`：创建多线程打印 `tid + hart_id`，验证 hart_id 0 和 1 都出现
- `smp_mutex_test`：跨核并发更新共享计数器，无锁版和加锁版对比，加锁后结果精确
- `matrix_bench`：单线程 vs 双线程矩阵乘法，量化 SMP 的性能提升
- `t4_usertest`：汇总以上测例顺序运行，输出整体 pass/fail


## 二、对内核态中断和多核机制的理解

### 内核态 Timer 中断的机制

RISC-V 的中断分为 M-mode 和 S-mode 两层。SBI 负责 M-mode 的 timer 中断处理，当 `mtime >= mtimecmp` 时触发 M-mode timer 中断，SBI 把它转发为 S-mode 的 `SupervisorTimer` 中断。

在 ch8 的调度循环里，每次 execute 一个线程之前调用 `set_timer(now + QUANTUM)` 设置下次中断时间。当线程跑了足够时间，timer 中断触发，execute 返回（此时 `scause == SupervisorTimer`），内核清除 pending 状态（`set_timer(u64::MAX)`），然后强制挂起当前线程重新调度。

这里有一个细节：内核自己执行时要保持 `sstatus.SIE = 0`，否则内核代码执行到一半被 timer 中断，如果内核里有不可重入的全局状态（比如全局调度锁），就会出现死锁或数据损坏。只有在进入用户程序执行（execute）期间，timer 中断才是安全的，因为此时内核没有持有任何锁，中断到来时内核处于干净的切换点。

### 多核处理的机制

多核启动的核心是 Hart State Management。hart 0 先启动进入内核，通过 SBI ecall 发起 `hart_start(hartid=1, start_addr, opaque)` 请求，M-mode 收到后通过向 hart 1 发送 CLINT 软件中断将其唤醒，hart 1 在 M-mode 中读取 start_addr 和 opaque（用于传递副核的内核栈地址），然后 mret 进入 S-mode 的副核入口函数 `secondary_rust_main`。

副核在 `secondary_rust_main` 里初始化自己的地址空间视图（写 satp），然后初始化 portal，最后进入和主核相同的 `schedule_loop`。从这个点开始，两个核就都在跑各自的调度循环了。

保证正确性的关键是 `SpinNoIrq` 锁。所有跨核共享的状态（进程表、线程表、就绪队列）都被一把全局自旋锁保护，同一时刻只有一个核能修改这些状态。这带来了一个限制：syscall 路径是串行的，两个核不能同时处理 syscall。但对于这个教学内核来说，这是合理的简化——核心要展示的是"用户态计算可以真正并行"，而不是"内核也高度并发"。

## 三、学习效果评估

这个任务是整个课程里最麻烦的（奇奇怪怪的bug最多的），

对"内核态中断"的理解从之前的"S-mode 可以接中断，通过 scause 区分"提升到了"如何在不破坏内核执行安全性的前提下让 timer 中断驱动调度"。这里最关键的是理解为什么内核执行时必须关中断——不是因为中断在技术上不能发生，而是因为如果中断在持锁期间触发然后中断处理路径又试图加锁，就会发生死锁。

对"多核处理"的理解从"多个核可以并行"提升到了"如何让多个核安全地共享内核状态"。M-mode 的 Hart State Management 是一个我之前完全不了解的机制，通过这次实现理解了它的完整流程。`SpinNoIrq`（在持有锁期间关中断）也是一个很关键的设计，它保证了"即使 timer 中断来了，如果当前核正持有全局锁，也不会触发重入"。

局限是这次实现的 SMP 还是比较保守的：调度锁是全局的，不支持真正的无锁并发内核路径，副核也是固定双核，没有 N 核泛化。这些是真实操作系统（比如 ArceOS）在这个方向上更进一步的地方，作为教学内核的实现已经足够展示核心机制，但距离工业级实现还有一段距离。
