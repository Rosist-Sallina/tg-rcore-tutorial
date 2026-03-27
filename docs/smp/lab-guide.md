# T2L10 实验说明

## 1. 编译

```bash
cd tg-rcore-tutorial-ch3
cargo check --features smp

cd ../tg-rcore-tutorial-ch4
cargo check --features smp

cd ../tg-rcore-tutorial-ch5
cargo check --features smp
```

## 2. 运行

```bash
cd tg-rcore-tutorial-ch3
cargo run --features smp

cd ../tg-rcore-tutorial-ch4
cargo run --features smp

cd ../tg-rcore-tutorial-ch5
cargo run --features smp
```

## 3. 推荐观察点

### ch3

- 主核和副核是否都进入调度
- 不同应用是否会交错执行
- 任务结束数是否正确归零

### ch4

- 副核是否打印页表激活完成日志
- 两个 hart 是否都能运行用户程序
- `mmap/sbrk` 相关程序是否还能跑

### ch5

- 是否能进到 `Rust user shell`
- 是否能稳定出现 `>>`
- 是否还能继续运行 fork / exec / wait 流程

## 4. 建议答题点

- 为什么单核的关中断不够保护多核共享数据
- 为什么副核必须自己重新写 `satp`
- 为什么 `ch5` 必须把“当前运行进程”改成每核独立语义
