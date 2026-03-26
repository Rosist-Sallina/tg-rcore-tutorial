# 第九章：虚拟内存算法

## 学生提交信息

- crate 名称：`rosist-sallina-tg-rcore-tutorial-T2L6`
- crate 版本：`0.0.1-preview.1`
- 依赖 crate：`rosist-sallina-tg-rcore-tutorial-kernel-vm-t2l6`
- 依赖 crate：`rosist-sallina-tg-rcore-tutorial-syscall-t3l3`
- 用户程序来源：构建时从 `rosist-sallina-tg-rcore-tutorial-user-t1l5` 注入 `vm_touch/vm_swap/vm_prot/vm_bench`
- 仓库地址：`https://github.com/Rosist-Sallina/tg-rcore-tutorial`
- 仓库页面：`https://github.com/Rosist-Sallina/tg-rcore-tutorial/tree/test/tg-rcore-tutorial-ch9`
- 建议 tag：`rosist-sallina-tg-rcore-tutorial-T2L6-v0.0.1-preview.1`

复现方式：

```bash
cargo clone rosist-sallina-tg-rcore-tutorial-T2L6
cd rosist-sallina-tg-rcore-tutorial-T2L6
./test.sh all
```

```bash
git clone https://github.com/Rosist-Sallina/tg-rcore-tutorial.git
cd tg-rcore-tutorial/tg-rcore-tutorial-ch9
./test.sh all
```

本章基于 `ch4` 的地址空间实现，继续补齐教学内核中的经典虚存算法链路：

- 匿名页懒分配
- 缺页异常处理
- 页面置换：FIFO / Clock / LRU-approx / Working Set
- 抖动检测与统计输出

## 学习目标

- 理解“页表映射”和“真正的按需分页”之间的差别
- 能在 RISC-V Sv39 上处理 `Load/Store/Instruction Page Fault`
- 理解 `A/D` 位在置换算法中的作用
- 用统一 benchmark 观察不同算法在不同访存局部性下的表现

## 练习入口

本章默认模式保留完整脚手架，方便先跑通章节；`cargo run --features exercise` 会切到练习测例集。

建议同学优先补这几处：

- `src/process.rs` 的 `change_program_brk`
- `src/process.rs` 的 `mmap_anonymous`
- `src/process.rs` 的 `handle_page_fault`
- `src/pager.rs` 的 `handle_page_fault`
- `src/pager.rs` 的 `pick_fifo_index`
- `src/pager.rs` 的 `pick_clock_index`

如果前面做完，再继续看 `LRU-approx / Working Set / thrashing` 的扩展实现。

## 目录结构

```text
tg-rcore-tutorial-ch9/
├── README.md
├── exercise.md
├── build.rs
├── test.sh
├── src
│   ├── main.rs
│   ├── pager.rs
│   └── process.rs
└── .cargo/config.toml
```

## 运行

基础演示：

```bash
cargo run
./test.sh base
```

算法对比：

```bash
cargo run --features exercise
./test.sh exercise
```

## Source Nav

- `src/main.rs`：内核启动、调度、trap 分发、`vmctl` 接口
- `src/process.rs`：进程地址空间、`mmap/sbrk`、缺页入口
- `src/pager.rs`：页状态、置换算法、统计和抖动检测
