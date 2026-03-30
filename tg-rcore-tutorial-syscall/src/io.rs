// 教程说明：
// 这三个常量对应类 Unix 约定的标准文件描述符。

/// 标准输入文件描述符。
pub const STDIN: usize = 0;
/// 标准输出文件描述符。
pub const STDOUT: usize = 1;
/// 标准错误/调试输出文件描述符。
pub const STDDEBUG: usize = 2;

/// 帧缓冲信息。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameBufferInfo {
    /// 像素宽度。
    pub width: u32,
    /// 像素高度。
    pub height: u32,
}

impl FrameBufferInfo {
    /// 全零初始化值。
    pub const ZERO: Self = Self {
        width: 0,
        height: 0,
    };
}

/// DOOM 扩展使用的 framebuffer 信息。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FramebufferInfo {
    /// 逻辑宽度（像素）。
    pub width: u32,
    /// 逻辑高度（像素）。
    pub height: u32,
    /// 每行字节数。
    pub stride: u32,
    /// 像素位宽（bits per pixel）。
    pub bpp: u32,
}

impl FramebufferInfo {
    /// 全零初始化值。
    pub const ZERO: Self = Self {
        width: 0,
        height: 0,
        stride: 0,
        bpp: 0,
    };
}

/// DOOM 扩展使用的输入事件（Linux evdev 风格）。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct InputEventUser {
    /// 事件类型（例如 EV_KEY=1）。
    pub event_type: u16,
    /// 键码（例如 KEY_W=17）。
    pub code: u16,
    /// 值（按下=1，释放=0，重复=2）。
    pub value: u32,
}
