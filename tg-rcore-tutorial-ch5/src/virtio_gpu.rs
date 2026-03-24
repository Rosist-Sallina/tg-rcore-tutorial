use alloc::boxed::Box;
use core::{
    cell::UnsafeCell,
    ptr::{null_mut, NonNull},
};
use tg_syscall::FrameBufferInfo;
use virtio_drivers::{Hal, MmioTransport, VirtIOGpu, VirtIOHeader};

const GPU_MMIO_BASE: usize = 0x1000_1000;
const DMA_POOL_SIZE: usize = 4 << 20;

struct GpuCell {
    inner: UnsafeCell<*mut GpuDevice>,
}

unsafe impl Sync for GpuCell {}

impl GpuCell {
    const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(null_mut()),
        }
    }

    fn get(&self) -> *mut *mut GpuDevice {
        self.inner.get()
    }
}

static GPU: GpuCell = GpuCell::new();

struct GpuDevice {
    driver: Box<VirtIOGpu<'static, VirtioHal, MmioTransport>>,
    frame_buffer: FrameBuffer,
}

struct FrameBuffer {
    ptr: *mut u32,
    len: usize,
    width: usize,
    height: usize,
}

unsafe impl Send for GpuDevice {}

#[repr(C, align(4096))]
struct DmaPool {
    inner: UnsafeCell<[u8; DMA_POOL_SIZE]>,
}

unsafe impl Sync for DmaPool {}

static DMA_POOL: DmaPool = DmaPool {
    inner: UnsafeCell::new([0; DMA_POOL_SIZE]),
};
static DMA_NEXT: GpuOffsetCell = GpuOffsetCell::new();

struct GpuOffsetCell {
    inner: UnsafeCell<usize>,
}

unsafe impl Sync for GpuOffsetCell {}

impl GpuOffsetCell {
    const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(0),
        }
    }

    fn get(&self) -> *mut usize {
        self.inner.get()
    }
}

pub fn init() {
    let transport = unsafe {
        MmioTransport::new(NonNull::new(GPU_MMIO_BASE as *mut VirtIOHeader).unwrap())
            .expect("failed to create GPU MMIO transport")
    };
    let mut driver = VirtIOGpu::new_boxed(transport).expect("failed to create VirtIO GPU driver");
    let (width, height) = driver.resolution().expect("failed to read GPU resolution");
    let (ptr, len) = {
        let frame_buffer = driver
            .setup_framebuffer()
            .expect("failed to setup GPU framebuffer");
        (frame_buffer.as_mut_ptr() as *mut u32, frame_buffer.len() / 4)
    };

    let device = Box::new(GpuDevice {
        driver,
        frame_buffer: FrameBuffer {
            ptr,
            len,
            width: width as usize,
            height: height as usize,
        },
    });
    unsafe { *GPU.get() = Box::into_raw(device) };
}

pub fn info() -> FrameBufferInfo {
    let gpu = unsafe { *GPU.get() };
    let fb = &unsafe { gpu.as_ref() }.expect("gpu not initialized").frame_buffer;
    FrameBufferInfo {
        width: fb.width as u32,
        height: fb.height as u32,
    }
}

pub fn clear(color: u32) {
    let info = info();
    fill_rect(0, 0, info.width as usize, info.height as usize, color);
}

pub fn fill_rect(x: usize, y: usize, w: usize, h: usize, color: u32) {
    let gpu = unsafe { *GPU.get() };
    let fb = &mut unsafe { gpu.as_mut() }
        .expect("gpu not initialized")
        .frame_buffer;
    if w == 0 || h == 0 || x >= fb.width || y >= fb.height {
        return;
    }

    let x_end = x.saturating_add(w).min(fb.width);
    let y_end = y.saturating_add(h).min(fb.height);
    let pixels = unsafe { core::slice::from_raw_parts_mut(fb.ptr, fb.len) };
    for row in y..y_end {
        let start = row * fb.width + x;
        let end = row * fb.width + x_end;
        pixels[start..end].fill(color);
    }
}

pub fn present() {
    let gpu = unsafe { *GPU.get() };
    unsafe { gpu.as_mut() }
        .expect("gpu not initialized")
        .driver
        .flush()
        .expect("failed to flush GPU framebuffer");
}

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    0xff00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

struct VirtioHal;

impl Hal for VirtioHal {
    fn dma_alloc(pages: usize) -> usize {
        let size = pages << 12;
        let next = unsafe { &mut *DMA_NEXT.get() };
        let aligned = (*next + 4095) & !4095;
        let end = aligned + size;
        if end > DMA_POOL_SIZE {
            return 0;
        }
        *next = end;
        unsafe { (*DMA_POOL.inner.get()).as_mut_ptr() as usize + aligned }
    }

    fn dma_dealloc(_paddr: usize, _pages: usize) -> i32 {
        0
    }

    fn phys_to_virt(paddr: usize) -> usize {
        paddr
    }

    fn virt_to_phys(vaddr: usize) -> usize {
        vaddr
    }
}
