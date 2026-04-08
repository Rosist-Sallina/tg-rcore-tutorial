# T2L10 常见 Bug

## 1. 写死 hart 0 是主核

现象：

- `hart_start` 返回 `already available`
- 或者主副核角色错乱

原因：

- OpenSBI 不保证 boot hart 一定是 0

修法：

- 运行时读取 `hart_id()`
- 用 `other = 1 - current`

## 2. 副核启动了但不能进入地址空间

现象：

- `ch4/ch5` 副核一启动就死

原因：

- 副核没有自己写 `satp`
- 没做 `sfence.vma`

## 3. 两个核拿到同一个进程

现象：

- `ch5` 同一个进程被重复执行
- `wait` / `exit` 结果错乱

修法：

- 当前运行进程从共享结构中临时拿走
- Trap 返回后再决定放回还是回收

## 4. shell 输入全是乱码

现象：

- `>>` 后面直接刷一串 `ÿ`

原因：

- 没处理 `console_getchar() == -1`

修法：

- 把读标准输入改成阻塞读
