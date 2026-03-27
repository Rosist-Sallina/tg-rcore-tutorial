# T2L10 测试计划

## 已完成

- `ch3`: `cargo check --features smp`
- `ch4`: `cargo check --features smp`
- `ch5`: `cargo check --features smp`

- `ch3`: `cargo run --features smp`
- `ch4`: `cargo run --features smp`
- `ch5`: `cargo run --features smp`

## 运行级观察结论

### ch3

- 能进入双核调度
- 两个 hart 都会推进用户程序

### ch4

- 能双核启动
- 副核完成页表激活
- 基础用户程序能继续运行

### ch5

- 能双核启动
- 能进入 `Rust user shell`
- 提示符 `>>` 正常出现

## 后续可补强的测试

- 在 `ch5` shell 中继续手动执行 fork/exec/wait 相关程序
- 设计 `TLB shootdown` 专项压力用例
- 设计负载不均衡场景观察 stealing 行为
