//! VirtIO 输入设备驱动（键盘）。
//!
//! 对外提供：
//! - `init_input()`：初始化 VirtIO-Input 设备
//! - `pop_event()`：非阻塞读取一个输入事件

use crate::{build_flags, Sv39, KERNEL_SPACE};
use alloc::alloc::{alloc_zeroed, dealloc};
use core::{alloc::Layout, ptr::NonNull};
use spin::{Lazy, Mutex};
use tg_kernel_vm::page_table::{MmuMeta, VAddr, VmFlags};
use virtio_drivers::{Hal, MmioTransport, VirtIOHeader, VirtIOInput};

/// VirtIO Input 的 MMIO 基地址（bus.2）
const INPUT_MMIO_BASE: usize = 0x1000_3000;

/// 用户可见的输入事件结构（Linux evdev 风格）。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InputEvent {
    pub(crate) event_type: u16,
    pub(crate) code: u16,
    pub(crate) value: u32,
}

struct InputState {
    driver: VirtIOInput<VirtioHal, MmioTransport>,
}

// Safety: 设备访问通过全局 Mutex 串行化。
unsafe impl Send for InputState {}

static INPUT_DEVICE: Lazy<Mutex<Option<InputState>>> = Lazy::new(|| Mutex::new(None));

/// 初始化 VirtIO 输入设备。
pub(crate) fn init_input() -> Result<(), &'static str> {
    let mut guard = INPUT_DEVICE.lock();
    if guard.is_some() {
        return Ok(());
    }

    let transport = unsafe {
        MmioTransport::new(NonNull::new(INPUT_MMIO_BASE as *mut VirtIOHeader).unwrap())
            .map_err(|_| "failed to create input MMIO transport")?
    };
    let driver = VirtIOInput::new(transport).map_err(|_| "failed to create VirtIOInput")?;
    *guard = Some(InputState { driver });
    Ok(())
}

/// 非阻塞获取一个输入事件。
pub(crate) fn pop_event() -> Option<InputEvent> {
    let mut guard = INPUT_DEVICE.lock();
    let state = guard.as_mut()?;
    let _ = state.driver.ack_interrupt();
    state.driver.pop_pending_event().map(|event| InputEvent {
        event_type: event.event_type,
        code: event.code,
        value: event.value,
    })
}

/// VirtIO HAL（硬件抽象层）实现。
struct VirtioHal;

impl Hal for VirtioHal {
    /// DMA 内存分配。
    fn dma_alloc(pages: usize) -> usize {
        unsafe {
            alloc_zeroed(Layout::from_size_align_unchecked(
                pages << Sv39::PAGE_BITS,
                1 << Sv39::PAGE_BITS,
            )) as _
        }
    }

    /// DMA 内存释放。
    fn dma_dealloc(paddr: usize, pages: usize) -> i32 {
        unsafe {
            dealloc(
                paddr as _,
                Layout::from_size_align_unchecked(pages << Sv39::PAGE_BITS, 1 << Sv39::PAGE_BITS),
            )
        }
        0
    }

    /// 物理地址转虚拟地址（恒等映射）。
    fn phys_to_virt(paddr: usize) -> usize {
        paddr
    }

    /// 虚拟地址转物理地址。
    fn virt_to_phys(vaddr: usize) -> usize {
        const VALID: VmFlags<Sv39> = build_flags("__V");
        let ptr: NonNull<u8> = unsafe {
            KERNEL_SPACE
                .assume_init_ref()
                .translate(VAddr::new(vaddr), VALID)
                .unwrap()
        };
        ptr.as_ptr() as usize
    }
}
