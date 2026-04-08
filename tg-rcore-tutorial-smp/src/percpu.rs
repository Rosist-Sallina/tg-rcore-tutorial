//! Per-CPU 数据结构。

use crate::hart::hart_id;
use core::cell::UnsafeCell;

/// 每核一份的数据槽。
pub struct PerCpu<T> {
    data: UnsafeCell<[T; crate::MAX_CPUS]>,
}

unsafe impl<T: Send> Sync for PerCpu<T> {}

impl<T: Copy> PerCpu<T> {
    /// 创建新的 Per-CPU 数组。
    pub const fn new(init: [T; crate::MAX_CPUS]) -> Self {
        Self {
            data: UnsafeCell::new(init),
        }
    }

    /// 读取当前 hart 槽位。
    #[inline(always)]
    pub fn get(&self) -> T {
        unsafe { (*self.data.get())[hart_id()] }
    }

    /// 写入当前 hart 槽位。
    #[inline(always)]
    pub fn set(&self, value: T) {
        unsafe {
            (*self.data.get())[hart_id()] = value;
        }
    }
}

impl<T> PerCpu<T> {
    /// 在当前 hart 上获取可变引用。
    #[inline(always)]
    pub unsafe fn get_mut(&self) -> &mut T {
        unsafe { &mut (*self.data.get())[hart_id()] }
    }

    /// 在指定 hart 上获取可变引用。
    #[inline(always)]
    pub unsafe fn get_remote_mut(&self, cpu_id: usize) -> &mut T {
        unsafe { &mut (*self.data.get())[cpu_id] }
    }
}
