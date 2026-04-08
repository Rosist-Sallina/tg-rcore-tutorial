# DOOM 用户态工程

当前目录包含两部分：

- `src/`：Cargo 用户程序入口（默认是 framebuffer/input 的 stub 演示程序，二进制名 `doom`）
- `doomgeneric/`：从 `doomgeneric` 参考实现拷贝的源码目录（后续可接入 `cc` 编译流程）
- `assets/doom1.wad`：shareware WAD 资源文件

## 构建（stub）

```bash
cd tg-rcore-tutorial-ch8/doom
cargo build --target riscv64gc-unknown-none-elf
```

生成的 `target/riscv64gc-unknown-none-elf/debug/doom` 会被 `tg-rcore-tutorial-ch8/build.rs`
自动打包进 `fs.img`（若文件存在）。

## 说明

- 默认构建为 `stub`（验证 framebuffer + input syscall 通路）。
- `real_doom` 功能位已预留，但还未接入完整 C 侧编译与 libc 桩链接流程。
