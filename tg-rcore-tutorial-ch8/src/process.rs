//! 进程与线程管理模块
//!
//! ## 与第七章的区别
//!
//! 第七章中 `Process` 既是资源容器又是执行单元。
//! 第八章将两者分离：
//! - **Process**：资源容器，管理地址空间、文件描述符、**同步原语列表**、信号
//! - **Thread**：执行单元，管理 TID 和上下文
//!
//! 同一进程的所有线程共享 `Process` 中的资源。
//!
//! ## 新增字段
//!
//! | 字段 | 说明 |
//! |------|------|
//! | `semaphore_list` | 信号量列表（进程内所有线程共享） |
//! | `mutex_list` | 互斥锁列表 |
//! | `condvar_list` | 条件变量列表 |
//!
//! 教程阅读建议：
//!
//! - 先看 `Process` 与 `Thread` 的字段分工：明确“资源归进程、执行归线程”；
//! - 再看 `fork/exec/from_elf`：理解跨线程模型后，进程复制与替换语义如何变化；
//! - 最后结合 `processor.rs` 看线程生命周期与进程资源回收的关系。

use crate::{
    build_flags, fs::Fd, map_portal, parse_flags, processor::ProcessorInner, Sv39, Sv39Manager,
    PROCESSOR,
};
use alloc::{alloc::alloc_zeroed, boxed::Box, collections::BTreeMap, sync::Arc, vec::Vec};
use core::alloc::Layout;
use spin::Mutex;
use tg_kernel_context::{foreign::ForeignContext, LocalContext};
use tg_kernel_vm::{
    page_table::{MmuMeta, VAddr, PPN, VPN},
    AddressSpace,
};
use tg_signal::Signal;
use tg_signal_impl::SignalImpl;
use tg_sync::{Condvar, Mutex as MutexTrait, Semaphore};
use tg_task_manage::{ProcId, ThreadId};
use xmas_elf::{
    header::{self, HeaderPt2, Machine},
    program, ElfFile,
};

/// 线程（执行单元）
///
/// 每个线程有独立的 TID 和上下文（寄存器状态、satp）。
/// 同一进程的多个线程共享地址空间。
pub struct Thread {
    /// 线程 ID（不可变）
    pub tid: ThreadId,
    /// 执行上下文（包含 LocalContext + satp）
    pub context: ForeignContext,
}

impl Thread {
    /// 创建新线程
    pub fn new(satp: usize, context: LocalContext) -> Self {
        Self {
            tid: ThreadId::new(),
            context: ForeignContext { context, satp },
        }
    }
}

/// 进程（资源容器）
///
/// 管理地址空间、文件描述符、同步原语、信号等共享资源。
/// 一个进程可以包含多个线程。
pub struct Process {
    /// 进程 ID
    pub pid: ProcId,
    /// 地址空间（所有线程共享）
    pub address_space: AddressSpace<Sv39, Sv39Manager>,
    /// 文件描述符表（所有线程共享）
    pub fd_table: Vec<Option<Mutex<Fd>>>,
    /// 信号处理器
    pub signal: Box<dyn Signal>,
    /// 信号量列表（**本章新增**，所有线程共享）
    pub semaphore_list: Vec<Option<Arc<Semaphore>>>,
    /// 互斥锁列表（**本章新增**，所有线程共享）
    pub mutex_list: Vec<Option<Arc<dyn MutexTrait>>>,
    /// 条件变量列表（**本章新增**，所有线程共享）
    pub condvar_list: Vec<Option<Arc<Condvar>>>,

    /// 是否为当前进程启用死锁检测。
    pub deadlock: bool,
    mutex_owner: Vec<Option<ThreadId>>,
    mutex_wait: BTreeMap<ThreadId, usize>,
    semaphore_allocation: BTreeMap<ThreadId, Vec<usize>>,
    semaphore_wait: BTreeMap<ThreadId, usize>,
}

impl Process {
    fn reset_deadlock_state(&mut self) {
        self.mutex_owner = vec![None; self.mutex_list.len()];
        self.mutex_wait.clear();
        self.semaphore_allocation.clear();
        self.semaphore_wait.clear();
    }

    fn ensure_mutex_slot(&mut self, mutex_id: usize) {
        if self.mutex_owner.len() <= mutex_id {
            self.mutex_owner.resize(mutex_id + 1, None);
        }
    }

    fn ensure_semaphore_slot(alloc: &mut Vec<usize>, sem_id: usize) {
        if alloc.len() <= sem_id {
            alloc.resize(sem_id + 1, 0);
        }
    }

    fn semaphore_allocation_mut(&mut self, tid: ThreadId, sem_id: usize) -> &mut Vec<usize> {
        let alloc = self.semaphore_allocation.entry(tid).or_insert_with(Vec::new);
        Self::ensure_semaphore_slot(alloc, sem_id);
        alloc
    }

