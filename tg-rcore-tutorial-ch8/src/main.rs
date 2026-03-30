//! # 第八章：并发
//!
//! 本章在第七章"进程间通信与信号"的基础上，引入了 **线程** 和 **同步原语**。
//!
//! ## 核心概念
//!
//! ### 1. 线程（Thread）
//!
//! 将原来的"进程"拆分为两个独立的抽象：
//! - **Process（进程）**：管理共享资源（地址空间、文件描述符表、同步原语列表、信号）
//! - **Thread（线程）**：管理执行状态（上下文、TID）
//!
//! 同一进程的多个线程共享地址空间，但各自有独立的用户栈和执行上下文。
//!
//! ### 2. 同步原语
//!
//! - **Mutex（互斥锁）**：保证临界区互斥访问
//! - **Semaphore（信号量）**：P/V 操作，支持计数型资源管理
//! - **Condvar（条件变量）**：配合互斥锁使用，支持线程等待/唤醒
//!
//! ### 3. 线程阻塞
//!
//! 当线程尝试获取已被占用的锁/信号量时，内核将其标记为**阻塞态**，
//! 从就绪队列中移除。当持有者释放资源时，唤醒等待队列中的线程。
//!
//! ## 与第七章的主要区别
//!
//! | 特性 | 第七章 | 第八章 |
//! |------|--------|--------|
//! | 执行单元 | Process（进程即线程） | Thread（线程），Process 仅管理资源 |
//! | 管理器 | PManager | PThreadManager（进程 + 线程双层管理） |
//! | 同步 | 无 | Mutex / Semaphore / Condvar |
//! | task-manage feature | `proc` | `thread` |
//! | 新增依赖 | — | tg-sync |
//!
//! 教程阅读建议：
//!
//! - 先看 `rust_main`：抓住“线程创建 + 双层管理 + trap 分发”总流程；
//! - 再看主循环中 `SEMAPHORE_DOWN/MUTEX_LOCK/CONDVAR_WAIT` 分支：理解阻塞态切换；
//! - 最后看 `impls`：把线程、信号、同步三类系统调用如何交织串起来。

// 不使用标准库
#![no_std]
// 不使用默认 main 入口
#![no_main]
#![cfg_attr(target_arch = "riscv64", deny(warnings, missing_docs))]
#![cfg_attr(not(target_arch = "riscv64"), allow(dead_code, unused_imports))]

/// 文件系统模块：easy-fs 封装 + 统一 Fd 枚举
mod fs;
/// 进程与线程模块：Process（资源容器）和 Thread（执行单元）
mod process;
/// 处理器模块：PROCESSOR 全局管理器（PThreadManager）
mod processor;
/// VirtIO 块设备驱动
mod virtio_block;
/// VirtIO GPU 驱动
mod virtio_gpu;
/// VirtIO 键盘输入驱动
mod virtio_input;

#[macro_use]
extern crate tg_console;

#[macro_use]
extern crate alloc;

use crate::{
    fs::{read_all, FS},
    impls::{Sv39Manager, SyscallContext},
    process::{Process, Thread},
    processor::{ProcManager, ProcessorInner, ThreadManager},
};
use alloc::alloc::alloc;
use core::{alloc::Layout, cell::UnsafeCell, mem::MaybeUninit};
use impls::Console;
pub use processor::PROCESSOR;
use riscv::register::*;
#[cfg(not(target_arch = "riscv64"))]
use stub::Sv39;
use tg_console::log;
use tg_easy_fs::{FSManager, OpenFlags};
use tg_kernel_context::foreign::MultislotPortal;
#[cfg(target_arch = "riscv64")]
use tg_kernel_vm::page_table::Sv39;
use tg_kernel_vm::{
    page_table::{MmuMeta, VAddr, VmFlags, VmMeta, PPN, VPN},
    AddressSpace,
};
use tg_sbi;
use tg_signal::SignalResult;
use tg_syscall::Caller;
use tg_task_manage::ProcId;
use xmas_elf::ElfFile;

/// 构建 VmFlags
#[cfg(target_arch = "riscv64")]
const fn build_flags(s: &str) -> VmFlags<Sv39> {
    VmFlags::build_from_str(s)
}

/// 解析 VmFlags
#[cfg(target_arch = "riscv64")]
fn parse_flags(s: &str) -> Result<VmFlags<Sv39>, ()> {
    s.parse()
}

#[cfg(not(target_arch = "riscv64"))]
use stub::{build_flags, parse_flags};

// 内核入口，栈 = 32 页 = 128 KiB。
//
// 这里不再调用 tg_linker::boot0! 宏，避免外部已发布版本与 Rust 2024
// 在属性语义上的兼容差异影响本 crate 的发布校验。
#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() -> ! {
    const STACK_SIZE: usize = 32 * 4096;
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

/// 内核可用内存窗口大小（QEMU `-m 128M` 下约 126 MiB）。
///
/// DRAM 从 `0x8000_0000` 开始，内核链接地址在 `0x8020_0000`，
/// 因此可用窗口约为 `128MiB - 2MiB = 126MiB`。
const MEMORY: usize = 126 << 20;
/// 异界传送门所在虚页
const PROTAL_TRANSIT: VPN<Sv39> = VPN::MAX;

/// 内核地址空间的全局存储
struct KernelSpace {
    inner: UnsafeCell<MaybeUninit<AddressSpace<Sv39, Sv39Manager>>>,
}

unsafe impl Sync for KernelSpace {}

impl KernelSpace {
    const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    unsafe fn write(&self, space: AddressSpace<Sv39, Sv39Manager>) {
        unsafe { *self.inner.get() = MaybeUninit::new(space) };
    }

    unsafe fn assume_init_ref(&self) -> &AddressSpace<Sv39, Sv39Manager> {
        unsafe { &*(*self.inner.get()).as_ptr() }
    }
}

/// 内核地址空间全局实例
static KERNEL_SPACE: KernelSpace = KernelSpace::new();

/// VirtIO MMIO 设备地址范围
pub const MMIO: &[(usize, usize)] = &[
    (0x1000_1000, 0x00_1000), // VirtIO block (bus.0)
    (0x1000_2000, 0x00_1000), // VirtIO GPU (bus.1)
    (0x1000_3000, 0x00_1000), // VirtIO Input-keyboard (bus.2)
];

