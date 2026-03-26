#![no_std]
#![no_main]
#![cfg_attr(target_arch = "riscv64", deny(warnings))]
#![cfg_attr(not(target_arch = "riscv64"), allow(dead_code, unused_imports))]

mod pager;
mod process;

#[macro_use]
extern crate tg_console;
extern crate alloc;

use crate::{
    pager::FaultKind,
    process::Process,
};
use alloc::{alloc::alloc, vec::Vec};
use core::{alloc::Layout, cell::UnsafeCell};
use impls::Console;
use tg_console::log;
use tg_kernel_context::{LocalContext, foreign::MultislotPortal};
#[cfg(target_arch = "riscv64")]
use riscv::register::*;
#[cfg(not(target_arch = "riscv64"))]
use stub::Sv39;
#[cfg(target_arch = "riscv64")]
use tg_kernel_vm::page_table::Sv39;
use tg_kernel_vm::{
    AddressSpace,
    page_table::{MmuMeta, PPN, VAddr, VPN, VmFlags, VmMeta},
};
use tg_syscall::Caller;
use xmas_elf::ElfFile;

#[cfg(target_arch = "riscv64")]
const fn build_flags(s: &str) -> VmFlags<Sv39> {
    VmFlags::build_from_str(s)
}

#[cfg(target_arch = "riscv64")]
fn parse_flags(s: &str) -> Result<VmFlags<Sv39>, ()> {
    s.parse()
}

#[cfg(not(target_arch = "riscv64"))]
use stub::{build_flags, parse_flags};

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!(env!("APP_ASM")));

#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() -> ! {
    const STACK_SIZE: usize = 6 * 4096;
    #[unsafe(link_section = ".boot.stack")]
    static mut STACK: [u8; STACK_SIZE] = [0u8; STACK_SIZE];

    core::arch::naked_asm!(
        "la sp, {stack} + {stack_size}",
        "j  {main}",
        stack = sym STACK,
        stack_size = const STACK_SIZE,
        main = sym rust_main,
    )
}

const MEMORY: usize = 24 << 20;
const PORTAL_TRANSIT: VPN<Sv39> = VPN::MAX;

struct ProcessList(UnsafeCell<Vec<Process>>);

unsafe impl Sync for ProcessList {}

impl ProcessList {
    const fn new() -> Self {
        Self(UnsafeCell::new(Vec::new()))
    }

    unsafe fn get_mut(&self) -> &mut Vec<Process> {
        unsafe { &mut *self.0.get() }
    }
}

static PROCESSES: ProcessList = ProcessList::new();

extern "C" fn rust_main() -> ! {
    let layout = tg_linker::KernelLayout::locate();
    unsafe { layout.zero_bss() };
    tg_console::init_console(&Console);
    tg_console::set_log_level(option_env!("LOG"));
    tg_console::test_log();
    tg_kernel_alloc::init(layout.start() as _);
    unsafe {
        tg_kernel_alloc::transfer(core::slice::from_raw_parts_mut(
            layout.end() as _,
            MEMORY - layout.len(),
        ))
    };

    let portal_size = MultislotPortal::calculate_size(1);
    let portal_layout = Layout::from_size_align(portal_size, 1 << Sv39::PAGE_BITS).unwrap();
    let portal_ptr = unsafe { alloc(portal_layout) };
    assert!(portal_layout.size() < 1 << Sv39::PAGE_BITS);
    let ks = kernel_space(layout, MEMORY, portal_ptr as _);
    let portal_idx = PORTAL_TRANSIT.index_in(Sv39::MAX_LEVEL);

    for elf in tg_linker::AppMeta::locate().iter() {
        if let Some(process) = Process::new(ElfFile::new(elf).unwrap()) {
            process.address_space.root()[portal_idx] = ks.root()[portal_idx];
            unsafe { PROCESSES.get_mut().push(process) };
        }
    }

    const SCHED_PAGES: usize = 16;
    const PAGE: Layout = unsafe {
        Layout::from_size_align_unchecked(
            SCHED_PAGES << Sv39::PAGE_BITS,
            1 << Sv39::PAGE_BITS,
        )
    };
    let stack = unsafe { alloc(PAGE) };
    let mut ks = ks;
    ks.map_extern(
        VPN::new((1 << 26) - SCHED_PAGES)..VPN::new(1 << 26),
        PPN::new(stack as usize >> Sv39::PAGE_BITS),
        build_flags("_WRV"),
    );
    let mut scheduling = LocalContext::thread(schedule as *const () as _, false);
    *scheduling.sp_mut() = 1 << 38;
    unsafe { scheduling.execute() };
    #[cfg(target_arch = "riscv64")]
    {
        log::error!("stval = {:#x}", stval::read());
        panic!("trap from scheduling thread: {:?}", scause::read().cause());
    }
    #[cfg(not(target_arch = "riscv64"))]
    panic!("trap from scheduling thread");
}