    fn semaphore_allocation_of(&self, tid: ThreadId) -> Option<&Vec<usize>> {
        self.semaphore_allocation.get(&tid)
    }

    /// 切换当前进程的死锁检测开关，并重置对应的跟踪状态。
    pub fn set_deadlock_detect(&mut self, is_enable: bool) {
        self.deadlock = is_enable;
        self.reset_deadlock_state();
    }

    /// 判断一次阻塞互斥锁请求是否会形成等待环路。
    pub fn would_mutex_deadlock(&self, tid: ThreadId, mutex_id: usize) -> bool {
        let Some(owner) = self.mutex_owner.get(mutex_id).and_then(|owner| *owner) else {
            return false;
        };
        if owner == tid {
            return true;
        }
        let mut current = owner;
        for _ in 0..=self.mutex_wait.len() {
            if current == tid {
                return true;
            }
            let Some(wait_mutex_id) = self.mutex_wait.get(&current).copied() else {
                return false;
            };
            let Some(next_owner) = self.mutex_owner.get(wait_mutex_id).and_then(|owner| *owner)
            else {
                return false;
            };
            current = next_owner;
        }
        false
    }

    /// 记录互斥锁获取成功后的所有权。
    pub fn record_mutex_lock(&mut self, tid: ThreadId, mutex_id: usize) {
        self.ensure_mutex_slot(mutex_id);
        self.mutex_wait.remove(&tid);
        self.mutex_owner[mutex_id] = Some(tid);
    }

    /// 记录互斥锁阻塞等待关系。
    pub fn record_mutex_wait(&mut self, tid: ThreadId, mutex_id: usize) {
        self.ensure_mutex_slot(mutex_id);
        self.mutex_wait.insert(tid, mutex_id);
    }

    /// 记录互斥锁释放后的所有权转移。
    pub fn record_mutex_unlock(&mut self, mutex_id: usize, waking_tid: Option<ThreadId>) {
        self.ensure_mutex_slot(mutex_id);
        match waking_tid {
            Some(tid) => {
                self.mutex_wait.remove(&tid);
                self.mutex_owner[mutex_id] = Some(tid);
            }
            None => self.mutex_owner[mutex_id] = None,
        }
    }

    /// 判断一次信号量请求是否会让当前等待状态变为不安全。
    pub fn would_semaphore_deadlock(
        &self,
        tid: ThreadId,
        sem_id: usize,
        threads: &[ThreadId],
        available: &[usize],
    ) -> bool {
        let mut work = available.to_vec();
        let mut finish = vec![false; threads.len()];
        let mut requests = self.semaphore_wait.clone();
        requests.insert(tid, sem_id);

        loop {
            let mut progressed = false;
            for (idx, thread) in threads.iter().copied().enumerate() {
                if finish[idx] {
                    continue;
                }
                let can_finish = match requests.get(&thread).copied() {
                    Some(request_sem) => work.get(request_sem).copied().unwrap_or(0) > 0,
                    None => true,
                };
                if !can_finish {
                    continue;
                }
                finish[idx] = true;
                progressed = true;
                if let Some(allocation) = self.semaphore_allocation_of(thread) {
                    for (resource_id, count) in allocation.iter().copied().enumerate() {
                        if resource_id >= work.len() {
                            break;
                        }
                        work[resource_id] += count;
                    }
                }
            }
            if !progressed {
                break;
            }
        }

        finish.into_iter().any(|done| !done)
    }

    /// 记录信号量获取成功后的分配关系。
    pub fn record_semaphore_down(&mut self, tid: ThreadId, sem_id: usize) {
        self.semaphore_wait.remove(&tid);
        let alloc = self.semaphore_allocation_mut(tid, sem_id);
        alloc[sem_id] += 1;
    }

    /// 记录信号量阻塞等待关系。
    pub fn record_semaphore_wait(&mut self, tid: ThreadId, sem_id: usize) {
        self.semaphore_wait.insert(tid, sem_id);
    }

    /// 记录信号量释放及被唤醒线程的资源交接。
    pub fn record_semaphore_up(&mut self, tid: ThreadId, sem_id: usize, waking_tid: Option<ThreadId>) {
        let alloc = self.semaphore_allocation_mut(tid, sem_id);
        if alloc[sem_id] > 0 {
            alloc[sem_id] -= 1;
        }
        if let Some(waking_tid) = waking_tid {
            self.semaphore_wait.remove(&waking_tid);
            let alloc = self.semaphore_allocation_mut(waking_tid, sem_id);
            alloc[sem_id] += 1;
        }
    }

    /// exec：替换当前进程的地址空间和主线程上下文
    ///
    /// 注意：只支持单线程进程执行 exec
    pub fn exec(&mut self, elf: ElfFile) {
        let (proc, thread) = Process::from_elf(elf).unwrap();
        self.address_space = proc.address_space;
        let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
        unsafe {
            let pthreads = (*processor).get_thread(self.pid).unwrap();
            (*processor).get_task(pthreads[0]).unwrap().context = thread.context;
        }
    }

