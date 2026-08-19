#!/usr/bin/env sh
set -eu

kernel=${1:-target/armv7a-none-eabi/debug/trains-kernel}
if [ "$#" -gt 0 ]; then
    shift
fi

exec qemu-system-arm \
    -machine raspi2b \
    -display none \
    -monitor none \
    -serial stdio \
    -no-reboot \
    -kernel "$kernel" \
    "$@"

