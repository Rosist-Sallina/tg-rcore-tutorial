# T2L10 设计报告

## 1. 目标

本实验为 `ch3` 到 `ch5` 增加基础的双核支持，重点包括：

- 副核启动
- Per-CPU 数据组织
- 单核同步原语向多核同步原语迁移
- `ch4` 的共享页表切换
- `ch5` 的进程并发调度与父子进程关系维护

## 2. 这次采用的运行环境

这次最终没有继续使用 `-bios none` 路径做实验运行，而是改成让 QEMU 使用默认 OpenSBI。

原因很直接：

- `-bios none` 下的教学最小 SBI 只适合单核
- 真正多核需要可靠的 HSM / IPI 语义
- 直接接 OpenSBI，副核启动更稳定，调试成本更低

对应改动：

- 三章 build.rs 改用 `tg_linker::SCRIPT`
- 三章 `tg-sbi` 依赖移除 `nobios`
- QEMU 运行参数改为默认固件 + `-smp 2`

## 3. 基础设施

新增 crate：

- `tg-rcore-tutorial-smp`

包含：

- `hart.rs`：hart id、启动、等待
- `percpu.rs`：Per-CPU 存储
- `spin.rs`：`SpinLock` / `SpinNoIrq`
- `ipi.rs`：共享 pending 位 + SBI sPI
- `barrier.rs`：内存屏障

同时升级：

- `tg-rcore-tutorial-sync`

做法是保留 `UPIntrFreeCell` 接口不变，但在 `smp` feature 下把内部实现切成 `SpinNoIrq<T>`。

## 4. 各章实现

### ch3

- 入口把 `tp` 设成 `hart_id`
- 主核加载用户程序后按范围分给两个核
- 剩余任务数改成原子变量
- 副核独立进入自己的调度循环

### ch4

- Portal slot 从 1 扩成 2
- 主核记录内核 `satp`
- 副核启动后先写 `satp` 再 `sfence.vma`
- 进程表改成 `SpinLock<Vec<Process>>`
- syscall 通过 Per-CPU 当前进程表指针访问已上锁的进程数组

### ch5

- 单核 `processor.rs` 在 `smp` 下不再参与调度
- SMP 路径单独维护：
  - 全局进程表
  - 父子关系表
  - 每核就绪队列
  - Per-CPU 当前运行进程指针
- 当前运行进程从全局表中取出，Trap 后再放回或回收

## 5. 容易出错的点

### 5.1 主核不一定是 hart 0

OpenSBI 的 boot hart 在 QEMU 里不保证固定为 0。

所以这次不能写死：

- “hart 0 是主核”
- “hart 1 是副核”

修法是统一用：

- `current = hart::hart_id()`
- `other = 1 - current`

### 5.2 单核 `current` 语义在多核下直接失效

`ch5` 原来的 `PROCESSOR.current()` 默认只有一份。多核下必须改成每核独立的当前进程。

### 5.3 用户态输入不能把 `usize::MAX` 当字符

OpenSBI 下 `console_getchar()` 没有输入时会返回 `-1`，如果直接转 `u8`，shell 会被喂满 `0xff`。

这次把 `read` 改成了真正阻塞读。

## 6. 当前验证结果

已经完成：

- `cargo check --features smp` for `ch3/ch4/ch5`
- `cargo run --features smp` for `ch3/ch4/ch5`

观察结果：

- `ch3`：双核可以同时推进多个用户程序
- `ch4`：副核能够激活共享内核页表并进入用户程序调度
- `ch5`：双核下能够启动 shell，并稳定到交互提示符
