#!/bin/sh
set -e

cd "$(dirname "$0")"
cargo build --features pong

if [ -t 0 ]; then
    old="$(stty -g)"
    trap 'stty "$old"' EXIT INT TERM
    stty raw -echo
fi

exec qemu-system-riscv64 \
    -machine virt \
    -display cocoa \
    -serial mon:stdio \
    -global virtio-mmio.force-legacy=false \
    -device virtio-gpu-device,bus=virtio-mmio-bus.0,xres=320,yres=240 \
    -bios none \
    -kernel target/riscv64gc-unknown-none-elf/debug/rosist-sallina-tg-rcore-tutorial-T3L5
