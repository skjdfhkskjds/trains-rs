#!/usr/bin/env sh
set -eu

kernel=${1:-target/aarch64-unknown-none/debug/trains-kernel}
if [ "$#" -gt 0 ]; then
    shift
fi

exec qemu-system-aarch64 \
    -machine raspi4b \
    -cpu cortex-a72 \
    -display none \
    -monitor none \
    -serial stdio \
    -no-reboot \
    -kernel "$kernel" \
    "$@"
