#![cfg_attr(feature = "exercise", allow(dead_code, unused_imports, unused_variables))]

extern crate alloc;

use crate::{Sv39, build_flags, impls::Sv39Manager};
use alloc::{
    alloc::alloc_zeroed,
    boxed::Box,
    collections::BTreeMap,
    vec,
    vec::Vec,
};
use core::{
    alloc::Layout,
    ops::Range,
};
use tg_console::log;
use tg_kernel_vm::{
    AddressSpace,
    page_table::{MmuMeta, PPN, VAddr, VPN, VmFlags},
};
use tg_syscall::{VmAlgo, VmStats};

const PAGE_SIZE: usize = 1 << Sv39::PAGE_BITS;
const DEFAULT_QUOTA: usize = 16;
const MAX_QUOTA: usize = 32;
const WORKING_SET_TAU: u64 = 8;
const THRASH_STEP: usize = 4;
const FLAG_A: VmFlags<Sv39> = build_flags("A");
const FLAG_D: VmFlags<Sv39> = build_flags("D");

#[derive(Clone, Copy)]
pub(crate) enum FaultKind {
    Load,
    Store,
    Instruction,
}

#[derive(Clone)]
struct VmRegion {
    range: Range<VPN<Sv39>>,
}

#[derive(Clone, Copy)]
enum PageBacking {
    ZeroFill,
    Swap(usize),
}

struct PageState {
    flags: VmFlags<Sv39>,
    backing: PageBacking,
    resident: bool,
}

#[derive(Clone, Copy)]
struct ResidentPage {
    vpn: VPN<Sv39>,
    ppn: PPN<Sv39>,
    loaded_at: u64,
    last_epoch: u64,
    age: u8,
}

struct RuntimeStats {
    page_faults: u64,
    evictions: u64,
    write_backs: u64,
    thrash_events: u64,
    start_time_ns: u64,
    start_instret: u64,
    window_faults_base: u64,
    window_evictions_base: u64,
}

impl RuntimeStats {
    const fn new() -> Self {
        Self {
            page_faults: 0,
            evictions: 0,
            write_backs: 0,
            thrash_events: 0,
            start_time_ns: 0,
            start_instret: 0,
            window_faults_base: 0,
            window_evictions_base: 0,
        }
    }

    fn reset(&mut self) {
        self.page_faults = 0;
        self.evictions = 0;
        self.write_backs = 0;
        self.thrash_events = 0;
        self.start_time_ns = read_time_ns();
        self.start_instret = read_instret();
        self.window_faults_base = 0;
        self.window_evictions_base = 0;
    }
}

pub(crate) struct Pager {
    regions: Vec<VmRegion>,
    pages: BTreeMap<usize, PageState>,
    resident: Vec<ResidentPage>,
    free_frames: Vec<PPN<Sv39>>,
    swap_slots: Vec<Option<Box<[u8]>>>,
    quota: usize,
    algo: VmAlgo,
    next_loaded_at: u64,
    epoch: u64,
    clock_hand: usize,
    stats: RuntimeStats,
}

impl Pager {
    pub(crate) fn new() -> Self {
        let mut stats = RuntimeStats::new();
        stats.reset();
        Self {
            regions: Vec::new(),
            pages: BTreeMap::new(),
            resident: Vec::new(),
            free_frames: Vec::new(),
            swap_slots: Vec::new(),
            quota: DEFAULT_QUOTA,
            algo: VmAlgo::Fifo,
            next_loaded_at: 0,
            epoch: 0,
            clock_hand: 0,
            stats,
        }
    }

    pub(crate) fn can_register(
        &self,
        space: &AddressSpace<Sv39, Sv39Manager>,
        range: Range<VPN<Sv39>>,
    ) -> bool {
        let valid = build_flags("V");
        let mut vpn = range.start;
        while vpn < range.end {
            if self.pages.contains_key(&vpn.val())
                || space.translate::<u8>(vpn.base(), valid).is_some()
            {
                return false;
            }
            vpn = vpn + 1;
        }
        true
    }

    pub(crate) fn register_range(&mut self, range: Range<VPN<Sv39>>, flags: VmFlags<Sv39>) {
        self.regions.push(VmRegion {
            range: range.start..range.end,
        });
        let mut vpn = range.start;
        while vpn < range.end {
            self.pages.insert(
                vpn.val(),
                PageState {
                    flags,
                    backing: PageBacking::ZeroFill,
                    resident: false,
                },
            );
            vpn = vpn + 1;
        }
    }

