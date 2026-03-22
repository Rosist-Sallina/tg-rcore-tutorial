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
