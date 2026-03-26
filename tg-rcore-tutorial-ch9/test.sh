#!/bin/bash
set -e
set -o pipefail

run_base() {
    cargo run 2>&1 | tee /dev/stderr | grep -E "vm_touch OK|vm_swap OK|vm_prot begin" >/dev/null
}

run_exercise() {
    cargo run --features exercise 2>&1 | tee /dev/stderr | grep -E "algo +pattern|VMBENCH PASS" >/dev/null
}

case "${1:-all}" in
    base)
        run_base
        ;;
    exercise)
        run_exercise
        ;;
    all)
        run_base
        run_exercise
        ;;
    *)
        echo "用法: $0 [base|exercise|all]"
        exit 1
        ;;
esac