    /// fork：创建子进程（复制地址空间和主线程上下文）
    ///
    /// 子进程继承父进程的地址空间（深拷贝）、文件描述符和信号配置。
    /// 同步原语列表不继承（子进程创建空的列表）。
    pub fn fork(&mut self) -> Option<(Self, Thread)> {
        let pid = ProcId::new();
        // 深拷贝地址空间
        let parent_addr_space = &self.address_space;
        let mut address_space: AddressSpace<Sv39, Sv39Manager> = AddressSpace::new();
        parent_addr_space.cloneself(&mut address_space);
        map_portal(&address_space);
        // 复制主线程上下文
        let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
        let pthreads = unsafe { (*processor).get_thread(self.pid).unwrap() };
        let context = unsafe {
            (*processor).get_task(pthreads[0]).unwrap().context.context.clone()
        };
        let satp = (8 << 60) | address_space.root_ppn().val();
        let thread = Thread::new(satp, context);
        // 复制文件描述符表
        let new_fd_table: Vec<Option<Mutex<Fd>>> = self.fd_table
            .iter()
            .map(|fd| fd.as_ref().map(|f| Mutex::new(f.lock().clone())))
            .collect();
        Some((
            Self {
                pid,
                address_space,
                fd_table: new_fd_table,
                signal: self.signal.from_fork(),
                // 子进程的同步原语列表初始为空
                semaphore_list: Vec::new(),
                mutex_list: Vec::new(),
                condvar_list: Vec::new(),
                deadlock: false,
                mutex_owner: Vec::new(),
                mutex_wait: BTreeMap::new(),
                semaphore_allocation: BTreeMap::new(),
                semaphore_wait: BTreeMap::new(),
            },
            thread,
        ))
    }

    /// 从 ELF 文件创建进程和主线程
    ///
    /// 解析 ELF 段，建立地址空间，分配用户栈，创建初始上下文。
    pub fn from_elf(elf: ElfFile) -> Option<(Self, Thread)> {
        let entry = match elf.header.pt2 {
            HeaderPt2::Header64(pt2)
                if pt2.type_.as_type() == header::Type::Executable
                    && pt2.machine.as_machine() == Machine::RISC_V =>
            { pt2.entry_point as usize }
            _ => None?,
        };

        const PAGE_SIZE: usize = 1 << Sv39::PAGE_BITS;
        const PAGE_MASK: usize = PAGE_SIZE - 1;

        let mut address_space = AddressSpace::new();
        for program in elf.program_iter() {
            if !matches!(program.get_type(), Ok(program::Type::Load)) { continue; }
            let off_file = program.offset() as usize;
            let len_file = program.file_size() as usize;
            let off_mem = program.virtual_addr() as usize;
            let end_mem = off_mem + program.mem_size() as usize;
            assert_eq!(off_file & PAGE_MASK, off_mem & PAGE_MASK);
            let mut flags: [u8; 5] = *b"U___V";
            if program.flags().is_execute() { flags[1] = b'X'; }
            if program.flags().is_write() { flags[2] = b'W'; }
            if program.flags().is_read() { flags[3] = b'R'; }
            address_space.map(
                VAddr::new(off_mem).floor()..VAddr::new(end_mem).ceil(),
                &elf.input[off_file..][..len_file],
                off_mem & PAGE_MASK,
                parse_flags(unsafe { core::str::from_utf8_unchecked(&flags) }).unwrap(),
            );
        }
        // 分配 2 页用户栈
        let stack = unsafe {
            alloc_zeroed(Layout::from_size_align_unchecked(
                2 << Sv39::PAGE_BITS, 1 << Sv39::PAGE_BITS,
            ))
        };
        address_space.map_extern(
            VPN::new((1 << 26) - 2)..VPN::new(1 << 26),
            PPN::new(stack as usize >> Sv39::PAGE_BITS),
            build_flags("U_WRV"),
        );
        map_portal(&address_space);
        let satp = (8 << 60) | address_space.root_ppn().val();
        let mut context = LocalContext::user(entry);
        *context.sp_mut() = 1 << 38;
        let thread = Thread::new(satp, context);

        Some((
            Self {
                pid: ProcId::new(),
                address_space,
                fd_table: vec![
                    // stdin
                    Some(Mutex::new(Fd::Empty { read: true, write: false })),
                    // stdout
                    Some(Mutex::new(Fd::Empty { read: false, write: true })),
                    // stderr
                    Some(Mutex::new(Fd::Empty { read: false, write: true })),
                ],
                signal: Box::new(SignalImpl::new()),
                semaphore_list: Vec::new(),
                mutex_list: Vec::new(),
                condvar_list: Vec::new(),
                deadlock: false,
                mutex_owner: Vec::new(),
                mutex_wait: BTreeMap::new(),
                semaphore_allocation: BTreeMap::new(),
                semaphore_wait: BTreeMap::new(),
            },
            thread,
        ))
    }
}
