//! SMP 基础设施。

#![no_std]
#![deny(warnings)]

/// 当前实验固定支持的核数。
pub const MAX_CPUS: usize = 2;

pub mod barrier;
pub mod hart;
pub mod ipi;
pub mod percpu;
pub mod spin;
