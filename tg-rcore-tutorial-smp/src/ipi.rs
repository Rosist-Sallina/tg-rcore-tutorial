//! 共享内存轮询版 IPI。

use core::sync::atomic::{AtomicU8, Ordering};

const SBI_EXT_IPI: usize = 0x735049;

/// IPI 动作。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IpiAction {
    /// 无动作。
    None = 0,
    /// 刷新 TLB。
    TlbShootdown = 1,
    /// 唤醒提示。
    Wakeup = 2,
    /// 请求重调度。
    Reschedule = 3,
}

static PENDING_IPI: [AtomicU8; crate::MAX_CPUS] =
    [AtomicU8::new(0), AtomicU8::new(0)];

/// 向目标 hart 投递动作。
pub fn send_ipi(target_hart: usize, action: IpiAction) {
    PENDING_IPI[target_hart].store(action as u8, Ordering::Release);
    unsafe {
        let hart_mask = 1usize << target_hart;
        let _error: isize;
        core::arch::asm!(
            "ecall",
            inlateout("x10") hart_mask => _error,
            in("x11") 0usize,
            in("x16") 0usize,
            in("x17") SBI_EXT_IPI,
        );
    }
}

/// 取出当前 hart 的待处理动作。
pub fn take_pending_ipi() -> IpiAction {
    let value = PENDING_IPI[crate::hart::hart_id()].swap(0, Ordering::AcqRel);
    match value {
        1 => IpiAction::TlbShootdown,
        2 => IpiAction::Wakeup,
        3 => IpiAction::Reschedule,
        _ => IpiAction::None,
    }
}

/// 由上层章节实现具体动作。
pub trait IpiHandler {
    /// 处理 TLB shootdown。
    fn handle_tlb_shootdown(&self);

    /// 处理唤醒。
    fn handle_wakeup(&self);

    /// 处理重调度。
    fn handle_reschedule(&self);

    /// 分发当前待处理动作。
    fn dispatch_ipi(&self) {
        match take_pending_ipi() {
            IpiAction::None => {}
            IpiAction::TlbShootdown => self.handle_tlb_shootdown(),
            IpiAction::Wakeup => self.handle_wakeup(),
            IpiAction::Reschedule => self.handle_reschedule(),
        }
    }
}
