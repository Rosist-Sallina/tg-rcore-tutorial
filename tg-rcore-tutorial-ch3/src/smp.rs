//! 第三章的最小双核调度支持。

use crate::task::{SchedulingEvent, TaskControlBlock};
use crate::APP_CAPACITY;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};
use riscv::register::{scause, sie, time};
use tg_smp::hart;
use tg_smp::percpu::PerCpu;
use tg_smp::spin::SpinLock;

const NUM_CPUS: usize = tg_smp::MAX_CPUS;
const PER_CPU_STACK_SIZE: usize = (APP_CAPACITY + 2) * 8192;

#[repr(transparent)]
struct StackStorage([UnsafeCell<u8>; PER_CPU_STACK_SIZE]);

unsafe impl Sync for StackStorage {}

impl StackStorage {
    const fn zeroed() -> Self {
        Self(unsafe { core::mem::MaybeUninit::zeroed().assume_init() })
    }

    fn top(&self) -> usize {
        unsafe { self.0.as_ptr().add(PER_CPU_STACK_SIZE) as usize }
    }
}

#[unsafe(link_section = ".boot.stack")]
static SECONDARY_STACK: StackStorage = StackStorage::zeroed();

/// 剩余未结束任务数。
pub static REMAIN: AtomicUsize = AtomicUsize::new(0);

/// 全局任务数组。
pub static TCBS: SpinLock<[TaskControlBlock; APP_CAPACITY]> =
    SpinLock::new([TaskControlBlock::ZERO; APP_CAPACITY]);

#[derive(Clone, Copy)]
struct SchedulerRange {
    start: usize,
    end: usize,
}

static SCHED_RANGES: PerCpu<SchedulerRange> = PerCpu::new(
    [SchedulerRange { start: 0, end: 0 }; NUM_CPUS]
);

/// 初始化各 hart 的任务范围。
pub fn init_smp_scheduler(total_tasks: usize) {
    let split = total_tasks.div_ceil(NUM_CPUS);
    let primary = hart::hart_id();
    let secondary = 1 - primary;
    unsafe {
        *SCHED_RANGES.get_mut() = SchedulerRange {
            start: 0,
            end: split.min(total_tasks),
        };
        *SCHED_RANGES.get_remote_mut(secondary) = SchedulerRange {
            start: split.min(total_tasks),
            end: total_tasks,
        };
    }
    REMAIN.store(total_tasks, Ordering::Release);
}

/// 启动副核。
pub fn boot_secondary() {
    let primary = hart::hart_id();
    let secondary = 1 - primary;
    tg_console::log::info!("[hart {primary}] booting hart {secondary}");
    hart::boot_hart(secondary, _secondary_start as *const () as usize, SECONDARY_STACK.top());
    tg_console::log::info!("[hart {primary}] hart_start returned");
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
    let hart_id = hart::hart_id();
    tg_console::log::info!("[hart {hart_id}] entered secondary_rust_main");
    hart::mark_hart_started(hart_id);
    unsafe {
        sie::set_stimer();
    }
    schedule_loop()
}

/// 供主核和副核共用的调度循环。
pub fn schedule_loop() -> ! {
    let range = SCHED_RANGES.get();
    if range.start >= range.end {
        loop {
            core::hint::spin_loop();
        }
    }

    let hart_id = hart::hart_id();
    let mut index = range.start;
    while REMAIN.load(Ordering::Acquire) > 0 {
        if index >= range.end {
            index = range.start;
        }
        let done = {
            let mut tcbs = TCBS.lock();
            let tcb = &mut tcbs[index];
            if tcb.finish {
                false
            } else {
                #[cfg(not(feature = "coop"))]
                tg_sbi::set_timer(time::read64() + 12500);
                unsafe {
                    tcb.execute();
                }
                let finish = match scause::read().cause() {
                    scause::Trap::Interrupt(scause::Interrupt::SupervisorTimer) => {
                        tg_sbi::set_timer(u64::MAX);
                        false
                    }
                    scause::Trap::Exception(scause::Exception::UserEnvCall) => {
                        match tcb.handle_syscall() {
                            SchedulingEvent::None | SchedulingEvent::Yield => false,
                            SchedulingEvent::Exit(code) => {
                                tg_console::log::info!(
                                    "[hart {hart_id}] app{index} exit with code {code}"
                                );
                                true
                            }
                            SchedulingEvent::UnsupportedSyscall(id) => {
                                tg_console::log::error!(
                                    "[hart {hart_id}] app{index} unsupported syscall {}",
                                    id.0
                                );
                                true
                            }
                        }
                    }
                    scause::Trap::Exception(e) => {
                        tg_console::log::error!(
                            "[hart {hart_id}] app{index} killed by {e:?}"
                        );
                        true
                    }
                    scause::Trap::Interrupt(ir) => {
                        tg_console::log::error!(
                            "[hart {hart_id}] app{index} killed by interrupt {ir:?}"
                        );
                        true
                    }
                };
                if finish {
                    tcb.finish = true;
                }
                finish
            }
        };
        if done {
            REMAIN.fetch_sub(1, Ordering::AcqRel);
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
