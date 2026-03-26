#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::mmap;

const START: usize = 0x2200_0000;

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    println!("vm_prot begin");
    assert_eq!(0, mmap(START, 0x1000, 0b001));
    unsafe {
        *(START as *mut u8) = 1;
    }
    println!("vm_prot unexpected");
    -1
}
