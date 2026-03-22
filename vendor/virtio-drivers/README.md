# VirtIO-drivers-rs

## 学生提交信息

- crate 名称：`rosist-sallina-tg-rcore-tutorial-virtio-drivers-t3l3`
- crate 版本：`0.0.1-preview.1`
- 仓库地址：`https://github.com/Rosist-Sallina/tg-rcore-tutorial`
- 仓库页面：`https://github.com/Rosist-Sallina/tg-rcore-tutorial/tree/test/vendor/virtio-drivers`
- 建议 tag：`rosist-sallina-tg-rcore-tutorial-virtio-drivers-t3l3-v0.0.1-preview.1`

这个 crate 是 `ch3` 拓展实验图形版贪吃蛇的配套驱动依赖，包含本地修过的 GPU 初始化和队列路径。

[![CI](https://github.com/rcore-os/virtio-drivers/workflows/CI/badge.svg?branch=master)](https://github.com/rcore-os/virtio-drivers/actions)

VirtIO guest drivers in Rust. For **no_std** environment.

## Support status

### Device types

| Device  | Supported |
| ------- | --------- |
| Block   | ✅        |
| Net     | ✅        |
| GPU     | ✅        |
| Input   | ✅        |
| Console | ✅        |
| ...     | ❌        |

### Transports

| Transport   | Supported |           |
| ----------- | --------- | --------- |
| Legacy MMIO | ✅        | version 1 |
| MMIO        | ✅        | version 2 |
| PCI         | ❌        |           |

### Device-independent features

| Feature flag                 | Supported |                                         |
| ---------------------------- | --------- | --------------------------------------- |
| `VIRTIO_F_INDIRECT_DESC`     | ❌        | Indirect descriptors                    |
| `VIRTIO_F_EVENT_IDX`         | ❌        | `avail_event` and `used_event` fields   |
| `VIRTIO_F_VERSION_1`         | TODO      | VirtIO version 1 compliance             |
| `VIRTIO_F_ACCESS_PLATFORM`   | ❌        | Limited device access to memory         |
| `VIRTIO_F_RING_PACKED`       | ❌        | Packed virtqueue layout                 |
| `VIRTIO_F_IN_ORDER`          | ❌        | Optimisations for in-order buffer usage |
| `VIRTIO_F_ORDER_PLATFORM`    | ❌        | Platform ordering for memory access     |
| `VIRTIO_F_SR_IOV`            | ❌        | Single root I/O virtualization          |
| `VIRTIO_F_NOTIFICATION_DATA` | ❌        | Extra data in device notifications      |

## Examples & Tests

- x86_64 (TODO)

- [RISCV](./examples/riscv)
