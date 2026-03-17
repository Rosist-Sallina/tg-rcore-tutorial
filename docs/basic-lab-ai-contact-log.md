# 这里是基础测试与AI的对话过程全文

*我使用的模型和辅助工具为: Codex Cli: GPT5.4-XHigh*

## ch3

**用户：**

我正在学习操作系统，请逐步帮我实现ch3（我不要直接的实现，请以教程的方式帮助我）

**AI：**

行，老哥。这次我按“教程带做”的方式来，而且直接围绕 `ch3 exercise` 讲，不绕基础题外话。你最终要做的不是重写整个 `ch3`，而是在现成的多任务内核上补 `sys_trace`，让 `ch3_trace` 这个用户程序过掉。

先把题目压缩成一句话：  
在 `tg-rcore-tutorial-ch3` 里，为“当前任务”增加 syscall 调用统计，并实现 `trace` 的 3 种行为：

1. 读当前任务地址里的 1 字节
2. 向当前任务地址写 1 字节
3. 查询某个 syscall 号已经调用了多少次，而且本次 `trace` 也要算进去

你这题只要盯 2 个文件：
- [main.rs](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/main.rs)
- [task.rs](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/task.rs)

但真正该动手的核心，基本都在 [task.rs](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/task.rs)。

**用户：**

那完整实现路线是什么？你先给我一个全方案。

**AI：**

可以，直接给你完整教程版路线。

### 第 1 步：先把题目的调用链看明白

从用户态看，`ch3_trace.rs` 最后会走到这三类调用：
- `trace_read(ptr)` 对应 `trace(0, ptr, 0)`
- `trace_write(ptr, value)` 对应 `trace(1, ptr, value)`
- `count_syscall(id)` 对应 `trace(2, id, 0)`

也就是说你内核里只要正确处理 `trace_request = 0/1/2` 三种情况就够了。

### 第 2 步：在 TCB 里放 syscall 统计状态

在 [task.rs](/Users/sallinarosist/OS/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/task.rs) 的 `TaskControlBlock` 里新增一个 syscall 统计表。

但这里别本能地开大数组。原因是 `rust_main()` 里当前是：
- `let mut tcbs = [TaskControlBlock::ZERO; APP_CAPACITY];`

也就是说整组 TCB 是放在内核栈上的。  
如果你给每个 TCB 都塞一个几百上千项的 `usize` 数组，32 个任务一乘，内核栈很容易被你自己撑爆。

所以更稳妥的做法是：
- 用一个小型稀疏表
- 只记录当前测试真正会出现的 syscall 种类

比如像这样：
- `[(syscall_id, count); 8]`
- 空槽位用 `usize::MAX` 标记

当前 `exercise` 实际关心的 syscall 也就：
- `CLOCK_GETTIME`
- `TRACE`
- `WRITE`
- `SCHED_YIELD`
- `EXIT`

8 个槽位绰绰有余。

### 第 3 步：初始化时清空统计表

你要同时处理两个地方：
- `TaskControlBlock::ZERO`
- `TaskControlBlock::init()`

这样每个任务第一次初始化时统计都是干净的，不会串数据。

### 第 4 步：在 `handle_syscall()` 最前面先计数

一进入 `handle_syscall()`：
1. 先从 `a7` 取 syscall id
2. 先给这个 id 记一次数
3. 然后再做后续分发

这个顺序不能反。因为题目明确写了：
- `trace_request == 2` 查询 syscall 次数时
- “本次调用也计入统计”

也就是说你如果先查再加，`count_syscall(SYS_TRACE)` 一定少 1。

### 第 5 步：在 `handle_syscall()` 里本地拦截 `TRACE`

不要让 `TRACE` 继续走 `tg_syscall::handle(...)`，而是在 `handle_syscall()` 里单独判断：

- 如果 `id == TRACE`
  - `trace_request == 0`：把 `id` 当 `*const u8` 读 1 字节，返回这个字节值
  - `trace_request == 1`：把 `id` 当 `*mut u8`，写入 `data as u8`，返回 `0`
  - `trace_request == 2`：返回“目标 syscall id”的累计次数
  - 其他值：返回 `-1`

这一步题目已经明确说了，不要求做安全检查，所以这里直接 `unsafe` 读写裸指针就行。

### 第 6 步：按普通 syscall 的方式收尾

本地处理完 `TRACE` 后，别忘了跟普通 syscall 一样做 2 件事：
- 把返回值写回 `a0`
- 调 `move_next()` 跳过 `ecall`

最后返回 `Event::None`。

否则用户态会重新执行同一条 `ecall`，你会看到程序像卡死一样不停陷回内核。

### 第 7 步：验证

先跑：

```bash
cd tg-rcore-tutorial-ch3
cargo run --features exercise
```

你应该能看到这些关键信号：
- `get_time OK!`
- `Test sleep1 passed!`
- `string from task trace test`
- `Test trace OK!`

然后再跑：

```bash
./test.sh base
./test.sh exercise
```

如果环境没拦你，最后应该分别看到：
- `✓ ch3 基础测试通过`
- `✓ ch3 练习测试通过`

**用户：**

为啥`count_syscall(SYS_TRACE)` 一直缺1，review一下

**AI：**

先查 `handle_syscall()` 里的顺序。

你要确认是不是这样：

1. 读出当前 syscall id
2. 立刻 `record_syscall(id)`
3. 然后如果它是 `TRACE`，再进入 `trace_request == 2` 的查询逻辑

如果你把第 2 步放在查询后面，就会稳定少 1。  
这不是随机 bug，是逻辑顺序错了。

**用户：**

那 `main.rs` 里的 `impl Trace for SyscallContext` 放着没关系吗

**AI：**

从这题的最小正确实现来说，不需要依赖它。

你可以保留那个占位实现不动，因为当前 `TRACE` 已经在 `TaskControlBlock::handle_syscall()` 里被本地拦截掉了，根本不会继续掉进 `tg_syscall::handle()` 那层。

这也是为什么我前面说：  
真正的状态在当前任务里，所以处理点应该在 TCB，不在全局 syscall context。
