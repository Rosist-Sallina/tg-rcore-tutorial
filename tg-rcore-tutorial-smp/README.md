# tg-rcore-tutorial-smp

## 学生提交信息

- crate 名称：`rosist-sallina-tg-rcore-tutorial-smp-t2l10`
- crate 版本：`0.0.1-preview.1`
- 服务实验：`rosist-sallina-tg-rcore-tutorial-T2L10`
- 仓库地址：`https://github.com/Rosist-Sallina/tg-rcore-tutorial`
- 仓库页面：`https://github.com/Rosist-Sallina/tg-rcore-tutorial/tree/t2l10-redo/tg-rcore-tutorial-smp`
- 建议 tag：`rosist-sallina-tg-rcore-tutorial-smp-t2l10-v0.0.1-preview.1`

这个 crate 提供 `T2L10` 需要的最小 SMP 基础设施：
- hart 启动与状态标记
- per-cpu 存储
- 自旋锁与 `SpinNoIrq`
- IPI action/pending 管理
- RISC-V `fence` 封装