extern "C" fn schedule() -> ! {
    let portal = unsafe { MultislotPortal::init_transit(PORTAL_TRANSIT.base().val(), 1) };
    tg_syscall::init_io(&impls::SyscallContext);
    tg_syscall::init_process(&impls::SyscallContext);
    tg_syscall::init_scheduling(&impls::SyscallContext);
    tg_syscall::init_clock(&impls::SyscallContext);
    tg_syscall::init_memory(&impls::SyscallContext);

    while !unsafe { PROCESSES.get_mut().is_empty() } {
        let process = unsafe { &mut PROCESSES.get_mut()[0] };
        unsafe { process.context.execute(portal, ()) };

        match scause::read().cause() {
            scause::Trap::Exception(scause::Exception::UserEnvCall) => {
                use tg_syscall::{SyscallId as Id, SyscallResult as Ret};

                let (id, args) = {
                    let process = unsafe { &mut PROCESSES.get_mut()[0] };
                    let ctx = &process.context.context;
                    (
                        Id::from(ctx.a(7)),
                        [ctx.a(0), ctx.a(1), ctx.a(2), ctx.a(3), ctx.a(4), ctx.a(5)],
                    )
                };
                match tg_syscall::handle(Caller { entity: 0, flow: 0 }, id, args) {
                    Ret::Done(ret) => match id {
                        Id::EXIT => unsafe {
                            PROCESSES.get_mut().remove(0);
                        },
                        _ => {
                            let process = unsafe { &mut PROCESSES.get_mut()[0] };
                            let ctx = &mut process.context.context;
                            *ctx.a_mut(0) = ret as _;
                            ctx.move_next();
                        }
                    },
                    Ret::Unsupported(_) => unsafe {
                        PROCESSES.get_mut().remove(0);
                    },
                }
            }
            scause::Trap::Exception(scause::Exception::LoadPageFault) => {
                if !unsafe { &mut PROCESSES.get_mut()[0] }
                    .handle_page_fault(FaultKind::Load, stval::read())
                {
                    kill_current_process("load page fault");
                }
            }
            scause::Trap::Exception(scause::Exception::StorePageFault) => {
                if !unsafe { &mut PROCESSES.get_mut()[0] }
                    .handle_page_fault(FaultKind::Store, stval::read())
                {
                    kill_current_process("store page fault");
                }
            }
            scause::Trap::Exception(scause::Exception::InstructionPageFault) => {
                if !unsafe { &mut PROCESSES.get_mut()[0] }
                    .handle_page_fault(FaultKind::Instruction, stval::read())
                {
                    kill_current_process("instruction page fault");
                }
            }
            other => {
                log::error!(
                    "unsupported trap: {:?}, stval = {:#x}, sepc = {:#x}",
                    other,
                    stval::read(),
                    unsafe { PROCESSES.get_mut()[0].context.context.pc() },
                );
                unsafe { PROCESSES.get_mut().remove(0) };
            }
        }
    }
    tg_sbi::shutdown(false)
}

fn kill_current_process(reason: &str) {
    log::error!(
        "{}: stval={:#x} sepc={:#x}",
        reason,
        stval::read(),
        unsafe { PROCESSES.get_mut()[0].context.context.pc() },
    );
    unsafe { PROCESSES.get_mut().remove(0) };
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    log::error!("{info}");
    tg_sbi::shutdown(true)
}

