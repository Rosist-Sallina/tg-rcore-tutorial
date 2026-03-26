#![cfg_attr(feature = "exercise", allow(dead_code, unused_variables))]

use crate::{
    Sv39, build_flags, impls::Sv39Manager, pager::FaultKind, pager::Pager, parse_flags,
};
use alloc::alloc::alloc_zeroed;
use core::{alloc::Layout, ops::Range, ptr::NonNull};
use tg_kernel_context::{LocalContext, foreign::ForeignContext};
use tg_kernel_vm::{
    AddressSpace,
    page_table::{MmuMeta, PPN, VAddr, VPN, VmFlags},
};
use tg_syscall::VmStats;
use xmas_elf::{
    ElfFile,
    header::{self, HeaderPt2, Machine},
    program,
};

const PAGE_SIZE: usize = 1 << Sv39::PAGE_BITS;
const PAGE_MASK: usize = PAGE_SIZE - 1;

pub(crate) struct Process {
    pub(crate) context: ForeignContext,
    pub(crate) address_space: AddressSpace<Sv39, Sv39Manager>,
    heap_bottom: usize,
    program_brk: usize,
    pager: Pager,
}

impl Process {
    pub(crate) fn new(elf: ElfFile) -> Option<Self> {
        let entry = match elf.header.pt2 {
            HeaderPt2::Header64(pt2)
                if pt2.type_.as_type() == header::Type::Executable
                    && pt2.machine.as_machine() == Machine::RISC_V =>
            {
                pt2.entry_point as usize
            }
            _ => None?,
        };

        let mut address_space = AddressSpace::new();
        let mut max_end_va: usize = 0;
        for program in elf.program_iter() {
            if !matches!(program.get_type(), Ok(program::Type::Load)) {
                continue;
            }
            let off_file = program.offset() as usize;
            let len_file = program.file_size() as usize;
            let off_mem = program.virtual_addr() as usize;
            let end_mem = off_mem + program.mem_size() as usize;
            assert_eq!(off_file & PAGE_MASK, off_mem & PAGE_MASK);
            max_end_va = max_end_va.max(end_mem);

            let mut flags: [u8; 7] = *b"U___V__";
            if program.flags().is_execute() {
                flags[1] = b'X';
            }
            if program.flags().is_write() {
                flags[2] = b'W';
            }
            if program.flags().is_read() {
                flags[3] = b'R';
            }
            address_space.map(
                VAddr::new(off_mem).floor()..VAddr::new(end_mem).ceil(),
                &elf.input[off_file..][..len_file],
                off_mem & PAGE_MASK,
                parse_flags(unsafe { core::str::from_utf8_unchecked(&flags) }).unwrap(),
            );
        }

        let heap_bottom = VAddr::<Sv39>::new(max_end_va).ceil().base().val();
        let stack = unsafe {
            alloc_zeroed(Layout::from_size_align_unchecked(
                2 << Sv39::PAGE_BITS,
                1 << Sv39::PAGE_BITS,
            ))
        };
        address_space.map_extern(
            VPN::new((1 << 26) - 2)..VPN::new(1 << 26),
            PPN::new(stack as usize >> Sv39::PAGE_BITS),
            build_flags("U_WRV"),
        );

        let mut context = LocalContext::user(entry);
        let satp = (8 << 60) | address_space.root_ppn().val();
        *context.sp_mut() = 1 << 38;
        Some(Self {
            context: ForeignContext { context, satp },
            address_space,
            heap_bottom,
            program_brk: heap_bottom,
            pager: Pager::new(),
        })
    }

    pub(crate) fn translate_ptr<T>(
        &self,
        addr: usize,
        flags: VmFlags<Sv39>,
    ) -> Option<NonNull<T>> {
        self.address_space.translate(VAddr::new(addr), flags)
    }

