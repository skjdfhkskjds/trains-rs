#!/usr/bin/env sh
set -eu

cargo build --release

toolchain_root=$(rustc --print sysroot)
host_target=$(rustc -vV | sed -n 's/^host: //p')
llvm_objcopy="$toolchain_root/lib/rustlib/$host_target/bin/llvm-objcopy"
kernel_elf="target/aarch64-unknown-none/release/trains-kernel"

if [ ! -x "$llvm_objcopy" ]; then
    echo "llvm-objcopy is missing; install the llvm-tools-preview rustup component" >&2
    exit 1
fi

mkdir -p build
"$llvm_objcopy" --output-target=binary "$kernel_elf" build/kernel8.img

echo "ELF:       $kernel_elf"
echo "Pi image:  build/kernel8.img"
