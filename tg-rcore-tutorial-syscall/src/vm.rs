/// `vmctl` 命令：设置当前进程算法。
pub const VMCTL_SET_ALGO: usize = 0;
/// `vmctl` 命令：设置当前进程常驻页配额。
pub const VMCTL_SET_QUOTA: usize = 1;
/// `vmctl` 命令：重置当前进程统计基线。
pub const VMCTL_RESET_STATS: usize = 2;
/// `vmctl` 命令：读取当前进程统计。
pub const VMCTL_GET_STATS: usize = 3;

/// 章节实验支持的页面置换算法。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum VmAlgo {
    /// 先进先出。
    Fifo = 0,
    /// 二次机会时钟。
    Clock = 1,
    /// Aging 近似 LRU。
    LruApprox = 2,
    /// 简化工作集。
    WorkingSet = 3,
}

impl VmAlgo {
    /// 从原始值解析算法。
    #[inline]
    pub const fn from_raw(raw: usize) -> Option<Self> {
        match raw {
            0 => Some(Self::Fifo),
            1 => Some(Self::Clock),
            2 => Some(Self::LruApprox),
            3 => Some(Self::WorkingSet),
            _ => None,
        }
    }
}

/// 当前进程虚拟内存实验统计。
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct VmStats {
    /// 当前算法。
    pub algo: u32,
    /// 当前配额。
    pub quota: u32,
    /// 当前常驻页数。
    pub resident_pages: u32,
    /// 当前换出页数。
    pub swap_pages: u32,
    /// 累积缺页次数。
    pub page_faults: u64,
    /// 累积置换次数。
    pub evictions: u64,
    /// 累积脏页回写次数。
    pub write_backs: u64,
    /// 累积抖动事件次数。
    pub thrash_events: u64,
    /// 统计窗口内的纳秒时间。
    pub elapsed_ns: u64,
    /// 统计窗口内的 retired instructions。
    pub instret_delta: u64,
}

impl VmStats {
    /// 全零统计值。
    pub const ZERO: Self = Self {
        algo: VmAlgo::Fifo as u32,
        quota: 0,
        resident_pages: 0,
        swap_pages: 0,
        page_faults: 0,
        evictions: 0,
        write_backs: 0,
        thrash_events: 0,
        elapsed_ns: 0,
        instret_delta: 0,
    };
}