fn kernel_space(
    layout: tg_linker::KernelLayout,
    memory: usize,
    portal: usize,
) -> AddressSpace<Sv39, impls::Sv39Manager> {
    let mut space = AddressSpace::<Sv39, impls::Sv39Manager>::new();
    for region in layout.iter() {
        use tg_linker::KernelRegionTitle::*;
        let flags = match region.title {
            Text => "X_RV",
            Rodata => "__RV",
            Data | Boot => "_WRV",
        };
        let s = VAddr::<Sv39>::new(region.range.start);
        let e = VAddr::<Sv39>::new(region.range.end);
        space.map_extern(
            s.floor()..e.ceil(),
            PPN::new(s.floor().val()),
            build_flags(flags),
        );
    }

    let s = VAddr::<Sv39>::new(layout.end());
    let e = VAddr::<Sv39>::new(layout.start() + memory);
    space.map_extern(
        s.floor()..e.ceil(),
        PPN::new(s.floor().val()),
        build_flags("_WRV"),
    );
    space.map_extern(
        PORTAL_TRANSIT..PORTAL_TRANSIT + 1,
        PPN::new(portal >> Sv39::PAGE_BITS),
        build_flags("__G_XWRV"),
    );
    unsafe { satp::set(satp::Mode::Sv39, 0, space.root_ppn().val()) };
    space
}

mod impls {
    use crate::{PROCESSES, Sv39, build_flags};
    use core::ptr::NonNull;
    use tg_kernel_vm::{
        PageManager,
        page_table::{MmuMeta, PPN, Pte, VAddr, VPN, VmFlags},
    };
    use tg_syscall::*;

    #[repr(transparent)]
    pub(crate) struct Sv39Manager(NonNull<Pte<Sv39>>);

    impl Sv39Manager {
        const OWNED: VmFlags<Sv39> = unsafe { VmFlags::from_raw(1 << 8) };

        #[inline]
        fn page_alloc<T>(count: usize) -> *mut T {
            unsafe {
                alloc::alloc::alloc_zeroed(core::alloc::Layout::from_size_align_unchecked(
                    count << Sv39::PAGE_BITS,
                    1 << Sv39::PAGE_BITS,
                ))
            }
            .cast()
        }
    }

    impl PageManager<Sv39> for Sv39Manager {
        fn new_root() -> Self {
            Self(NonNull::new(Self::page_alloc(1)).unwrap())
        }

        fn root_ppn(&self) -> PPN<Sv39> {
            PPN::new(self.0.as_ptr() as usize >> Sv39::PAGE_BITS)
        }

        fn root_ptr(&self) -> NonNull<Pte<Sv39>> {
            self.0
        }

        fn p_to_v<T>(&self, ppn: PPN<Sv39>) -> NonNull<T> {
            unsafe { NonNull::new_unchecked(VPN::<Sv39>::new(ppn.val()).base().as_mut_ptr()) }
        }

        fn v_to_p<T>(&self, ptr: NonNull<T>) -> PPN<Sv39> {
            PPN::new(VAddr::<Sv39>::new(ptr.as_ptr() as _).floor().val())
        }

        fn check_owned(&self, pte: Pte<Sv39>) -> bool {
            pte.flags().contains(Self::OWNED)
        }

        fn allocate(&mut self, len: usize, flags: &mut VmFlags<Sv39>) -> NonNull<u8> {
            *flags |= Self::OWNED;
            NonNull::new(Self::page_alloc(len)).unwrap()
        }

        fn deallocate(&mut self, _pte: Pte<Sv39>, _len: usize) -> usize {
            0
        }

        fn drop_root(&mut self) {}
    }

    pub(crate) struct Console;

    impl tg_console::Console for Console {
        fn put_char(&self, c: u8) {
            tg_sbi::console_putchar(c);
        }
    }

    pub(crate) struct SyscallContext;