    pub(crate) fn change_program_brk(&mut self, size: isize) -> Option<usize> {
        #[cfg(feature = "exercise")]
        {
            todo!(
                "练习题：将 sbrk 改成懒分配。这里需要只登记新页区间，缩容时撤销对应 pageable 区间"
            );
        }
        #[cfg(not(feature = "exercise"))]
        {
        let old_brk = self.program_brk;
        let new_brk = self.program_brk as isize + size;
        if new_brk < self.heap_bottom as isize {
            return None;
        }
        let new_brk = new_brk as usize;
        let old_brk_ceil = VAddr::<Sv39>::new(old_brk).ceil();
        let new_brk_ceil = VAddr::<Sv39>::new(new_brk).ceil();
        if size > 0 && new_brk_ceil.val() > old_brk_ceil.val() {
            self.register_pageable_range(old_brk_ceil..new_brk_ceil, build_flags("U_WRV"))?;
        } else if size < 0 && old_brk_ceil.val() > new_brk_ceil.val() {
            if !self.pager.remove_range(&mut self.address_space, new_brk_ceil..old_brk_ceil) {
                return None;
            }
        }
        self.program_brk = new_brk;
        Some(old_brk)
        }
    }

    pub(crate) fn mmap_anonymous(&mut self, addr: usize, len: usize, prot: i32) -> Option<()> {
        #[cfg(feature = "exercise")]
        {
            todo!(
                "练习题：实现匿名映射的 lazy register。检查参数合法性后，只登记 pageable 虚拟页区间"
            );
        }
        #[cfg(not(feature = "exercise"))]
        {
        if addr & PAGE_MASK != 0 || prot & !0x7 != 0 || prot & 0x7 == 0 {
            return None;
        }
        let vpn_range = self.checked_vpn_range(addr, len)?;
        if vpn_range.start == vpn_range.end {
            return Some(());
        }
        self.register_pageable_range(vpn_range, prot_to_flags(prot))
        }
    }

    pub(crate) fn munmap_anonymous(&mut self, addr: usize, len: usize) -> Option<()> {
        let vpn_range = self.checked_vpn_range(addr, len)?;
        if vpn_range.start == vpn_range.end {
            return Some(());
        }
        self.pager
            .remove_range(&mut self.address_space, vpn_range)
            .then_some(())
    }

    pub(crate) fn handle_page_fault(&mut self, kind: FaultKind, addr: usize) -> bool {
        #[cfg(feature = "exercise")]
        {
            todo!(
                "练习题：根据 fault 地址和访问类型恢复合法缺页。最终应委托 pager 做 fault-in / swap-in"
            );
        }
        #[cfg(not(feature = "exercise"))]
        {
        self.pager
            .handle_page_fault(&mut self.address_space, kind, addr)
        }
    }

    pub(crate) fn vm_set_algo(&mut self, raw: usize) -> bool {
        self.pager.set_algo(raw)
    }

    pub(crate) fn vm_set_quota(&mut self, quota: usize) -> bool {
        self.pager.set_quota(&mut self.address_space, quota)
    }

    pub(crate) fn vm_reset_stats(&mut self) {
        self.pager.reset_stats();
    }

    pub(crate) fn vm_snapshot_stats(&self) -> VmStats {
        self.pager.snapshot_stats()
    }

    fn register_pageable_range(
        &mut self,
        range: Range<VPN<Sv39>>,
        flags: VmFlags<Sv39>,
    ) -> Option<()> {
        if !self.pager.can_register(&self.address_space, range.start..range.end) {
            return None;
        }
        self.pager.register_range(range, flags);
        Some(())
    }

    fn checked_vpn_range(&self, addr: usize, len: usize) -> Option<Range<VPN<Sv39>>> {
        if addr & PAGE_MASK != 0 {
            return None;
        }
        let end = addr.checked_add(len)?;
        Some(VAddr::new(addr).floor()..VAddr::new(end).ceil())
    }
}

fn prot_to_flags(prot: i32) -> VmFlags<Sv39> {
    let mut flags: [u8; 5] = *b"U___V";
    if prot & 0b100 != 0 {
        flags[1] = b'X';
    }
    if prot & 0b010 != 0 {
        flags[2] = b'W';
    }
    if prot & 0b001 != 0 {
        flags[3] = b'R';
    }
    parse_flags(unsafe { core::str::from_utf8_unchecked(&flags) }).unwrap()
}
