# chapter9 练习

## 任务一：懒分配与缺页

- 让 `mmap` 和 `sbrk` 只登记匿名区间，不立即分配物理页
- 处理 `LoadPageFault` / `StorePageFault` / `InstructionPageFault`
- 合法页访问恢复执行，非法页访问终止进程

建议先补：

- `src/process.rs::change_program_brk`
- `src/process.rs::mmap_anonymous`
- `src/process.rs::handle_page_fault`
- `src/pager.rs::handle_page_fault`

## 任务二：访问位与脏位

- 使用 RISC-V PTE 的 `A/D` 位
- 在扫描 victim 时读取并按算法需要清除 `A`
- 在淘汰时根据 `D` 决定是否回写

本任务默认不要求你重新设计 syscall 或 userland 输出协议，框架已经给好。

## 任务三：页面置换

- 先实现 FIFO
- 再实现 Clock

建议先补：

- `src/pager.rs::pick_fifo_index`
- `src/pager.rs::pick_clock_index`

## 扩展任务

- 实现 LRU 近似
- 实现简化 Working Set
- 做抖动检测与动态调参

## 任务四：统计与对比

- 输出缺页次数、缺页率、置换次数、脏页回写次数、抖动次数
- 用户态提供顺序扫描、随机访问、局部性强、局部性弱四种 pattern
- 单次运行输出多种算法对比表

## 验收

- `cargo run` 能看到章节脚手架正常启动
- 完成任务一到任务三后，`cargo run --features exercise` 应能输出算法对比表和 `VMBENCH PASS`
