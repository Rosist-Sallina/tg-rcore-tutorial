#![no_std]
#![no_main]

#[cfg(feature = "real_doom")]
mod platform;

use core::cell::UnsafeCell;
use user_lib::{
    framebuffer_flush, framebuffer_info, framebuffer_init, input_event, sched_yield,
    FramebufferInfo, InputEventUser, println,
};

const WIDTH: usize = 320;
const HEIGHT: usize = 200;
const BPP: usize = 4;
const FRAME_BYTES: usize = WIDTH * HEIGHT * BPP;
const EV_KEY: u16 = 1;
const KEY_ESC: u16 = 1;

struct StaticFrame {
    inner: UnsafeCell<[u8; FRAME_BYTES]>,
}

unsafe impl Sync for StaticFrame {}

impl StaticFrame {
    const fn new() -> Self {
        Self {
            inner: UnsafeCell::new([0; FRAME_BYTES]),
        }
    }

    #[inline]
    fn get(&self) -> *mut [u8; FRAME_BYTES] {
        self.inner.get()
    }
}

static FRAME: StaticFrame = StaticFrame::new();

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    if framebuffer_init() != 0 {
        println!("[doom] framebuffer_init failed");
        return -1;
    }
    let mut info = FramebufferInfo::ZERO;
    if framebuffer_info(&mut info) != 0 {
        println!("[doom] framebuffer_info failed");
        return -1;
    }

    #[cfg(feature = "real_doom")]
    {
        return platform::run();
    }

    #[cfg(not(feature = "real_doom"))]
    {
        println!(
            "[doom] stub mode: fb={}x{} stride={} bpp={}",
            info.width, info.height, info.stride, info.bpp
        );
        run_stub();
        0
    }
}

#[cfg(not(feature = "real_doom"))]
fn run_stub() {
    let frame = unsafe { &mut *FRAME.get() };
    let mut tick = 0u32;
    loop {
        if should_exit() {
            println!("[doom] exit by ESC");
            return;
        }
        fill_frame(frame, tick);
        if framebuffer_flush(frame) != 0 {
            println!("[doom] framebuffer_flush failed");
            return;
        }
        tick = tick.wrapping_add(1);
        sched_yield();
    }
}

#[cfg(not(feature = "real_doom"))]
fn should_exit() -> bool {
    let mut event = InputEventUser::default();
    if input_event(&mut event) == 0 {
        return event.event_type == EV_KEY && event.code == KEY_ESC && event.value != 0;
    }
    false
}

#[cfg(not(feature = "real_doom"))]
fn fill_frame(frame: &mut [u8], tick: u32) {
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let offset = (y * WIDTH + x) * BPP;
            let r = ((x as u32 + tick) & 0xff) as u8;
            let g = ((y as u32 + tick * 2) & 0xff) as u8;
            let b = (((x as u32) ^ (y as u32) ^ tick) & 0xff) as u8;
            frame[offset] = b;
            frame[offset + 1] = g;
            frame[offset + 2] = r;
            frame[offset + 3] = 0xff;
        }
    }
}
