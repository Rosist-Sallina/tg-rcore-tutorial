//! Hart 启动与状态。

use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_arch = "riscv64")]
const SBI_EXT_HSM: usize = 0x48534D;
#[cfg(target_arch = "riscv64")]
const SBI_HSM_HART_START: usize = 0;

/// 记录每个 hart 是否已经进入 S 态初始化代码。
pub static HART_STARTED: [AtomicBool; crate::MAX_CPUS] =
    [AtomicBool::new(false), AtomicBool::new(false)];

/// 读取当前 hart id。
#[inline(always)]
pub fn hart_id() -> usize {
    #[cfg(target_arch = "riscv64")]
    {
        let id: usize;
        unsafe {
            core::arch::asm!("mv {}, tp", out(reg) id, options(nostack, preserves_flags));
        }
        id
    }

    #[cfg(not(target_arch = "riscv64"))]
    {
        0
    }
}

/// 启动目标 hart。
pub fn boot_hart(hart_id: usize, start_addr: usize, opaque: usize) {
    #[cfg(target_arch = "riscv64")]
    {
        let error: isize;
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("x10") hart_id => error,
                in("x11") start_addr,
                in("x12") opaque,
                in("x16") SBI_HSM_HART_START,
                in("x17") SBI_EXT_HSM,
            );
        }
        if error != 0 {
            panic!("SBI hart_start failed for hart {}: error {}", hart_id, error);
        }
    }

    #[cfg(not(target_arch = "riscv64"))]
    {
        let _ = (hart_id, start_addr, opaque);
        panic!("boot_hart is only supported on riscv64");
    }
}

/// 等待目标 hart 进入 S 态。
pub fn wait_for_hart(hart_id: usize) {
    while !HART_STARTED[hart_id].load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
}

/// 标记当前 hart 已经完成最早期初始化。
pub fn mark_hart_started(hart_id: usize) {
    HART_STARTED[hart_id].store(true, Ordering::Release);
}
