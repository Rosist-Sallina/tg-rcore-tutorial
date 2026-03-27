//! 第四章的基础 SMP 支持。

use crate::process::Process;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};
use tg_smp::hart;
use tg_smp::ipi::{self, IpiAction, IpiHandler};
use tg_smp::percpu::PerCpu;
use tg_smp::spin::SpinLock;

pub const NUM_CPUS: usize = tg_smp::MAX_CPUS;
const SECONDARY_STACK_SIZE: usize = 4 * 4096;

#[repr(transparent)]
struct StackStorage([UnsafeCell<u8>; SECONDARY_STACK_SIZE]);

unsafe impl Sync for StackStorage {}

impl StackStorage {
    const fn zeroed() -> Self {
        Self(unsafe { core::mem::MaybeUninit::zeroed().assume_init() })
    }

    fn top(&self) -> usize {
        unsafe { self.0.as_ptr().add(SECONDARY_STACK_SIZE) as usize }
    }
}

#[unsafe(link_section = ".boot.stack")]
static SECONDARY_STACK: StackStorage = StackStorage::zeroed();

pub static PROCESSES: SpinLock<alloc::vec::Vec<Process>> = SpinLock::new(alloc::vec::Vec::new());
pub static REMAIN: AtomicUsize = AtomicUsize::new(0);
static KERNEL_SATP: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy)]
struct SchedulerRange {
    start: usize,
    end: usize,
}

static SCHED_RANGES: PerCpu<SchedulerRange> = PerCpu::new(
    [SchedulerRange { start: 0, end: 0 }; NUM_CPUS]
);

pub fn set_kernel_satp(satp: usize) {
    KERNEL_SATP.store(satp, Ordering::Release);
}

pub fn init_smp_scheduler(total: usize) {
    let split = total.div_ceil(NUM_CPUS);
    let secondary = 1 - hart::hart_id();
    unsafe {
        *SCHED_RANGES.get_mut() = SchedulerRange {
            start: 0,
            end: split.min(total),
        };
        *SCHED_RANGES.get_remote_mut(secondary) = SchedulerRange {
            start: split.min(total),
            end: total,
        };
    }
    REMAIN.store(total, Ordering::Release);
}

pub fn boot_secondary() {
    let primary = hart::hart_id();
    let secondary = 1 - primary;
    tg_console::log::info!("[hart {primary}] booting hart {secondary}");
    hart::boot_hart(secondary, _secondary_start as *const () as usize, SECONDARY_STACK.top());
    hart::wait_for_hart(secondary);
    tg_console::log::info!("[hart {primary}] hart {secondary} is ready");
}

#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
unsafe extern "C" fn _secondary_start() -> ! {
    core::arch::naked_asm!(
        "mv tp, a0",
        "mv sp, a1",
        "j {main}",
        main = sym secondary_rust_main,
    )
}

extern "C" fn secondary_rust_main() -> ! {
    use riscv::register::sie;

    let satp_value = KERNEL_SATP.load(Ordering::Acquire);
    unsafe {
        core::arch::asm!(
            "csrw satp, {value}",
            "sfence.vma",
            value = in(reg) satp_value,
            options(nostack),
        );
        sie::set_ssoft();
    }
    let hart_id = hart::hart_id();
    tg_console::log::info!("[hart {hart_id}] vm is ready");
    hart::mark_hart_started(hart_id);
    schedule_loop()
}

#[allow(dead_code)]
pub fn tlb_shootdown() {
    let target = 1 - hart::hart_id();
    ipi::send_ipi(target, IpiAction::TlbShootdown);
}

struct Ch4Ipi;

impl IpiHandler for Ch4Ipi {
    fn handle_tlb_shootdown(&self) {
        unsafe {
            core::arch::asm!("sfence.vma", options(nostack));
        }
    }

    fn handle_wakeup(&self) {}

    fn handle_reschedule(&self) {}
}

static IPI_HANDLER: Ch4Ipi = Ch4Ipi;
static SYSCALL_CONTEXT: crate::impls::SyscallContext = crate::impls::SyscallContext;

pub fn schedule_loop() -> ! {
    use riscv::register::{scause, sie, stval};
    use tg_kernel_context::foreign::{MultislotPortal, TpReg};
    use tg_syscall::{Caller, SyscallId as Id, SyscallResult as Ret};

    unsafe {
        sie::set_ssoft();
    }

    let portal = unsafe {
        MultislotPortal::init_transit(crate::PROTAL_TRANSIT.base().val(), NUM_CPUS)
    };
    tg_syscall::init_io(&SYSCALL_CONTEXT);
    tg_syscall::init_process(&SYSCALL_CONTEXT);
    tg_syscall::init_scheduling(&SYSCALL_CONTEXT);
    tg_syscall::init_clock(&SYSCALL_CONTEXT);
    tg_syscall::init_trace(&SYSCALL_CONTEXT);
    tg_syscall::init_memory(&SYSCALL_CONTEXT);

    let range = SCHED_RANGES.get();
    let hart_id = hart::hart_id();
    let mut index = range.start;
    while REMAIN.load(Ordering::Acquire) > 0 {
        IPI_HANDLER.dispatch_ipi();
        if range.start == range.end {
            continue;
        }
        if index >= range.end {
            index = range.start;
        }

        let action = {
            let mut procs = PROCESSES.lock();
            if index >= procs.len() {
                None
            } else {
                crate::impls::set_current_procs(&mut *procs as *mut _);
                let process = &mut procs[index];
                unsafe {
                    process.context.execute(portal, TpReg);
                }
                let action = match scause::read().cause() {
                    scause::Trap::Exception(scause::Exception::UserEnvCall) => {
                        let (id, args) = {
                            let ctx = &process.context.context;
                            (
                                Id::from(ctx.a(7)),
                                [ctx.a(0), ctx.a(1), ctx.a(2), ctx.a(3), ctx.a(4), ctx.a(5)],
                            )
                        };
                        process.record_syscall(id.0);
                        match tg_syscall::handle(Caller { entity: index, flow: 0 }, id, args) {
                            Ret::Done(ret) => match id {
                                Id::EXIT => Some(true),
                                _ => {
                                    let ctx = &mut process.context.context;
                                    *ctx.a_mut(0) = ret as _;
                                    ctx.move_next();
                                    Some(false)
                                }
                            },
                            Ret::Unsupported(_) => Some(true),
                        }
                    }
                    scause::Trap::Interrupt(scause::Interrupt::SupervisorSoft) => Some(false),
                    e => {
                        tg_console::log::error!(
                            "[hart {hart_id}] process {index} trap {e:?}, stval = {:#x}",
                            stval::read()
                        );
                        Some(true)
                    }
                };
                crate::impls::clear_current_procs();
                action.map(|finish| {
                    if finish {
                        procs.remove(index);
                    }
                    finish
                })
            }
        };

        if let Some(true) = action {
            REMAIN.fetch_sub(1, Ordering::AcqRel);
            continue;
        }
        index += 1;
    }
    if hart_id == 0 {
        tg_sbi::shutdown(false);
    }
    loop {
        core::hint::spin_loop();
    }
}