/// 内核主函数
///
/// 与第七章相比：
/// - 新增 `init_thread`（线程系统调用）和 `init_sync_mutex`（同步原语系统调用）
/// - 使用 `PThreadManager`（双层管理器）替代 `PManager`
/// - 初始化时同时创建 Process 和 Thread
/// - 主循环中新增**线程阻塞**处理（SEMAPHORE_DOWN/MUTEX_LOCK/CONDVAR_WAIT）
extern "C" fn rust_main() -> ! {
    let layout = tg_linker::KernelLayout::locate();
    // 步骤 1：BSS 清零
    unsafe { layout.zero_bss() };
    // 步骤 2：控制台和日志
    tg_console::init_console(&Console);
    tg_console::set_log_level(option_env!("LOG"));
    tg_console::test_log();
    // 步骤 3：堆分配器
    tg_kernel_alloc::init(layout.start() as _);
    unsafe {
        tg_kernel_alloc::transfer(core::slice::from_raw_parts_mut(
            layout.end() as _,
            MEMORY - layout.len(),
        ))
    };
    // 步骤 4：异界传送门
    let portal_size = MultislotPortal::calculate_size(1);
    let portal_layout = Layout::from_size_align(portal_size, 1 << Sv39::PAGE_BITS).unwrap();
    let portal_ptr = unsafe { alloc(portal_layout) };
    assert!(portal_layout.size() < 1 << Sv39::PAGE_BITS);
    // 步骤 5：内核地址空间
    kernel_space(layout, MEMORY, portal_ptr as _);
    // 步骤 6：外设初始化（GPU + Keyboard）
    if let Err(err) = virtio_gpu::init_gpu() {
        log::info!("init GPU failed: {err}");
    }
    if let Err(err) = virtio_input::init_input() {
        log::info!("init input failed: {err}");
    }
    // 步骤 7：异界传送门初始化
    let portal = unsafe { MultislotPortal::init_transit(PROTAL_TRANSIT.base().val(), 1) };
    // 步骤 8：系统调用初始化
    tg_syscall::init_io(&SyscallContext);
    tg_syscall::init_process(&SyscallContext);
    tg_syscall::init_scheduling(&SyscallContext);
    tg_syscall::init_clock(&SyscallContext);
    tg_syscall::init_memory(&SyscallContext);
    tg_syscall::init_signal(&SyscallContext);
    tg_syscall::init_thread(&SyscallContext);       // 本章新增：线程系统调用
    tg_syscall::init_sync_mutex(&SyscallContext);   // 本章新增：同步原语系统调用
    tg_syscall::init_framebuffer(&SyscallContext);  // ch8 扩展：DOOM framebuffer/input syscall
    // 步骤 9：加载 initproc（返回 Process + Thread）
    let initproc = read_all(FS.open("initproc", OpenFlags::RDONLY).unwrap());
    if let Some((process, thread)) = Process::from_elf(ElfFile::new(initproc.as_slice()).unwrap()) {
        // 初始化双层管理器：ProcManager（进程）+ ThreadManager（线程）
        PROCESSOR.get_mut().set_proc_manager(ProcManager::new());
        PROCESSOR.get_mut().set_manager(ThreadManager::new());
        let (pid, tid) = (process.pid, thread.tid);
        PROCESSOR
            .get_mut()
            .add_proc(pid, process, ProcId::from_usize(usize::MAX));
        PROCESSOR.get_mut().add(tid, thread, pid);
    }

    // ─── 主调度循环 ───
    loop {
        let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
        if let Some(task) = unsafe { (*processor).find_next() } {
            unsafe { task.context.execute(portal, ()) };

            match scause::read().cause() {
                // ─── 系统调用 ───
                scause::Trap::Exception(scause::Exception::UserEnvCall) => {
                    use tg_syscall::{SyscallId as Id, SyscallResult as Ret};
                    let ctx = &mut task.context.context;
                    ctx.move_next();
                    let id: Id = ctx.a(7).into();
                    let args = [ctx.a(0), ctx.a(1), ctx.a(2), ctx.a(3), ctx.a(4), ctx.a(5)];
                    let syscall_ret = tg_syscall::handle(Caller { entity: 0, flow: 0 }, id, args);

                    // ─── 信号处理 ───
                    let current_proc = unsafe { (*processor).get_current_proc().unwrap() };
                    match current_proc.signal.handle_signals(ctx) {
                        SignalResult::ProcessKilled(exit_code) => unsafe {
                            (*processor).make_current_exited(exit_code as _)
                        },
                        _ => match syscall_ret {
                            Ret::Done(ret) => match id {
                                Id::EXIT => unsafe { (*processor).make_current_exited(ret) },
                                // ─── 本章新增：同步原语阻塞处理 ───
                                // 当 semaphore_down / mutex_lock / condvar_wait 返回 -1 时，
                                // 表示资源不可用，将当前线程标记为阻塞态
                                Id::SEMAPHORE_DOWN | Id::MUTEX_LOCK | Id::CONDVAR_WAIT => {
                                    let ctx = &mut task.context.context;
                                    *ctx.a_mut(0) = ret as _;
                                    if ret == -1 {
                                        // 阻塞：从就绪队列移除，等待资源释放后唤醒
                                        unsafe { (*processor).make_current_blocked() };
                                    } else {
                                        // 成功获取：正常挂起（时间片轮转）
                                        unsafe { (*processor).make_current_suspend() };
                                    }
                                }
                                _ => {
                                    let ctx = &mut task.context.context;
                                    *ctx.a_mut(0) = ret as _;
                                    unsafe { (*processor).make_current_suspend() };
                                }
                            },
                            Ret::Unsupported(_) => {
                                log::info!("id = {id:?}");
                                unsafe { (*processor).make_current_exited(-2) };
                            }
                        },
                    }
                }
                e => {
                    log::error!("unsupported trap: {e:?}");
                    unsafe { (*processor).make_current_exited(-3) };
                }
            }
        } else {
            println!("no task");
            break;
        }
    }

    tg_sbi::shutdown(false)
}

/// panic 处理
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("{info}");
    tg_sbi::shutdown(true)
}

