# trains-rs boot skeleton

This repository contains the AArch64 base platform for a microkernel targeting
the PiKVM V4 Plus. That device uses a Raspberry Pi Compute Module 4 with a
BCM2711 SoC and four Armv8-A Cortex-A72 cores.

The workspace contains:

- `crates/application`: Interactive entrypoint and application commands.
- `crates/kernel`: AArch64 boot, exception, task, scheduler, and command runtime.
- `crates/logger`: Allocation-free, color-coded logging over the platform console.
- `crates/platform`: `no_std` BCM2711 GPIO, PL011, Arm generic timer, and
  GIC-400 support.
- `xtask`: host-side image builder used by `cargo image`.

## Build and run in QEMU

Install QEMU with `qemu-system-aarch64` and run:

```sh
cargo image
cargo qemu
```

The QEMU runner uses the `raspi4b` machine, which models a BCM2711-class
Cortex-A72 system. The release ELF is passed directly so QEMU honors its load
segments and entry point at `0x80000`.

Expected serial output:

```text
[INFO] trains-rs: Raspberry Pi 4 / BCM2711 platform ready
[INFO] trains-rs: exception handling ready
[INFO] trains-rs: context switching ready
[INFO] trains-rs: task primitive ready
[INFO] trains-rs: cooperative scheduling ready
[INFO] trains-rs: Arm generic timer ready
[INFO] trains-rs: interrupt handling ready
> demo
[INFO] trains-rs: cooperative yield example
[DEBUG] task 0 (priority 0): before yield
[DEBUG] task 0 (priority 0): after yield
[DEBUG] task 1 (priority 1): before yield
[DEBUG] task 1 (priority 1): after yield
[DEBUG] task 2 (priority 2): before yield
[DEBUG] task 2 (priority 2): after yield
[INFO] trains-rs: cooperative yield example complete
>
```

The serial terminal renders info headers in light blue, warnings in yellow,
errors in red, and debug headers in magenta.

Enter `demo` at the prompt to run the cooperative scheduling example again.

Exit QEMU with Ctrl-C.

QEMU's Pi 4 model does not implement every PiKVM device. In particular, PCIe,
GENET Ethernet, and the PiKVM carrier board's capture and management devices
require validation on real hardware.

## PiKVM V4 Plus image

`cargo image` emits `build/kernel8.img`. Copy it to a Raspberry Pi firmware
boot partition and use these minimum `config.txt` settings:

```ini
arm_64bit=1
enable_gic=1
kernel=kernel8.img
enable_uart=1
```

Keep the normal Raspberry Pi 4 firmware and the CM4 device tree on the boot
partition. The platform currently assumes low-peripheral mode, where BCM2711
devices start at `0xfe000000` and the GIC-400 starts at `0xff840000`.

## Current boot contract

- ISA and ABI: little-endian AArch64 (`aarch64-unknown-none`)
- CPU: Armv8-A Cortex-A72, four cores
- QEMU machine: `raspi4b`, 2 GiB RAM
- firmware image: `kernel8.img`
- linked/load address: `0x00080000`
- firmware argument: device-tree address in `x0`
- execution level: firmware EL3/EL2 entry is normalized to non-secure EL1h
- primary core: MPIDR affinity 0; secondary cores park in `wfe`
- peripherals: BCM2711 GPIO, PL011, Arm generic physical timer, GIC-400
- exception vectors: installed at EL1; exceptions capture general-purpose and FP/SIMD registers
- context switching: complete register contexts can be captured from and restored through an exception frame
- tasks: a pinned task owns an 8 KiB stack and its initialized register context
- scheduling: lower-valued priorities run first; equal priorities use FIFO ordering
- IRQ handling: GIC claims are dispatched and completed; timer IRQ delivery is enabled on demand
- MMU, caches, allocator, and SMP: not initialized
- IRQ delivery is enabled on demand; FIQ remains masked