    pub(crate) fn remove_range(
        &mut self,
        space: &mut AddressSpace<Sv39, Sv39Manager>,
        range: Range<VPN<Sv39>>,
    ) -> bool {
        let mut vpn = range.start;
        while vpn < range.end {
            if !self.pages.contains_key(&vpn.val()) {
                return false;
            }
            vpn = vpn + 1;
        }

        let mut vpn = range.start;
        while vpn < range.end {
            if let Some(index) = self.resident.iter().position(|page| page.vpn == vpn) {
                let victim = self.remove_resident(index);
                space.unmap_page(vpn);
                self.free_frames.push(victim.ppn);
            }
            if let Some(PageState { backing, .. }) = self.pages.remove(&vpn.val()) {
                if let PageBacking::Swap(slot) = backing {
                    self.release_slot(slot);
                }
            }
            vpn = vpn + 1;
        }

        let mut new_regions = Vec::new();
        for area in self.regions.drain(..) {
            if area.range.end <= range.start || area.range.start >= range.end {
                new_regions.push(area);
            } else {
                if area.range.start < range.start {
                    new_regions.push(VmRegion {
                        range: area.range.start..range.start,
                    });
                }
                if area.range.end > range.end {
                    new_regions.push(VmRegion {
                        range: range.end..area.range.end,
                    });
                }
            }
        }
        self.regions = new_regions;
        true
    }

    pub(crate) fn handle_page_fault(
        &mut self,
        space: &mut AddressSpace<Sv39, Sv39Manager>,
        kind: FaultKind,
        addr: usize,
    ) -> bool {
        #[cfg(feature = "exercise")]
        {
            let _ = (space, kind, addr);
            todo!(
                "练习题：完成 pager 的缺页主路径，包括合法性检查、分配/置换、换入和 A/D 位初始化"
            );
        }
        #[cfg(not(feature = "exercise"))]
        {
        let vpn = VAddr::<Sv39>::new(addr).floor();
        let Some(state) = self.pages.get(&vpn.val()) else {
            return false;
        };
        if !can_access(state.flags, kind) {
            return false;
        }
        let flags = state.flags;
        let backing = state.backing;
        let resident = state.resident;

        if resident {
            let mut set = FLAG_A;
            if matches!(kind, FaultKind::Store) {
                set |= FLAG_D;
            }
            let _ = space.update_pte_flags(vpn, set, VmFlags::ZERO);
            return true;
        }

        self.stats.page_faults += 1;
        self.epoch += 1;

        let ppn = if let Some(ppn) = self.free_frames.pop() {
            ppn
        } else if self.resident.len() < self.quota {
            allocate_frame()
        } else {
            match self.evict_one(space) {
                Some(ppn) => ppn,
                None => return false,
            }
        };

        match backing {
            PageBacking::ZeroFill => frame_bytes_mut(ppn).fill(0),
            PageBacking::Swap(slot) => {
                if let Some(bytes) = self.swap_slots.get(slot).and_then(|slot| slot.as_ref()) {
                    frame_bytes_mut(ppn).copy_from_slice(bytes);
                } else {
                    frame_bytes_mut(ppn).fill(0);
                }
            }
        }

        space.map_page_extern(vpn, ppn, flags);
        if let Some(state) = self.pages.get_mut(&vpn.val()) {
            state.resident = true;
        }
        self.next_loaded_at += 1;
        self.resident.push(ResidentPage {
            vpn,
            ppn,
            loaded_at: self.next_loaded_at,
            last_epoch: self.epoch,
            age: u8::MAX,
        });
        self.check_thrashing();
        true
        }
    }

    pub(crate) fn set_algo(&mut self, algo_raw: usize) -> bool {
        let Some(algo) = VmAlgo::from_raw(algo_raw) else {
            return false;
        };
        self.algo = algo;
        true
    }

    pub(crate) fn set_quota(
        &mut self,
        space: &mut AddressSpace<Sv39, Sv39Manager>,
        quota: usize,
    ) -> bool {
        if quota == 0 || quota > MAX_QUOTA {
            return false;
        }
        self.quota = quota;
        while self.resident.len() > self.quota {
            if self.evict_one(space).is_none() {
                return false;
            }
        }
        true
    }

    pub(crate) fn reset_stats(&mut self) {
        self.stats.reset();
    }

