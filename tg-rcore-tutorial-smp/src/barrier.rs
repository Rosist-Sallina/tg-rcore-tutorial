//! 内存屏障封装。

use core::sync::atomic::{compiler_fence, Ordering};

/// 完整读写屏障。
#[inline(always)]
pub fn fence() {
    #[cfg(target_arch = "riscv64")]
    unsafe {
        core::arch::asm!("fence rw, rw", options(nostack, preserves_flags));
    }

    #[cfg(not(target_arch = "riscv64"))]
    compiler_fence(Ordering::SeqCst);
}

/// 编译器屏障。
#[inline(always)]
pub fn compiler_barrier() {
    compiler_fence(Ordering::SeqCst);
}
