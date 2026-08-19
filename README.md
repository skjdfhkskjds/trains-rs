# trains-rs boot skeleton

This repository contains the base platform for a 32-bit Raspberry Pi 2B kernel
written in Rust. It selects Rust's bare-metal Armv7-A soft-float target, enters
through a short A32 assembly stub, clears `.bss`, establishes a stack, and calls
Rust. The Raspberry Pi 2 platform layer provides typed GPIO, PL011 console,
system timer, and interrupt-controller access.

The product is a Cargo workspace:

- `crates/kernel` owns the boot entry point, linker layout, and kernel binary.
- `crates/platform` is a reusable `no_std` hardware abstraction crate.
- `xtask` is an isolated host-side helper used by `cargo image`.

Kernel code selects `RASPI2B` and uses the `Platform` trait to obtain its
console, GPIO, timer, and interrupt-controller capabilities. The concrete
Raspberry Pi implementation exposes no public inherent device accessors.

## Build and run in QEMU

The pinned toolchain file asks rustup for `armv7a-none-eabi` and
`llvm-tools-preview`. Then:

```sh
cargo image
cargo qemu
```

`cargo image` builds the release ELF and extracts `build/kernel7.img` from its
loadable sections. `cargo qemu` builds the release ELF and passes it to QEMU.
This is intentional: QEMU recognizes
the ELF segments and loads them at the linker's `0x8000` address. Passing the
raw `kernel7.img` to QEMU's `-kernel` option instead invokes QEMU's generic
32-bit Linux image loader, which relocates it to `0x10000` and does not match
the Raspberry Pi firmware layout.

Expected serial output:

```text
trains-rs: Raspberry Pi 2 platform ready
trains-rs: system timer ready
trains-rs: interrupt controller ready
```

Exit QEMU with Ctrl-C.

## Real Raspberry Pi 2B image

`cargo image` also strips the ELF container and emits
`build/kernel7.img`. A Pi boot partition needs the normal Raspberry Pi firmware
files (`bootcode.bin`, `start.elf`, and matching `fixup.dat`), the appropriate
Pi 2 DTB, `build/kernel7.img`, and the settings in `boot/config.txt`.

The entry stub preserves `r0`, `r1`, and `r2`; in the firmware boot path `r2`
can carry the device-tree pointer. The Rust entry does not parse it yet.

## Current boot contract

- ISA: 32-bit Armv7-A (A32), soft-float ABI
- QEMU machine: `raspi2b` (Cortex-A7, four cores, 1 GiB RAM)
- linked/load address: `0x00008000`
- early stack: 64 KiB linker-reserved `NOLOAD` region
- primary core: MPIDR affinity 0; other QEMU cores park in `wfe`
- platform: GPIO, PL011, system timer, and interrupt-controller register access
- MMU, caches, exception vectors, allocator, and SMP: not initialized
- IRQ lines remain CPU-masked until exception handling is implemented

Using soft-float avoids making FPU setup part of the initial boot contract.
CPU-specific optimization can be enabled after the low-level VFP/NEON state is
initialized deliberately.
