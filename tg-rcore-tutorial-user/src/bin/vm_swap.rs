#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{VmAlgo, mmap, munmap, vm_set_algo, vm_set_quota};

const START: usize = 0x2100_0000;
const PAGE_SIZE: usize = 0x1000;
const PAGES: usize = 32;

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    assert_eq!(0, vm_set_algo(VmAlgo::Fifo));
    assert_eq!(0, vm_set_quota(4));
    assert_eq!(0, mmap(START, PAGE_SIZE * PAGES, 0b11));

    for page in 0..PAGES {
        unsafe {
            *((START + page * PAGE_SIZE) as *mut u8) = (page as u8).wrapping_mul(3);
        }
    }
    for page in 0..PAGES {
        let value = unsafe { *((START + page * PAGE_SIZE) as *mut u8) };
        assert_eq!(value, (page as u8).wrapping_mul(3));
    }
    assert_eq!(0, munmap(START, PAGE_SIZE * PAGES));
    println!("vm_swap OK");
    0
}