    impl IO for SyscallContext {
        fn write(&self, caller: Caller, fd: usize, buf: usize, count: usize) -> isize {
            match fd {
                STDOUT | STDDEBUG => {
                    const READABLE: VmFlags<Sv39> = build_flags("RV");
                    if let Some(ptr) = unsafe { PROCESSES.get_mut() }
                        .get(caller.entity)
                        .and_then(|process| process.translate_ptr::<u8>(buf, READABLE))
                    {
                        print!("{}", unsafe {
                            core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                                ptr.as_ptr(),
                                count,
                            ))
                        });
                        count as isize
                    } else {
                        -1
                    }
                }
                _ => -1,
            }
        }
    }

    impl Process for SyscallContext {
        fn exit(&self, _caller: Caller, _status: usize) -> isize {
            0
        }

        fn sbrk(&self, caller: Caller, size: i32) -> isize {
            unsafe { PROCESSES.get_mut() }
                .get_mut(caller.entity)
                .and_then(|process| process.change_program_brk(size as isize))
                .map(|old_brk| old_brk as isize)
                .unwrap_or(-1)
        }
    }

    impl Scheduling for SyscallContext {
        fn sched_yield(&self, _caller: Caller) -> isize {
            0
        }
    }

    impl Clock for SyscallContext {
        fn clock_gettime(&self, caller: Caller, clock_id: ClockId, tp: usize) -> isize {
            const WRITABLE: VmFlags<Sv39> = build_flags("W_V");
            match clock_id {
                ClockId::CLOCK_MONOTONIC => {
                    if let Some(mut ptr) = unsafe { PROCESSES.get_mut() }
                        .get(caller.entity)
                        .and_then(|process| process.translate_ptr::<TimeSpec>(tp, WRITABLE))
                    {
                        let time = riscv::register::time::read() * 10000 / 125;
                        *unsafe { ptr.as_mut() } = TimeSpec {
                            tv_sec: time / 1_000_000_000,
                            tv_nsec: time % 1_000_000_000,
                        };
                        0
                    } else {
                        -1
                    }
                }
                _ => -1,
            }
        }
    }

    impl Memory for SyscallContext {
        fn mmap(
            &self,
            caller: Caller,
            addr: usize,
            len: usize,
            prot: i32,
            _flags: i32,
            _fd: i32,
            _offset: usize,
        ) -> isize {
            unsafe { PROCESSES.get_mut() }
                .get_mut(caller.entity)
                .and_then(|process| process.mmap_anonymous(addr, len, prot))
                .map(|()| 0)
                .unwrap_or(-1)
        }

        fn munmap(&self, caller: Caller, addr: usize, len: usize) -> isize {
            unsafe { PROCESSES.get_mut() }
                .get_mut(caller.entity)
                .and_then(|process| process.munmap_anonymous(addr, len))
                .map(|()| 0)
                .unwrap_or(-1)
        }

        fn vmctl(&self, caller: Caller, cmd: usize, arg0: usize, _arg1: usize) -> isize {
            const WRITABLE: VmFlags<Sv39> = build_flags("W_V");
            let Some(process) = unsafe { PROCESSES.get_mut() }.get_mut(caller.entity) else {
                return -1;
            };
            match cmd {
                VMCTL_SET_ALGO => process.vm_set_algo(arg0).then_some(0).unwrap_or(-1),
                VMCTL_SET_QUOTA => process.vm_set_quota(arg0).then_some(0).unwrap_or(-1),
                VMCTL_RESET_STATS => {
                    process.vm_reset_stats();
                    0
                }
                VMCTL_GET_STATS => {
                    let Some(mut ptr) = process.translate_ptr::<VmStats>(arg0, WRITABLE) else {
                        return -1;
                    };
                    *unsafe { ptr.as_mut() } = process.vm_snapshot_stats();
                    0
                }
                _ => -1,
            }
        }
    }
}

#[cfg(not(target_arch = "riscv64"))]
mod stub {
    use tg_kernel_vm::page_table::{MmuMeta, VmFlags};

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
    pub struct Sv39;

    impl MmuMeta for Sv39 {
        const P_ADDR_BITS: usize = 56;
        const PAGE_BITS: usize = 12;
        const LEVEL_BITS: &'static [usize] = &[9, 9, 9];
        const PPN_POS: usize = 10;

        fn is_leaf(value: usize) -> bool {
            value & 0b1110 != 0
        }
    }

    pub const fn build_flags(_s: &str) -> VmFlags<Sv39> {
        unsafe { VmFlags::from_raw(0) }
    }

    pub fn parse_flags(_s: &str) -> Result<VmFlags<Sv39>, ()> {
        Ok(unsafe { VmFlags::from_raw(0) })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn main() -> i32 {
        0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn __libc_start_main() -> i32 {
        0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn rust_eh_personality() {}
}