    pub(crate) fn snapshot_stats(&self) -> VmStats {
        let swap_pages = self
            .pages
            .values()
            .filter(|state| !state.resident && matches!(state.backing, PageBacking::Swap(_)))
            .count();
        VmStats {
            algo: self.algo as u32,
            quota: self.quota as u32,
            resident_pages: self.resident.len() as u32,
            swap_pages: swap_pages as u32,
            page_faults: self.stats.page_faults,
            evictions: self.stats.evictions,
            write_backs: self.stats.write_backs,
            thrash_events: self.stats.thrash_events,
            elapsed_ns: read_time_ns().saturating_sub(self.stats.start_time_ns),
            instret_delta: read_instret().saturating_sub(self.stats.start_instret),
        }
    }

    fn evict_one(&mut self, space: &mut AddressSpace<Sv39, Sv39Manager>) -> Option<PPN<Sv39>> {
        let index = match self.algo {
            VmAlgo::Fifo => self.pick_fifo_index()?,
            VmAlgo::Clock => self.pick_clock_index(space)?,
            VmAlgo::LruApprox => self.pick_lru_index(space)?,
            VmAlgo::WorkingSet => self.pick_working_set_index(space)?,
        };
        let victim = self.remove_resident(index);
        let current_backing = self.pages.get(&victim.vpn.val())?.backing;
        let pte = space.pte_of(victim.vpn)?;
        let dirty = pte.flags().contains(FLAG_D);
        let mut new_backing = current_backing;
        if dirty {
            let slot = match current_backing {
                PageBacking::ZeroFill => {
                    let slot = self.allocate_slot();
                    new_backing = PageBacking::Swap(slot);
                    slot
                }
                PageBacking::Swap(slot) => slot,
            };
            if let Some(bytes) = self.swap_slots.get_mut(slot).and_then(|slot| slot.as_mut()) {
                bytes.copy_from_slice(frame_bytes(victim.ppn));
            }
            self.stats.write_backs += 1;
        }
        if let Some(state) = self.pages.get_mut(&victim.vpn.val()) {
            state.backing = new_backing;
            state.resident = false;
        }
        let _ = space.update_pte_flags(victim.vpn, VmFlags::ZERO, FLAG_A | FLAG_D);
        space.unmap_page(victim.vpn);
        self.stats.evictions += 1;
        Some(victim.ppn)
    }

    fn pick_fifo_index(&self) -> Option<usize> {
        #[cfg(feature = "exercise")]
        {
            todo!("练习题：实现 FIFO victim 选择，返回 resident 集合中最早装入页面的下标");
        }
        #[cfg(not(feature = "exercise"))]
        {
        self.resident
            .iter()
            .enumerate()
            .min_by_key(|(_, page)| page.loaded_at)
            .map(|(index, _)| index)
        }
    }

    fn pick_clock_index(&mut self, space: &AddressSpace<Sv39, Sv39Manager>) -> Option<usize> {
        #[cfg(feature = "exercise")]
        {
            let _ = space;
            todo!(
                "练习题：实现 Clock(second chance) victim 选择，按 A 位清零并推进 clock hand"
            );
        }
        #[cfg(not(feature = "exercise"))]
        {
        if self.resident.is_empty() {
            return None;
        }
        let len = self.resident.len();
        for _ in 0..(len * 2) {
            let index = self.clock_hand % len;
            let vpn = self.resident[index].vpn;
            let accessed = space
                .pte_of(vpn)
                .map(|pte: tg_kernel_vm::page_table::Pte<Sv39>| pte.flags().contains(FLAG_A))
                .unwrap_or(false);
            if accessed {
                let _ = space.update_pte_flags(vpn, VmFlags::ZERO, FLAG_A);
                self.clock_hand = (index + 1) % len;
            } else {
                self.clock_hand = (index + 1) % len;
                return Some(index);
            }
        }
        Some(self.clock_hand % len)
        }
    }

    fn pick_lru_index(&mut self, space: &AddressSpace<Sv39, Sv39Manager>) -> Option<usize> {
        for page in &mut self.resident {
            let accessed = space
                .pte_of(page.vpn)
                .map(|pte: tg_kernel_vm::page_table::Pte<Sv39>| pte.flags().contains(FLAG_A))
                .unwrap_or(false);
            page.age >>= 1;
            if accessed {
                page.age |= 0x80;
                let _ = space.update_pte_flags(page.vpn, VmFlags::ZERO, FLAG_A);
            }
        }
        self.resident
            .iter()
            .enumerate()
            .min_by_key(|(_, page)| page.age)
            .map(|(index, _)| index)
    }

