#!/bin/sh
set -e

cd "$(dirname "$0")"
cargo build --features pacman

XRES="${PACMAN_XRES:-640}"
YRES="${PACMAN_YRES:-480}"

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
    -drive file=target/riscv64gc-unknown-none-elf/debug/fs.img,if=none,format=raw,id=x0 \
    -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
    -device virtio-gpu-device,bus=virtio-mmio-bus.1,xres="$XRES",yres="$YRES" \
    -bios none \
    -kernel target/riscv64gc-unknown-none-elf/debug/rosist-sallina-tg-rcore-tutorial-T3L7
