//! 第五章的基础多核调度。

use crate::process::Process;
use alloc::collections::{BTreeMap, VecDeque};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Lazy;
use tg_kernel_context::foreign::{MultislotPortal, TpReg};
use tg_smp::hart;
use tg_smp::percpu::PerCpu;
use tg_smp::spin::SpinNoIrq;
use tg_task_manage::{ProcId, ProcRel};

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

static KERNEL_SATP: AtomicUsize = AtomicUsize::new(0);
pub static LIVE_TASKS: AtomicUsize = AtomicUsize::new(0);
pub static CURRENT_PROCESS: PerCpu<usize> = PerCpu::new([0; tg_smp::MAX_CPUS]);
static NEXT_TARGET_CPU: AtomicUsize = AtomicUsize::new(0);

struct SmpState {
    tasks: BTreeMap<ProcId, Process>,
    rel_map: BTreeMap<ProcId, ProcRel>,
    ready: [VecDeque<ProcId>; NUM_CPUS],
}

impl SmpState {
    fn new() -> Self {
        Self {
            tasks: BTreeMap::new(),
            rel_map: BTreeMap::new(),
            ready: [VecDeque::new(), VecDeque::new()],
        }
    }

    fn fetch_for(&mut self, cpu: usize) -> Option<(ProcId, Process)> {
        let pid = self.ready[cpu]
            .pop_front()
            .or_else(|| self.ready[1 - cpu].pop_back())?;
        let task = self.tasks.remove(&pid)?;
        Some((pid, task))
    }
}

pub struct SmpProcessor {
    state: SpinNoIrq<SmpState>,
}

impl SmpProcessor {
    pub fn new() -> Self {
        Self {
            state: SpinNoIrq::new(SmpState::new()),
        }
    }

    pub fn set_kernel_satp(&self, satp: usize) {
        KERNEL_SATP.store(satp, Ordering::Release);
    }

    pub fn add_process(&self, pid: ProcId, process: Process, parent: ProcId) {
        let target = if parent.get_usize() == usize::MAX {
            hart::hart_id()
        } else {
            NEXT_TARGET_CPU.fetch_add(1, Ordering::Relaxed) % NUM_CPUS
        };
        let mut state = self.state.lock();
        state.tasks.insert(pid, process);
        if parent.get_usize() != usize::MAX {
            state.rel_map.get_mut(&parent).unwrap().add_child(pid);
        }
        state.rel_map.insert(pid, ProcRel::new(parent));
        state.ready[target].push_back(pid);
        LIVE_TASKS.fetch_add(1, Ordering::AcqRel);
    }

    fn fetch_next(&self, cpu: usize) -> Option<(ProcId, Process)> {
        self.state.lock().fetch_for(cpu)
    }

    fn suspend(&self, pid: ProcId, process: Process, cpu: usize) {
        let mut state = self.state.lock();
        state.tasks.insert(pid, process);
        state.ready[cpu].push_back(pid);
    }

    fn exit(&self, pid: ProcId, code: isize) {
        let mut state = self.state.lock();
        let rel = state.rel_map.remove(&pid).unwrap();
        if let Some(parent) = state.rel_map.get_mut(&rel.parent) {
            parent.del_child(pid, code);
        }
        for child in rel.children {
            state.rel_map.get_mut(&child).unwrap().parent = ProcId::from_usize(0);
            if let Some(init) = state.rel_map.get_mut(&ProcId::from_usize(0)) {
                init.add_child(child);
            }
        }
        LIVE_TASKS.fetch_sub(1, Ordering::AcqRel);
    }

    pub fn with_current<R>(&self, f: impl FnOnce(&mut Process) -> R) -> R {
        unsafe { f(&mut *(CURRENT_PROCESS.get() as *mut Process)) }
    }

    pub fn wait_current(&self, child_pid: ProcId) -> Option<(ProcId, isize)> {
        let current = self.with_current(|proc| proc.pid);
        let mut state = self.state.lock();
        let rel = state.rel_map.get_mut(&current)?;
        if child_pid.get_usize() == usize::MAX {
            rel.wait_any_child()
        } else {
            rel.wait_child(child_pid)
        }
    }

    pub fn fork_current(&self) -> isize {
        let (parent_pid, child) = self.with_current(|current| {
            let parent_pid = current.pid;
            let mut child = current.fork().unwrap();
            *child.context.context.a_mut(0) = 0;
            (parent_pid, child)
        });
        let pid = child.pid;
        self.add_process(pid, child, parent_pid);
        pid.get_usize() as isize
    }
}

pub static SMP_PROCESSOR: Lazy<SmpProcessor> = Lazy::new(SmpProcessor::new);

pub fn set_current_process(process: &mut Process) {
    CURRENT_PROCESS.set(process as *mut _ as usize);
}

pub fn clear_current_process() {
    CURRENT_PROCESS.set(0);
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
    let satp = KERNEL_SATP.load(Ordering::Acquire);
    unsafe {
        core::arch::asm!(
            "csrw satp, {satp}",
            "sfence.vma",
            satp = in(reg) satp,
            options(nostack),
        );
    }
    let hart_id = hart::hart_id();
    hart::mark_hart_started(hart_id);
    schedule_loop(unsafe {
        MultislotPortal::init_transit(crate::PROTAL_TRANSIT.base().val(), NUM_CPUS)
    })
}

pub fn schedule_loop(portal: &'static mut MultislotPortal) -> ! {
    use tg_syscall::{Caller, SyscallId as Id, SyscallResult as Ret};

    let hart_id = hart::hart_id();
    loop {
        if LIVE_TASKS.load(Ordering::Acquire) == 0 {
            if hart_id == 0 {
                tg_sbi::shutdown(false);
            }
            loop {
                core::hint::spin_loop();
            }
        }
        let Some((pid, mut process)) = SMP_PROCESSOR.fetch_next(hart_id) else {
            core::hint::spin_loop();
            continue;
        };
        set_current_process(&mut process);
        unsafe {
            process.context.execute(portal, TpReg);
        }

        match riscv::register::scause::read().cause() {
            riscv::register::scause::Trap::Exception(
                riscv::register::scause::Exception::UserEnvCall,
            ) => {
                let ctx = &mut process.context.context;
                ctx.move_next();
                let id: Id = ctx.a(7).into();
                let args = [ctx.a(0), ctx.a(1), ctx.a(2), ctx.a(3), ctx.a(4), ctx.a(5)];
                match tg_syscall::handle(Caller { entity: 0, flow: 0 }, id, args) {
                    Ret::Done(ret) => {
                        if id == Id::EXIT {
                            clear_current_process();
                            SMP_PROCESSOR.exit(pid, ret);
                            continue;
                        }
                        *ctx.a_mut(0) = ret as _;
                        clear_current_process();
                        SMP_PROCESSOR.suspend(pid, process, hart_id);
                    }
                    Ret::Unsupported(_) => {
                        clear_current_process();
                        SMP_PROCESSOR.exit(pid, -2);
                    }
                }
            }
            trap => {
                tg_console::log::error!(
                    "[hart {hart_id}] pid {} trap {trap:?}",
                    pid.get_usize()
                );
                clear_current_process();
                SMP_PROCESSOR.exit(pid, -3);
            }
        }
    }
}