    fn pick_working_set_index(
        &mut self,
        space: &AddressSpace<Sv39, Sv39Manager>,
    ) -> Option<usize> {
        let mut victim = None;
        for (index, page) in self.resident.iter_mut().enumerate() {
            let accessed = space
                .pte_of(page.vpn)
                .map(|pte: tg_kernel_vm::page_table::Pte<Sv39>| pte.flags().contains(FLAG_A))
                .unwrap_or(false);
            if accessed {
                page.last_epoch = self.epoch;
                let _ = space.update_pte_flags(page.vpn, VmFlags::ZERO, FLAG_A);
            } else if self.epoch.saturating_sub(page.last_epoch) > WORKING_SET_TAU && victim.is_none()
            {
                victim = Some(index);
            }
        }
        victim.or_else(|| self.pick_clock_index(space))
    }

    fn remove_resident(&mut self, index: usize) -> ResidentPage {
        let last = self.resident.len() - 1;
        let victim = self.resident.swap_remove(index);
        if self.clock_hand > last.saturating_sub(1) {
            self.clock_hand = 0;
        } else if index < self.clock_hand && self.clock_hand > 0 {
            self.clock_hand -= 1;
        }
        victim
    }

    fn allocate_slot(&mut self) -> usize {
        if let Some((index, slot)) = self
            .swap_slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        {
            *slot = Some(vec![0u8; PAGE_SIZE].into_boxed_slice());
            index
        } else {
            self.swap_slots
                .push(Some(vec![0u8; PAGE_SIZE].into_boxed_slice()));
            self.swap_slots.len() - 1
        }
    }

    fn release_slot(&mut self, slot: usize) {
        if let Some(entry) = self.swap_slots.get_mut(slot) {
            *entry = None;
        }
    }

    fn check_thrashing(&mut self) {
        let faults = self
            .stats
            .page_faults
            .saturating_sub(self.stats.window_faults_base);
        let evictions = self
            .stats
            .evictions
            .saturating_sub(self.stats.window_evictions_base);
        if faults >= self.quota as u64 && evictions.saturating_mul(2) >= faults {
            self.stats.thrash_events += 1;
            let boosted = matches!(self.algo, VmAlgo::WorkingSet) && self.quota < MAX_QUOTA;
            if boosted {
                self.quota = (self.quota + THRASH_STEP).min(MAX_QUOTA);
            }
            if self.stats.thrash_events <= 4 {
                log::warn!(
                    "thrashing detected: algo={:?} quota={} faults={} evictions={}{}",
                    self.algo,
                    self.quota,
                    faults,
                    evictions,
                    if boosted { " boosted" } else { "" },
                );
            }
            self.stats.window_faults_base = self.stats.page_faults;
            self.stats.window_evictions_base = self.stats.evictions;
        }
    }
}

fn can_access(flags: VmFlags<Sv39>, kind: FaultKind) -> bool {
    match kind {
        FaultKind::Load => flags.contains(build_flags("R")),
        FaultKind::Store => flags.contains(build_flags("W")),
        FaultKind::Instruction => flags.contains(build_flags("X")),
    }
}

fn allocate_frame() -> PPN<Sv39> {
    let ptr = unsafe {
        alloc_zeroed(Layout::from_size_align_unchecked(PAGE_SIZE, PAGE_SIZE))
    };
    assert!(!ptr.is_null());
    PPN::new((ptr as usize) >> Sv39::PAGE_BITS)
}

fn frame_bytes(ppn: PPN<Sv39>) -> &'static [u8] {
    unsafe { core::slice::from_raw_parts(frame_ptr(ppn), PAGE_SIZE) }
}

fn frame_bytes_mut(ppn: PPN<Sv39>) -> &'static mut [u8] {
    unsafe { core::slice::from_raw_parts_mut(frame_ptr(ppn), PAGE_SIZE) }
}

fn frame_ptr(ppn: PPN<Sv39>) -> *mut u8 {
    (ppn.val() << Sv39::PAGE_BITS) as *mut u8
}

fn read_time_ns() -> u64 {
    #[cfg(target_arch = "riscv64")]
    {
        (riscv::register::time::read() as u64) * 10000 / 125
    }
    #[cfg(not(target_arch = "riscv64"))]
    {
        0
    }
}

fn read_instret() -> u64 {
    #[cfg(target_arch = "riscv64")]
    {
        riscv::register::instret::read() as u64
    }
    #[cfg(not(target_arch = "riscv64"))]
    {
        0
    }
}
