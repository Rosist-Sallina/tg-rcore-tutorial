//! VirtIO GPU 驱动模块。
//!
//! 提供最小接口：
//! - `init_gpu()`：初始化设备并绑定 framebuffer
//! - `with_framebuffer_mut()`：访问 DMA framebuffer
//! - `flush()`：将 framebuffer 刷新到屏幕

use crate::{build_flags, Sv39, KERNEL_SPACE};
use alloc::{
    alloc::{alloc_zeroed, dealloc},
    boxed::Box,
};
use core::{alloc::Layout, ptr::NonNull};
use spin::{Lazy, Mutex};
use tg_kernel_vm::page_table::{MmuMeta, VAddr, VmFlags};
use virtio_drivers::{Hal, MmioTransport, VirtIOGpu, VirtIOHeader};

/// VirtIO GPU 的 MMIO 基地址（bus.1）
const GPU_MMIO_BASE: usize = 0x1000_2000;

struct GpuState {
    driver: Box<VirtIOGpu<'static, VirtioHal, MmioTransport>>,
    framebuffer: *mut u8,
    framebuffer_len: usize,
    width: u32,
    height: u32,
}

// Safety: 所有访问都通过全局 Mutex 串行化。
unsafe impl Send for GpuState {}

static GPU_DEVICE: Lazy<Mutex<Option<GpuState>>> = Lazy::new(|| Mutex::new(None));

/// 初始化 VirtIO GPU。
pub(crate) fn init_gpu() -> Result<(), &'static str> {
    let mut guard = GPU_DEVICE.lock();
    if guard.is_some() {
        return Ok(());
    }

    let transport = unsafe {
        MmioTransport::new(NonNull::new(GPU_MMIO_BASE as *mut VirtIOHeader).unwrap())
            .map_err(|_| "failed to create GPU MMIO transport")?
    };
    let mut driver =
        Box::new(VirtIOGpu::new(transport).map_err(|_| "failed to create VirtIOGpu")?);
    let (width, height) = driver
        .resolution()
        .map_err(|_| "failed to query GPU resolution")?;
    let (framebuffer_ptr, framebuffer_len) = {
        let framebuffer = driver
            .setup_framebuffer()
            .map_err(|_| "failed to setup GPU framebuffer")?;
        (framebuffer.as_mut_ptr(), framebuffer.len())
    };

    *guard = Some(GpuState {
        driver,
        framebuffer: framebuffer_ptr,
        framebuffer_len,
        width,
        height,
    });
    Ok(())
}

/// 访问 framebuffer 数据。
pub(crate) fn with_framebuffer_mut<R>(
    f: impl FnOnce(&mut [u8], u32, u32) -> R,
) -> Result<R, &'static str> {
    let mut guard = GPU_DEVICE.lock();
    let state = guard.as_mut().ok_or("gpu not initialized")?;
    let fb =
        unsafe { core::slice::from_raw_parts_mut(state.framebuffer, state.framebuffer_len) };
    Ok(f(fb, state.width, state.height))
}

/// 获取 framebuffer 的内核虚拟地址和长度（字节）。
pub(crate) fn framebuffer_addr_len() -> Result<(usize, usize), &'static str> {
    let guard = GPU_DEVICE.lock();
    let state = guard.as_ref().ok_or("gpu not initialized")?;
    Ok((state.framebuffer as usize, state.framebuffer_len))
}

/// 刷新 framebuffer 到显示设备。
pub(crate) fn flush() -> Result<(), &'static str> {
    let mut guard = GPU_DEVICE.lock();
    let state = guard.as_mut().ok_or("gpu not initialized")?;
    state
        .driver
        .flush()
        .map_err(|_| "failed to flush GPU framebuffer")
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