/// 建立内核地址空间（与前几章相同）
fn kernel_space(layout: tg_linker::KernelLayout, memory: usize, portal: usize) {
    let mut space = AddressSpace::new();
    for region in layout.iter() {
        log::info!("{region}");
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
        )
    }
    let s = VAddr::<Sv39>::new(layout.end());
    let e = VAddr::<Sv39>::new(layout.start() + memory);
    log::info!("(heap) ---> {:#10x}..{:#10x}", s.val(), e.val());
    space.map_extern(
        s.floor()..e.ceil(),
        PPN::new(s.floor().val()),
        build_flags("_WRV"),
    );
    space.map_extern(
        PROTAL_TRANSIT..PROTAL_TRANSIT + 1,
        PPN::new(portal >> Sv39::PAGE_BITS),
        build_flags("__G_XWRV"),
    );
    println!();
    for (base, len) in MMIO {
        let s = VAddr::<Sv39>::new(*base);
        let e = VAddr::<Sv39>::new(*base + *len);
        log::info!("MMIO range -> {:#10x}..{:#10x}", s.val(), e.val());
        space.map_extern(
            s.floor()..e.ceil(),
            PPN::new(s.floor().val()),
            build_flags("_WRV"),
        );
    }
    unsafe { satp::set(satp::Mode::Sv39, 0, space.root_ppn().val()) };
    unsafe { KERNEL_SPACE.write(space) };
}

/// 将异界传送门映射到用户地址空间
fn map_portal(space: &AddressSpace<Sv39, Sv39Manager>) {
    let portal_idx = PROTAL_TRANSIT.index_in(Sv39::MAX_LEVEL);
    space.root()[portal_idx] = unsafe { KERNEL_SPACE.assume_init_ref() }.root()[portal_idx];
}

/// 各种接口库的实现
///
/// 与第七章相比，本章新增了：
/// - `Thread` trait（thread_create/gettid/waittid）
/// - `SyncMutex` trait（mutex/semaphore/condvar 系统调用）
/// - 所有操作通过 `ProcessorInner`（PThreadManager）进行双层管理
mod impls {
    use crate::{
        build_flags,
        fs::{read_all, Fd, FS},
        processor::ProcessorInner,
        Sv39, Thread, PROCESSOR,
    };
    use alloc::sync::Arc;
    use alloc::{alloc::alloc_zeroed, vec::Vec};
    use core::{
        alloc::Layout,
        ptr::NonNull,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use spin::Mutex;
    use tg_console::log;
    use tg_easy_fs::{make_pipe, FSManager, OpenFlags, UserBuffer};
    use tg_kernel_vm::{
        page_table::{MmuMeta, Pte, VAddr, VmFlags, VmMeta, PPN, VPN},
        PageManager,
    };
    use tg_signal::SignalNo;
    use tg_sync::{Condvar, Mutex as MutexTrait, MutexBlocking, Semaphore};
    use tg_syscall::*;
    use tg_task_manage::{ProcId, ThreadId};
    use xmas_elf::ElfFile;

    // ─── Sv39 页表管理器 ───

    /// Sv39 页表管理器
    #[repr(transparent)]
    pub struct Sv39Manager(NonNull<Pte<Sv39>>);

    impl Sv39Manager {
        const OWNED: VmFlags<Sv39> = unsafe { VmFlags::from_raw(1 << 8) };
        #[inline]
        fn page_alloc<T>(count: usize) -> *mut T {
            unsafe {
                alloc_zeroed(Layout::from_size_align_unchecked(
                    count << Sv39::PAGE_BITS,
                    1 << Sv39::PAGE_BITS,
                ))
            }
            .cast()
        }
    }

    impl PageManager<Sv39> for Sv39Manager {
        #[inline]
        fn new_root() -> Self { Self(NonNull::new(Self::page_alloc(1)).unwrap()) }
        #[inline]
        fn root_ppn(&self) -> PPN<Sv39> { PPN::new(self.0.as_ptr() as usize >> Sv39::PAGE_BITS) }
        #[inline]
        fn root_ptr(&self) -> NonNull<Pte<Sv39>> { self.0 }
        #[inline]
        fn p_to_v<T>(&self, ppn: PPN<Sv39>) -> NonNull<T> {
            unsafe { NonNull::new_unchecked(VPN::<Sv39>::new(ppn.val()).base().as_mut_ptr()) }
        }
        #[inline]
        fn v_to_p<T>(&self, ptr: NonNull<T>) -> PPN<Sv39> {
            PPN::new(VAddr::<Sv39>::new(ptr.as_ptr() as _).floor().val())
        }
        #[inline]
        fn check_owned(&self, pte: Pte<Sv39>) -> bool { pte.flags().contains(Self::OWNED) }
        #[inline]
        fn allocate(&mut self, len: usize, flags: &mut VmFlags<Sv39>) -> NonNull<u8> {
            *flags |= Self::OWNED;
            NonNull::new(Self::page_alloc(len)).unwrap()
        }
        fn deallocate(&mut self, _pte: Pte<Sv39>, _len: usize) -> usize { todo!() }
        fn drop_root(&mut self) { todo!() }
    }

    // ─── 控制台 ───

    /// 控制台实现
    pub struct Console;
    impl tg_console::Console for Console {
        #[inline]
        fn put_char(&self, c: u8) { tg_sbi::console_putchar(c); }
    }

    // ─── 系统调用实现 ───

    /// 系统调用上下文
    pub struct SyscallContext;
    const READABLE: VmFlags<Sv39> = build_flags("RV");
    const WRITEABLE: VmFlags<Sv39> = build_flags("W_V");
    const DEADLOCK_CODE: isize = -0xdead;
    const DOOM_WIDTH: usize = 320;
    const DOOM_HEIGHT: usize = 200;
    const DOOM_BPP: usize = 4;
    const DOOM_FRAME_BYTES: usize = DOOM_WIDTH * DOOM_HEIGHT * DOOM_BPP;
    static FB_FLUSH_COUNT: AtomicUsize = AtomicUsize::new(0);

    /// IO 系统调用（与第七章基本相同）
    ///
    /// 注意：本章通过 `get_current_proc()` 获取当前线程所属的进程，
    /// 而非直接 `current()`，因为 fd_table 属于进程而非线程。
    impl IO for SyscallContext {
        fn write(&self, _caller: Caller, fd: usize, buf: usize, count: usize) -> isize {
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            if let Some(ptr) = current.address_space.translate(VAddr::new(buf), READABLE) {
                if fd == STDOUT || fd == STDDEBUG {
                    print!("{}", unsafe {
                        core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                            ptr.as_ptr(), count,
                        ))
                    });
                    count as _
                } else if let Some(file) = &current.fd_table[fd] {
                    let file = file.lock();
                    if file.writable() {
                        let mut v: Vec<&'static mut [u8]> = Vec::new();
                        unsafe { v.push(core::slice::from_raw_parts_mut(ptr.as_ptr(), count)) };
                        file.write(UserBuffer::new(v)) as _
                    } else { log::error!("file not writable"); -1 }
                } else { log::error!("unsupported fd: {fd}"); -1 }
            } else { log::error!("ptr not readable"); -1 }
        }

