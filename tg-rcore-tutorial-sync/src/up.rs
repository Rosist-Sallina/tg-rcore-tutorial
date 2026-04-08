//! 单核/多核统一的内部可变性容器。

#[cfg(not(feature = "smp"))]
mod inner {
    use core::cell::{RefCell, RefMut, UnsafeCell};
    use core::ops::{Deref, DerefMut};
    use riscv::register::sstatus;
    use spin::Lazy;

    /// 单核环境下的裸可变单元。
    pub struct UPSafeCellRaw<T> {
        inner: UnsafeCell<T>,
    }

    unsafe impl<T> Sync for UPSafeCellRaw<T> {}

    impl<T> UPSafeCellRaw<T> {
        /// 创建单元。
        pub unsafe fn new(value: T) -> Self {
            Self {
                inner: UnsafeCell::new(value),
            }
        }

        /// 取出可变引用。
        pub fn get_mut(&self) -> &mut T {
            unsafe { &mut *self.inner.get() }
        }
    }

    /// 中断屏蔽嵌套信息。
    pub struct IntrMaskingInfo {
        nested_level: usize,
        sie_before_masking: bool,
    }

    /// 全局中断屏蔽状态。
    pub static INTR_MASKING_INFO: Lazy<UPSafeCellRaw<IntrMaskingInfo>> =
        Lazy::new(|| unsafe { UPSafeCellRaw::new(IntrMaskingInfo::new()) });

    impl IntrMaskingInfo {
        /// 创建状态。
        pub const fn new() -> Self {
            Self {
                nested_level: 0,
                sie_before_masking: false,
            }
        }

        fn enter(&mut self) {
            let sie = sstatus::read().sie();
            unsafe {
                sstatus::clear_sie();
            }
            if self.nested_level == 0 {
                self.sie_before_masking = sie;
            }
            self.nested_level += 1;
        }

        fn exit(&mut self) {
            self.nested_level -= 1;
            if self.nested_level == 0 && self.sie_before_masking {
                unsafe {
                    sstatus::set_sie();
                }
            }
        }
    }

    /// 单核临界区容器。
    pub struct UPIntrFreeCell<T> {
        inner: RefCell<T>,
    }

    unsafe impl<T> Sync for UPIntrFreeCell<T> {}

    /// 单核守卫。
    pub struct UPIntrRefMut<'a, T>(Option<RefMut<'a, T>>);

    impl<T> UPIntrFreeCell<T> {
        /// 创建容器。
        pub unsafe fn new(value: T) -> Self {
            Self {
                inner: RefCell::new(value),
            }
        }

        /// 独占访问。
        pub fn exclusive_access(&self) -> UPIntrRefMut<'_, T> {
            INTR_MASKING_INFO.get_mut().enter();
            UPIntrRefMut(Some(self.inner.borrow_mut()))
        }

        /// 闭包形式的独占访问。
        pub fn exclusive_session<F, V>(&self, f: F) -> V
        where
            F: FnOnce(&mut T) -> V,
        {
            let mut inner = self.exclusive_access();
            f(inner.deref_mut())
        }
    }

    impl<T> Drop for UPIntrRefMut<'_, T> {
        fn drop(&mut self) {
            self.0 = None;
            INTR_MASKING_INFO.get_mut().exit();
        }
    }

    impl<T> Deref for UPIntrRefMut<'_, T> {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            self.0.as_ref().unwrap().deref()
        }
    }

    impl<T> DerefMut for UPIntrRefMut<'_, T> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            self.0.as_mut().unwrap().deref_mut()
        }
    }
}

#[cfg(feature = "smp")]
mod inner {
    use core::ops::{Deref, DerefMut};
    use tg_smp::spin::{SpinNoIrq, SpinNoIrqGuard};

    /// 多核下改为关中断自旋锁。
    pub struct UPIntrFreeCell<T> {
        inner: SpinNoIrq<T>,
    }

    unsafe impl<T: Send> Sync for UPIntrFreeCell<T> {}

    /// 多核守卫。
    pub struct UPIntrRefMut<'a, T>(SpinNoIrqGuard<'a, T>);

    impl<T> UPIntrFreeCell<T> {
        /// 创建容器。
        pub unsafe fn new(value: T) -> Self {
            Self {
                inner: SpinNoIrq::new(value),
            }
        }

        /// 独占访问。
        pub fn exclusive_access(&self) -> UPIntrRefMut<'_, T> {
            UPIntrRefMut(self.inner.lock())
        }

        /// 闭包形式的独占访问。
        pub fn exclusive_session<F, V>(&self, f: F) -> V
        where
            F: FnOnce(&mut T) -> V,
        {
            let mut inner = self.exclusive_access();
            f(inner.deref_mut())
        }
    }

    impl<T> Deref for UPIntrRefMut<'_, T> {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl<T> DerefMut for UPIntrRefMut<'_, T> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }
}

pub use inner::{UPIntrFreeCell, UPIntrRefMut};
