//! 自旋锁与关中断自旋锁。

use core::cell::UnsafeCell;
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// 最小自旋锁。
pub struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for SpinLock<T> {}
unsafe impl<T: Send> Send for SpinLock<T> {}

/// 自旋锁守卫。
pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> SpinLock<T> {
    /// 创建锁。
    pub const fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    /// 获取锁。
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        SpinLockGuard { lock: self }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.data.get() }
    }
}

/// 关中断自旋锁。
pub struct SpinNoIrq<T> {
    inner: SpinLock<T>,
}

unsafe impl<T: Send> Sync for SpinNoIrq<T> {}
unsafe impl<T: Send> Send for SpinNoIrq<T> {}

/// 关中断自旋锁守卫。
pub struct SpinNoIrqGuard<'a, T> {
    guard: ManuallyDrop<SpinLockGuard<'a, T>>,
    sie_before: bool,
}

impl<T> SpinNoIrq<T> {
    /// 创建锁。
    pub const fn new(data: T) -> Self {
        Self {
            inner: SpinLock::new(data),
        }
    }

    /// 关中断后加锁。
    pub fn lock(&self) -> SpinNoIrqGuard<'_, T> {
        let sie_before = riscv::register::sstatus::read().sie();
        unsafe {
            riscv::register::sstatus::clear_sie();
        }
        SpinNoIrqGuard {
            guard: ManuallyDrop::new(self.inner.lock()),
            sie_before,
        }
    }
}

impl<T> Drop for SpinNoIrqGuard<'_, T> {
    fn drop(&mut self) {
        unsafe {
            ManuallyDrop::drop(&mut self.guard);
        }
        if self.sie_before {
            unsafe {
                riscv::register::sstatus::set_sie();
            }
        }
    }
}

impl<T> Deref for SpinNoIrqGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<T> DerefMut for SpinNoIrqGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}
