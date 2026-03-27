//! 内存屏障封装。

use core::sync::atomic::{compiler_fence, Ordering};

/// 完整读写屏障。
#[inline(always)]
pub fn fence() {
    unsafe {
        core::arch::asm!("fence rw, rw", options(nostack, preserves_flags));
    }
}

/// 编译器屏障。
#[inline(always)]
pub fn compiler_barrier() {
    compiler_fence(Ordering::SeqCst);
}