        fn read(&self, _caller: Caller, fd: usize, buf: usize, count: usize) -> isize {
            #[repr(C)]
            struct RcoreInputEvent {
                timestamp_usec: u64,
                event_type: u16,
                code: u16,
                value: u32,
            }
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            if let Some(ptr) = current.address_space.translate(VAddr::new(buf), WRITEABLE) {
                if fd == STDIN {
                    let mut ptr = ptr.as_ptr();
                    for _ in 0..count {
                        unsafe { *ptr = tg_sbi::console_getchar() as u8; ptr = ptr.add(1); }
                    }
                    count as _
                } else if let Some(file) = &current.fd_table[fd] {
                    let file = file.lock();
                    match &*file {
                        Fd::Input => {
                            let event_size = core::mem::size_of::<RcoreInputEvent>();
                            if count < event_size {
                                return -1;
                            }
                            if let Some(mut out) = current.address_space.translate::<RcoreInputEvent>(
                                VAddr::new(buf),
                                WRITEABLE,
                            ) {
                                if let Some(event) = crate::virtio_input::pop_event() {
                                    let now_ns = riscv::register::time::read() * 10000 / 125;
                                    *unsafe { out.as_mut() } = RcoreInputEvent {
                                        timestamp_usec: (now_ns / 1000) as u64,
                                        event_type: event.event_type,
                                        code: event.code,
                                        value: event.value,
                                    };
                                    event_size as isize
                                } else {
                                    -1
                                }
                            } else {
                                -1
                            }
                        }
                        _ if file.readable() => {
                            let mut v: Vec<&'static mut [u8]> = Vec::new();
                            unsafe { v.push(core::slice::from_raw_parts_mut(ptr.as_ptr(), count)) };
                            file.read(UserBuffer::new(v)) as _
                        }
                        _ => { log::error!("file not readable"); -1 }
                    }
                } else { log::error!("unsupported fd: {fd}"); -1 }
            } else { log::error!("ptr not writeable"); -1 }
        }

        fn open(&self, _caller: Caller, path: usize, count: usize, flags: usize) -> isize {
            log::info!("sys_openat path={:#x} count={} flags={:#x}", path, count, flags);
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            if let Some(ptr) = current.address_space.translate(VAddr::new(path), READABLE) {
                let path_bytes =
                    unsafe { core::slice::from_raw_parts(ptr.as_ptr() as *const u8, count) };
                let Ok(path_str) = core::str::from_utf8(path_bytes) else {
                    log::info!("open utf8 decode failed");
                    return -1;
                };
                log::info!("open path = {path_str}");
                if path_str == "/dev/fb0" {
                    let new_fd = current.fd_table.len();
                    current.fd_table.push(Some(Mutex::new(Fd::Framebuffer)));
                    log::info!("open /dev/fb0 -> fd {new_fd}");
                    return new_fd as isize;
                }
                if path_str == "/dev/input0" {
                    let new_fd = current.fd_table.len();
                    current.fd_table.push(Some(Mutex::new(Fd::Input)));
                    log::info!("open /dev/input0 -> fd {new_fd}");
                    return new_fd as isize;
                }
                if let Some(file_handle) =
                    FS.open(path_str, OpenFlags::from_bits(flags as u32).unwrap_or(OpenFlags::RDONLY))
                {
                    let new_fd = current.fd_table.len();
                    current.fd_table.push(Some(Mutex::new(Fd::File((*file_handle).clone()))));
                    new_fd as isize
                } else { -1 }
            } else { log::error!("ptr not writeable"); -1 }
        }

        fn lseek(&self, _caller: Caller, fd: usize, offset: isize, whence: usize) -> isize {
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            if fd >= current.fd_table.len() || current.fd_table[fd].is_none() {
                return -1;
            }
            let file = current.fd_table[fd].as_ref().unwrap().lock();
            let Fd::File(file_handle) = &*file else {
                return -1;
            };
            let base = match whence {
                0 => 0isize,
                1 => file_handle.offset.get() as isize,
                2 => {
                    if let Some(inode) = &file_handle.inode {
                        let mut end = 0usize;
                        let mut buf = [0u8; 512];
                        loop {
                            let n = inode.read_at(end, &mut buf);
                            if n == 0 {
                                break;
                            }
                            end += n;
                            if n < buf.len() {
                                break;
                            }
                        }
                        end as isize
                    } else {
                        return -1;
                    }
                }
                _ => return -1,
            };
            let new_off = base + offset;
            if new_off < 0 {
                return -1;
            }
            file_handle.offset.set(new_off as usize);
            new_off
        }

        #[inline]
        fn close(&self, _caller: Caller, fd: usize) -> isize {
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            if fd >= current.fd_table.len() || current.fd_table[fd].is_none() { return -1; }
            current.fd_table[fd].take();
            0
        }

        /// pipe 系统调用
        fn pipe(&self, _caller: Caller, pipe: usize) -> isize {
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            let (read_end, write_end) = make_pipe();
            let read_fd = current.fd_table.len();
            let write_fd = read_fd + 1;
            if let Some(mut ptr) = current.address_space
                .translate::<usize>(VAddr::new(pipe), WRITEABLE)
            { unsafe { *ptr.as_mut() = read_fd }; } else { return -1; }
            if let Some(mut ptr) = current.address_space
                .translate::<usize>(VAddr::new(pipe + core::mem::size_of::<usize>()), WRITEABLE)
            { unsafe { *ptr.as_mut() = write_fd }; } else { return -1; }
            current.fd_table.push(Some(Mutex::new(Fd::PipeRead(read_end))));
            current.fd_table.push(Some(Mutex::new(Fd::PipeWrite(write_end))));
            0
        }

        fn fstat(&self, _caller: Caller, fd: usize, st: usize) -> isize {
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            if fd >= current.fd_table.len() || current.fd_table[fd].is_none() {
                return -1;
            }
            if let Some(mut ptr) = current
                .address_space
                .translate::<Stat>(VAddr::new(st), WRITEABLE)
            {
                *unsafe { ptr.as_mut() } = Stat::new();
                0
            } else {
                -1
            }
        }

