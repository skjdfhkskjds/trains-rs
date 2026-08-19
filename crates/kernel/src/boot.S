.syntax unified
.arch armv7-a
.arm

.section .text._start, "ax"
.global _start
.type _start, %function

_start:
    /* Preserve the firmware/QEMU boot arguments for Rust. */
    mov r4, r0
    mov r5, r1
    mov r6, r2

    cpsid if

    /* QEMU enters an ELF image on every emulated core. Keep only core 0. */
    mrc p15, 0, r0, c0, c0, 5
    ands r0, r0, #3
    bne .Lpark

    ldr sp, =__stack_top

    /* The raw image does not contain the NOBITS .bss section. */
    ldr r0, =__bss_start
    ldr r1, =__bss_end
    mov r2, #0
.Lzero_bss:
    cmp r0, r1
    strlo r2, [r0], #4
    blo .Lzero_bss

    dsb
    isb

    mov r0, r4
    mov r1, r5
    mov r2, r6
    bl kernel_main

.Lpark:
    wfe
    b .Lpark

.size _start, . - _start
