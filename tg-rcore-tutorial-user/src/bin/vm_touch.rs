#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use core::slice::from_raw_parts_mut;
use user_lib::{mmap, munmap};

const START: usize = 0x2000_0000;
const PAGE_SIZE: usize = 0x1000;

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    assert_eq!(0, mmap(START, PAGE_SIZE * 4, 0b11));
    let page = unsafe { &mut *from_raw_parts_mut(START as *mut u8, PAGE_SIZE * 4) };
    for i in 0..page.len() {
        page[i] = (i & 0xff) as u8;
    }
    for i in 0..page.len() {
        assert_eq!(page[i], (i & 0xff) as u8);
    }
    assert_eq!(0, munmap(START, PAGE_SIZE * 4));
    println!("vm_touch OK");
    0
}
