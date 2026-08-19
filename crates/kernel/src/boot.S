.arch armv8-a

.section .text._start, "ax"
.global _start
.type _start, %function

_start:
    /* The 64-bit Raspberry Pi boot protocol passes the DTB address in x0. */
    mov x19, x0

    msr daifset, #0xf

    /* QEMU enters an ELF image on every emulated core. Keep only core 0. */
    mrs x0, mpidr_el1
    ands x0, x0, #3
    bne .Lpark

    ldr x0, =__stack_top
    mov sp, x0

    /* The raw image does not contain the NOBITS .bss section. */
    ldr x0, =__bss_start
    ldr x1, =__bss_end
.Lzero_bss:
    cmp x0, x1
    str xzr, [x0], #8
    blo .Lzero_bss

    dsb sy
    isb

    mov x0, x19
    bl kernel_main

.Lpark:
    wfe
    b .Lpark

.size _start, . - _start