        fn ioctl(&self, _caller: Caller, fd: usize, request: usize, argp: usize) -> isize {
            const FB_FLUSH: usize = 1;
            const FB_GET_RESOLUTION: usize = 2;
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            if fd >= current.fd_table.len() || current.fd_table[fd].is_none() {
                return -1;
            }
            let file = current.fd_table[fd].as_ref().unwrap().lock();
            match &*file {
                Fd::Framebuffer => match request {
                    FB_GET_RESOLUTION => {
                        log::info!("ioctl fb get resolution");
                        if let Some(ptr) = current
                            .address_space
                            .translate::<u32>(VAddr::new(argp), WRITEABLE)
                        {
                            unsafe {
                                ptr.as_ptr().write_volatile(640);
                                ptr.as_ptr().add(1).write_volatile(400);
                            }
                            0
                        } else {
                            -1
                        }
                    }
                    FB_FLUSH => {
                        let count = FB_FLUSH_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                        let _ = crate::virtio_gpu::with_framebuffer_mut(|fb, width, height| {
                            let pixels = unsafe {
                                core::slice::from_raw_parts_mut(fb.as_mut_ptr() as *mut u32, fb.len() / 4)
                            };
                            for pixel in pixels.iter_mut() {
                                *pixel |= 0xff00_0000;
                            }
                            if count <= 3 || count % 120 == 0 {
                                let p0 = pixels.first().copied().unwrap_or(0);
                                let p1 = pixels.get((width as usize * height as usize) / 2).copied().unwrap_or(0);
                                log::info!(
                                    "fb_flush #{count}: sample pixels = {:#010x}, {:#010x}",
                                    p0,
                                    p1
                                );
                            }
                        });
                        crate::virtio_gpu::flush().map(|_| 0).unwrap_or(-1)
                    }
                    _ => -1,
                },
                _ => -1,
            }
        }
    }

    /// 进程管理系统调用
    impl Process for SyscallContext {
        #[inline]
        fn exit(&self, _caller: Caller, exit_code: usize) -> isize { exit_code as isize }

        /// fork：创建子进程（返回 Process + Thread）
        fn fork(&self, _caller: Caller) -> isize {
            let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
            let current_proc = unsafe { (*processor).get_current_proc().unwrap() };
            let parent_pid = current_proc.pid;
            let (proc, mut thread) = current_proc.fork().unwrap();
            let pid = proc.pid;
            *thread.context.context.a_mut(0) = 0 as _;
            unsafe {
                (*processor).add_proc(pid, proc, parent_pid);
                (*processor).add(thread.tid, thread, pid);
            }
            pid.get_usize() as isize
        }

        /// exec：从文件系统加载新程序
        fn exec(&self, _caller: Caller, path: usize, count: usize) -> isize {
            const READABLE: VmFlags<Sv39> = build_flags("RV");
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            current.address_space
                .translate(VAddr::new(path), READABLE)
                .map(|ptr| unsafe {
                    core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr.as_ptr(), count))
                })
                .and_then(|name| FS.open(name, OpenFlags::RDONLY))
                .map_or_else(
                    || {
                        log::error!("unknown app, select one in the list: ");
                        FS.readdir("").unwrap().into_iter().for_each(|app| println!("{app}"));
                        println!();
                        -1
                    },
                    |fd| { current.exec(ElfFile::new(&read_all(fd)).unwrap()); 0 },
                )
        }

        fn wait(&self, _caller: Caller, pid: isize, exit_code_ptr: usize) -> isize {
            let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
            let current = unsafe { (*processor).get_current_proc().unwrap() };
            const WRITABLE: VmFlags<Sv39> = build_flags("W_V");
            if let Some((dead_pid, exit_code)) =
                unsafe { (*processor).wait(ProcId::from_usize(pid as usize)) }
            {
                if let Some(mut ptr) = current.address_space
                    .translate::<i32>(VAddr::new(exit_code_ptr), WRITABLE)
                { unsafe { *ptr.as_mut() = exit_code as i32 }; }
                return dead_pid.get_usize() as isize;
            } else { return -1; }
        }

        fn getpid(&self, _caller: Caller) -> isize {
            PROCESSOR.get_mut().get_current_proc().unwrap().pid.get_usize() as _
        }

        fn sbrk(&self, _caller: Caller, size: i32) -> isize {
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            current
                .change_program_brk(size as isize)
                .map(|old_brk| old_brk as isize)
                .unwrap_or(-1)
        }
    }

    impl Scheduling for SyscallContext {
        #[inline]
        fn sched_yield(&self, _caller: Caller) -> isize { 0 }
    }

    impl Clock for SyscallContext {
        #[inline]
        fn clock_gettime(&self, _caller: Caller, clock_id: ClockId, tp: usize) -> isize {
            const WRITABLE: VmFlags<Sv39> = build_flags("W_V");
            match clock_id {
                ClockId::CLOCK_REALTIME
                | ClockId::CLOCK_MONOTONIC
                | ClockId::CLOCK_MONOTONIC_RAW
                | ClockId::CLOCK_REALTIME_COARSE
                | ClockId::CLOCK_MONOTONIC_COARSE
                | ClockId::CLOCK_BOOTTIME
                | ClockId::CLOCK_TAI => {
                    if let Some(mut ptr) = PROCESSOR.get_mut().get_current_proc().unwrap()
                        .address_space.translate(VAddr::new(tp), WRITABLE)
                    {
                        let time = riscv::register::time::read() * 10000 / 125;
                        *unsafe { ptr.as_mut() } = TimeSpec {
                            tv_sec: time / 1_000_000_000,
                            tv_nsec: time % 1_000_000_000,
                        };
                        0
                    } else { log::error!("ptr not readable"); -1 }
                }
                _ => -1,
            }
        }
    }

    impl Memory for SyscallContext {
        fn mmap(
            &self,
            _caller: Caller,
            addr: usize,
            length: usize,
            _prot: i32,
            _flags: i32,
            fd: i32,
            _offset: usize,
        ) -> isize {
            if length == 0 {
                return -1;
            }
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            let base = if addr == 0 { 0x4000_0000 } else { addr };
            let is_fb = fd >= 0
                && (fd as usize) < current.fd_table.len()
                && current.fd_table[fd as usize]
                    .as_ref()
                    .map(|item| matches!(&*item.lock(), Fd::Framebuffer))
                    .unwrap_or(false);

            if is_fb {
                let (fb_ptr, fb_len) = match crate::virtio_gpu::framebuffer_addr_len() {
                    Ok(value) => value,
                    Err(_) => return -1,
                };
                if fb_ptr & ((1 << Sv39::PAGE_BITS) - 1) != 0 {
                    return -1;
                }
                let map_len = core::cmp::min(length, fb_len);
                let start = VAddr::<Sv39>::new(base).floor();
                let end = VAddr::<Sv39>::new(base + map_len).ceil();
                current.address_space.map_extern(
                    start..end,
                    PPN::new(fb_ptr >> Sv39::PAGE_BITS),
                    build_flags("U_WRV"),
                );
                current.fb_map_base = base;
                current.fb_map_len = map_len;
                log::info!(
                    "mmap fb: user={:#x} len={} -> kernel_fb={:#x} fb_len={}",
                    base,
                    map_len,
                    fb_ptr,
                    fb_len
                );
                return base as isize;
            }

            let start = VAddr::<Sv39>::new(base).floor();
            let end = VAddr::<Sv39>::new(base + length).ceil();
            current
                .address_space
                .map(start..end, &[], 0, build_flags("U_WRV"));
            base as isize
        }

        fn munmap(&self, _caller: Caller, addr: usize, length: usize) -> isize {
            if length == 0 {
                return -1;
            }
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            let start = VAddr::<Sv39>::new(addr).floor();
            let end = VAddr::<Sv39>::new(addr + length).ceil();
            current.address_space.unmap(start..end);
            if current.fb_map_base == addr {
                current.fb_map_base = 0;
                current.fb_map_len = 0;
            }
            0
        }
    }

    /// DOOM framebuffer + 输入事件系统调用（ch8 扩展）
    impl tg_syscall::Framebuffer for SyscallContext {
        #[inline]
        fn framebuffer_init(&self, _caller: Caller) -> isize {
            if crate::virtio_gpu::init_gpu().is_err() {
                return -1;
            }
            if crate::virtio_input::init_input().is_err() {
                return -1;
            }
            0
        }

        fn framebuffer_flush(&self, _caller: Caller, buf_ptr: usize, len: usize) -> isize {
            if len < DOOM_FRAME_BYTES {
                return -1;
            }

            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            let Some(ptr) = current.address_space.translate(VAddr::new(buf_ptr), READABLE) else {
                return -1;
            };
            let src = unsafe { core::slice::from_raw_parts(ptr.as_ptr(), DOOM_FRAME_BYTES) };

            let draw_ret = crate::virtio_gpu::with_framebuffer_mut(|fb, width, height| {
                let fb_width = width as usize;
                let fb_height = height as usize;
                if fb_width < DOOM_WIDTH * 2 || fb_height < DOOM_HEIGHT * 2 {
                    return -1;
                }

                let src_pixels =
                    unsafe { core::slice::from_raw_parts(src.as_ptr() as *const u32, DOOM_WIDTH * DOOM_HEIGHT) };
                let dst_pixels =
                    unsafe { core::slice::from_raw_parts_mut(fb.as_mut_ptr() as *mut u32, fb.len() / 4) };

                for y in 0..DOOM_HEIGHT {
                    let src_row = &src_pixels[y * DOOM_WIDTH..(y + 1) * DOOM_WIDTH];
                    let row0 = (y * 2) * fb_width;
                    let row1 = row0 + fb_width;
                    for (x, &pixel) in src_row.iter().enumerate() {
                        let dx = x * 2;
                        let pixel = pixel | 0xff00_0000;
                        dst_pixels[row0 + dx] = pixel;
                        dst_pixels[row0 + dx + 1] = pixel;
                        dst_pixels[row1 + dx] = pixel;
                        dst_pixels[row1 + dx + 1] = pixel;
                    }
                }
                0
            })
            .unwrap_or(-1);

            if draw_ret != 0 {
                return -1;
            }
            if crate::virtio_gpu::flush().is_err() {
                return -1;
            }
            0
        }

        fn framebuffer_info(&self, _caller: Caller, info_ptr: usize) -> isize {
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            if let Some(mut ptr) = current
                .address_space
                .translate::<tg_syscall::FramebufferInfo>(VAddr::new(info_ptr), WRITEABLE)
            {
                *unsafe { ptr.as_mut() } = tg_syscall::FramebufferInfo {
                    width: DOOM_WIDTH as u32,
                    height: DOOM_HEIGHT as u32,
                    stride: (DOOM_WIDTH * DOOM_BPP) as u32,
                    bpp: (DOOM_BPP * 8) as u32,
                };
                0
            } else {
                -1
            }
        }

        fn input_event(&self, _caller: Caller, event_ptr: usize) -> isize {
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            let Some(event) = crate::virtio_input::pop_event() else {
                return -1;
            };
            if let Some(mut ptr) = current
                .address_space
                .translate::<tg_syscall::InputEventUser>(VAddr::new(event_ptr), WRITEABLE)
            {
                *unsafe { ptr.as_mut() } = tg_syscall::InputEventUser {
                    event_type: event.event_type,
                    code: event.code,
                    value: event.value,
                };
                0
            } else {
                -1
            }
        }
    }

    /// 信号系统调用（与第七章相同）
    impl Signal for SyscallContext {
        fn kill(&self, _caller: Caller, pid: isize, signum: u8) -> isize {
            if let Some(target_task) = PROCESSOR.get_mut()
                .get_proc(ProcId::from_usize(pid as usize))
            {
                if let Ok(signal_no) = SignalNo::try_from(signum) {
                    if signal_no != SignalNo::ERR {
                        target_task.signal.add_signal(signal_no);
                        return 0;
                    }
                }
            }
            -1
        }

        fn sigaction(&self, _caller: Caller, signum: u8, action: usize, old_action: usize) -> isize {
            if signum as usize > tg_signal::MAX_SIG { return -1; }
            let current = PROCESSOR.get_mut().get_current_proc().unwrap();
            if let Ok(signal_no) = SignalNo::try_from(signum) {
                if signal_no == SignalNo::ERR { return -1; }
                if old_action as usize != 0 {
                    if let Some(mut ptr) = current.address_space.translate(VAddr::new(old_action), WRITEABLE) {
                        if let Some(signal_action) = current.signal.get_action_ref(signal_no) {
                            *unsafe { ptr.as_mut() } = signal_action;
                        } else { return -1; }
                    } else { return -1; }
                }
                if action as usize != 0 {
                    if let Some(ptr) = current.address_space.translate(VAddr::new(action), READABLE) {
                        if !current.signal.set_action(signal_no, &unsafe { *ptr.as_ptr() }) { return -1; }
                    } else { return -1; }
                }
                return 0;
            }
            -1
        }

        fn sigprocmask(&self, _caller: Caller, mask: usize) -> isize {
            PROCESSOR.get_mut().get_current_proc().unwrap().signal.update_mask(mask) as isize
        }

        fn sigreturn(&self, _caller: Caller) -> isize {
            let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
            let current = unsafe { (*processor).get_current_proc().unwrap() };
            let current_thread = unsafe { (*processor).current().unwrap() };
            if current.signal.sig_return(&mut current_thread.context.context) { 0 } else { -1 }
        }
    }

    /// 线程系统调用（**本章新增**）
    impl tg_syscall::Thread for SyscallContext {
        /// thread_create：在当前进程中创建新线程
        ///
        /// 为新线程分配独立的用户栈（从高地址向下搜索未映射的页面），
        /// 创建新的执行上下文，入口为 entry，参数为 arg。
        fn thread_create(&self, _caller: Caller, entry: usize, arg: usize) -> isize {
            const USER_STACK_PAGES: usize = 16;
            let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
            let current_proc = unsafe { (*processor).get_current_proc().unwrap() };
            // 从最高用户栈位置向下搜索空闲的页表区域
            let mut vpn = VPN::<Sv39>::new((1 << 26) - USER_STACK_PAGES);
            let addrspace = &mut current_proc.address_space;
            loop {
                let idx = vpn.index_in(Sv39::MAX_LEVEL);
                if !addrspace.root()[idx].is_valid() { break; }
                vpn = VPN::<Sv39>::new(vpn.val() - (USER_STACK_PAGES + 1));
            }
            // 分配用户栈
            let stack = unsafe {
                alloc_zeroed(Layout::from_size_align_unchecked(
                    USER_STACK_PAGES << Sv39::PAGE_BITS,
                    1 << Sv39::PAGE_BITS,
                ))
            };
            addrspace.map_extern(
                vpn..vpn + USER_STACK_PAGES,
                PPN::new(stack as usize >> Sv39::PAGE_BITS),
                build_flags("U_WRV"),
            );
            let satp = (8 << 60) | addrspace.root_ppn().val();
            let mut context = tg_kernel_context::LocalContext::user(entry);
            *context.sp_mut() = (vpn + USER_STACK_PAGES).base().val();
            *context.a_mut(0) = arg;
            let thread = Thread::new(satp, context);
            let tid = thread.tid;
            unsafe { (*processor).add(tid, thread, current_proc.pid); }
            tid.get_usize() as _
        }

        /// gettid：获取当前线程 TID
        fn gettid(&self, _caller: Caller) -> isize {
            PROCESSOR.get_mut().current().unwrap().tid.get_usize() as _
        }

        /// waittid：等待指定线程退出
        fn waittid(&self, _caller: Caller, tid: usize) -> isize {
            let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
            let current_thread = unsafe { (*processor).current().unwrap() };
            if tid == current_thread.tid.get_usize() { return -1; }
            if let Some(exit_code) = unsafe { (*processor).waittid(ThreadId::from_usize(tid)) } {
                exit_code
            } else { -1 }
        }
    }

    /// 同步原语系统调用（**本章新增**）
    ///
    /// 实现 Mutex、Semaphore、Condvar 的创建和操作。
    /// 这些同步原语存储在 Process 的列表中，由所有线程共享。
    impl SyncMutex for SyscallContext {
        /// 创建信号量（初始计数 = res_count）
        fn semaphore_create(&self, _caller: Caller, res_count: usize) -> isize {
            let current_proc = PROCESSOR.get_mut().get_current_proc().unwrap();
            let id = if let Some(id) = current_proc.semaphore_list.iter().enumerate()
                .find(|(_, item)| item.is_none()).map(|(id, _)| id)
            {
                current_proc.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
                id
            } else {
                current_proc.semaphore_list.push(Some(Arc::new(Semaphore::new(res_count))));
                current_proc.semaphore_list.len() - 1
            };
            id as isize
        }

        /// V 操作：释放信号量，唤醒等待线程
        fn semaphore_up(&self, _caller: Caller, sem_id: usize) -> isize {
            let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
            let tid = unsafe { (*processor).current().unwrap().tid };
            let pid = unsafe { (*processor).get_current_proc().unwrap().pid };
            let (deadlock_enabled, sem) = {
                let current_proc = unsafe { (*processor).get_proc(pid).unwrap() };
                (
                    current_proc.deadlock,
                    Arc::clone(current_proc.semaphore_list[sem_id].as_ref().unwrap()),
                )
            };
            let waking_tid = sem.up();
            if deadlock_enabled {
                let current_proc = unsafe { (*processor).get_proc(pid).unwrap() };
                current_proc.record_semaphore_up(tid, sem_id, waking_tid);
            }
            if let Some(tid) = waking_tid {
                unsafe { (*processor).re_enque(tid); }
            }
            0
        }

        /// P 操作：获取信号量，不可用则阻塞
        fn semaphore_down(&self, _caller: Caller, sem_id: usize) -> isize {
            let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
            let tid = unsafe { (*processor).current().unwrap().tid };
            let pid = unsafe { (*processor).get_current_proc().unwrap().pid };
            let (deadlock_enabled, available, sem) = {
                let current_proc = unsafe { (*processor).get_proc(pid).unwrap() };
                let available = if current_proc.deadlock {
                    current_proc
                        .semaphore_list
                        .iter()
                        .map(|item| {
                            item.as_ref()
                                .map(|sem| sem.inner.exclusive_access().count.max(0) as usize)
                                .unwrap_or(0)
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                (
                    current_proc.deadlock,
                    available,
                    Arc::clone(current_proc.semaphore_list[sem_id].as_ref().unwrap()),
                )
            };
            if deadlock_enabled && available.get(sem_id).copied().unwrap_or(0) == 0 {
                let threads = unsafe { (*processor).get_thread(pid).unwrap().clone() };
                let current_proc = unsafe { (*processor).get_proc(pid).unwrap() };
                if current_proc.would_semaphore_deadlock(tid, sem_id, &threads, &available) {
                    return DEADLOCK_CODE;
                }
            }
            if !sem.down(tid) {
                if deadlock_enabled {
                    let current_proc = unsafe { (*processor).get_proc(pid).unwrap() };
                    current_proc.record_semaphore_wait(tid, sem_id);
                }
                -1
            } else {
                if deadlock_enabled {
                    let current_proc = unsafe { (*processor).get_proc(pid).unwrap() };
                    current_proc.record_semaphore_down(tid, sem_id);
                }
                0
            }
        }

        /// 创建互斥锁（blocking=true 为阻塞锁）
        fn mutex_create(&self, _caller: Caller, blocking: bool) -> isize {
            let new_mutex: Option<Arc<dyn MutexTrait>> = if blocking {
                Some(Arc::new(MutexBlocking::new()))
            } else { None };
            let current_proc = PROCESSOR.get_mut().get_current_proc().unwrap();
            if let Some(id) = current_proc.mutex_list.iter().enumerate()
                .find(|(_, item)| item.is_none()).map(|(id, _)| id)
            {
                current_proc.mutex_list[id] = new_mutex;
                id as isize
            } else {
                current_proc.mutex_list.push(new_mutex);
                current_proc.mutex_list.len() as isize - 1
            }
        }

        /// 解锁，唤醒等待线程
        fn mutex_unlock(&self, _caller: Caller, mutex_id: usize) -> isize {
            let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
            let pid = unsafe { (*processor).get_current_proc().unwrap().pid };
            let (deadlock_enabled, mutex) = {
                let current_proc = unsafe { (*processor).get_proc(pid).unwrap() };
                (
                    current_proc.deadlock,
                    Arc::clone(current_proc.mutex_list[mutex_id].as_ref().unwrap()),
                )
            };
            let waking_tid = mutex.unlock();
            if deadlock_enabled {
                let current_proc = unsafe { (*processor).get_proc(pid).unwrap() };
                current_proc.record_mutex_unlock(mutex_id, waking_tid);
            }
            if let Some(tid) = waking_tid {
                unsafe { (*processor).re_enque(tid); }
            }
            0
        }

        /// 加锁，已被占用则阻塞
        fn mutex_lock(&self, _caller: Caller, mutex_id: usize) -> isize {
            let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
            let tid = unsafe { (*processor).current().unwrap().tid };
            let pid = unsafe { (*processor).get_current_proc().unwrap().pid };
            let (deadlock_enabled, should_reject, mutex) = {
                let current_proc = unsafe { (*processor).get_proc(pid).unwrap() };
                (
                    current_proc.deadlock,
                    current_proc.deadlock && current_proc.would_mutex_deadlock(tid, mutex_id),
                    Arc::clone(current_proc.mutex_list[mutex_id].as_ref().unwrap()),
                )
            };
            if should_reject {
                return DEADLOCK_CODE;
            }
            if !mutex.lock(tid) {
                if deadlock_enabled {
                    let current_proc = unsafe { (*processor).get_proc(pid).unwrap() };
                    current_proc.record_mutex_wait(tid, mutex_id);
                }
                -1
            } else {
                if deadlock_enabled {
                    let current_proc = unsafe { (*processor).get_proc(pid).unwrap() };
                    current_proc.record_mutex_lock(tid, mutex_id);
                }
                0
            }
        }

        /// 创建条件变量
        fn condvar_create(&self, _caller: Caller, _arg: usize) -> isize {
            let current_proc = PROCESSOR.get_mut().get_current_proc().unwrap();
            let id = if let Some(id) = current_proc.condvar_list.iter().enumerate()
                .find(|(_, item)| item.is_none()).map(|(id, _)| id)
            {
                current_proc.condvar_list[id] = Some(Arc::new(Condvar::new()));
                id
            } else {
                current_proc.condvar_list.push(Some(Arc::new(Condvar::new())));
                current_proc.condvar_list.len() - 1
            };
            id as isize
        }

        /// 唤醒一个等待线程
        fn condvar_signal(&self, _caller: Caller, condvar_id: usize) -> isize {
            let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
            let current_proc = unsafe { (*processor).get_current_proc().unwrap() };
            let condvar = Arc::clone(current_proc.condvar_list[condvar_id].as_ref().unwrap());
            if let Some(tid) = condvar.signal() {
                unsafe { (*processor).re_enque(tid); }
            }
            0
        }

        /// 等待条件变量（释放锁 + 阻塞 + 重新获取锁）
        fn condvar_wait(&self, _caller: Caller, condvar_id: usize, mutex_id: usize) -> isize {
            let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
            let tid = unsafe { (*processor).current().unwrap().tid };
            let pid = unsafe { (*processor).get_current_proc().unwrap().pid };
            let (deadlock_enabled, condvar, mutex) = {
                let current_proc = unsafe { (*processor).get_proc(pid).unwrap() };
                (
                    current_proc.deadlock,
                    Arc::clone(current_proc.condvar_list[condvar_id].as_ref().unwrap()),
                    Arc::clone(current_proc.mutex_list[mutex_id].as_ref().unwrap()),
                )
            };
            let (flag, waking_tid) = condvar.wait_with_mutex(tid, mutex);
            if deadlock_enabled {
                let current_proc = unsafe { (*processor).get_proc(pid).unwrap() };
                current_proc.record_mutex_unlock(mutex_id, waking_tid);
                if flag {
                    current_proc.record_mutex_lock(tid, mutex_id);
                } else {
                    current_proc.record_mutex_wait(tid, mutex_id);
                }
            }
            if let Some(waking_tid) = waking_tid {
                unsafe { (*processor).re_enque(waking_tid); }
            }
            if !flag { -1 } else { 0 }
        }

        /// 死锁检测（TODO 练习题）
        fn enable_deadlock_detect(&self, _caller: Caller, is_enable: i32) -> isize {
            let current_proc = PROCESSOR.get_mut().get_current_proc().unwrap();
            match is_enable {
                0 => {
                    current_proc.set_deadlock_detect(false);
                    0
                }
                1 => {
                    current_proc.set_deadlock_detect(true);
                    0
                }
                _ => -1,
            }
        }
    }
}

/// 非 RISC-V64 架构的占位实现
#[cfg(not(target_arch = "riscv64"))]
mod stub {
    use tg_kernel_vm::page_table::{MmuMeta, VmFlags};

    /// Sv39 占位类型
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
    pub struct Sv39;
    impl MmuMeta for Sv39 {
        const P_ADDR_BITS: usize = 56;
        const PAGE_BITS: usize = 12;
        const LEVEL_BITS: &'static [usize] = &[9, 9, 9];
        const PPN_POS: usize = 10;
        #[inline]
        fn is_leaf(value: usize) -> bool { value & 0b1110 != 0 }
    }
    /// 构建 VmFlags 占位
    pub const fn build_flags(_s: &str) -> VmFlags<Sv39> { unsafe { VmFlags::from_raw(0) } }
    /// 解析 VmFlags 占位
    pub fn parse_flags(_s: &str) -> Result<VmFlags<Sv39>, ()> { Ok(unsafe { VmFlags::from_raw(0) }) }

    #[unsafe(no_mangle)]
    pub extern "C" fn main() -> i32 { 0 }
    #[unsafe(no_mangle)]
    pub extern "C" fn __libc_start_main() -> i32 { 0 }
    #[unsafe(no_mangle)]
    pub extern "C" fn rust_eh_personality() {}
}
