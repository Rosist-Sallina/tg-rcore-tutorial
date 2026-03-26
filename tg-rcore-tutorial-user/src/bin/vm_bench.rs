#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{
    VmAlgo, VmStats, mmap, munmap, vm_get_stats, vm_reset_stats, vm_set_algo, vm_set_quota,
};

const PAGE_SIZE: usize = 0x1000;
const PAGES: usize = 64;
const QUOTA: usize = 16;
const BASE: usize = 0x3000_0000;
const STRIDE: usize = 0x0010_0000;
const HOT: usize = QUOTA / 2;

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    println!("algo pattern faults faults/s faults/kinst evictions write_backs thrash_events");
    let algos = [
        VmAlgo::Fifo,
        VmAlgo::Clock,
        VmAlgo::LruApprox,
        VmAlgo::WorkingSet,
    ];
    let patterns = [
        Pattern::SeqScan,
        Pattern::RandUniform,
        Pattern::LocalityStrong,
        Pattern::LocalityWeak,
    ];
    let mut run = 0usize;
    for algo in algos {
        for pattern in patterns {
            let start = BASE + run * STRIDE;
            assert_eq!(0, mmap(start, PAGE_SIZE * PAGES, 0b11));
            assert_eq!(0, vm_set_algo(algo));
            assert_eq!(0, vm_set_quota(QUOTA));
            assert_eq!(0, vm_reset_stats());
            run_pattern(pattern, start);
            let mut stats = VmStats::ZERO;
            assert_eq!(0, vm_get_stats(&mut stats));
            print_stats(algo, pattern, &stats);
            assert_eq!(0, munmap(start, PAGE_SIZE * PAGES));
            run += 1;
        }
    }
    println!("VMBENCH PASS");
    0
}

#[derive(Clone, Copy)]
enum Pattern {
    SeqScan,
    RandUniform,
    LocalityStrong,
    LocalityWeak,
}

fn run_pattern(pattern: Pattern, start: usize) {
    match pattern {
        Pattern::SeqScan => {
            for round in 0..32 {
                for page in 0..PAGES {
                    touch(start, page, (round + page) & 0x3 == 0);
                }
            }
        }
        Pattern::RandUniform => {
            let mut state = 1u32;
            for step in 0..32768 {
                let page = lcg(&mut state) as usize % PAGES;
                touch(start, page, step & 0x3 == 0);
            }
        }
        Pattern::LocalityStrong => {
            let mut state = 7u32;
            for step in 0..32768 {
                let raw = lcg(&mut state) as usize;
                let page = if raw % 10 < 8 {
                    raw % HOT
                } else {
                    HOT + raw % (PAGES - HOT)
                };
                touch(start, page, step & 0x3 == 0);
            }
        }
        Pattern::LocalityWeak => {
            let mut state = 11u32;
            for step in 0..32768 {
                let raw = lcg(&mut state) as usize;
                let window = QUOTA * 2;
                let base = (step / 64) % (PAGES - window);
                let page = if raw % 10 < 6 {
                    base + raw % window
                } else {
                    raw % PAGES
                };
                touch(start, page, step & 0x3 == 0);
            }
        }
    }
}

fn touch(start: usize, page: usize, write: bool) {
    let ptr = (start + page * PAGE_SIZE) as *mut u8;
    unsafe {
        let value = ptr.read_volatile();
        if write {
            ptr.write_volatile(value.wrapping_add(page as u8).wrapping_add(1));
        }
    }
}

fn lcg(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1103515245).wrapping_add(12345);
    *state
}

fn print_stats(algo: VmAlgo, pattern: Pattern, stats: &VmStats) {
    let faults_per_sec = if stats.elapsed_ns == 0 {
        0
    } else {
        stats.page_faults.saturating_mul(1_000_000_000) / stats.elapsed_ns
    };
    let faults_per_kinst = if stats.instret_delta == 0 {
        0
    } else {
        stats.page_faults.saturating_mul(1000) / stats.instret_delta
    };
    println!(
        "{} {} {} {} {} {} {} {}",
        algo_name(algo),
        pattern_name(pattern),
        stats.page_faults,
        faults_per_sec,
        faults_per_kinst,
        stats.evictions,
        stats.write_backs,
        stats.thrash_events,
    );
}

fn algo_name(algo: VmAlgo) -> &'static str {
    match algo {
        VmAlgo::Fifo => "fifo",
        VmAlgo::Clock => "clock",
        VmAlgo::LruApprox => "lru",
        VmAlgo::WorkingSet => "ws",
    }
}

fn pattern_name(pattern: Pattern) -> &'static str {
    match pattern {
        Pattern::SeqScan => "seq_scan",
        Pattern::RandUniform => "rand_uniform",
        Pattern::LocalityStrong => "locality_strong",
        Pattern::LocalityWeak => "locality_weak",
    }
}
