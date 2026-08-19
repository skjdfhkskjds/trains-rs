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

    /* Normalize firmware and QEMU entry states to non-secure EL1h. */
    mrs x0, CurrentEL
    cmp x0, #0xc
    b.eq .Ldrop_from_el3
    cmp x0, #0x8
    b.eq .Ldrop_from_el2
    cmp x0, #0x4
    b.ne .Lpark
    b .Lenter_el1

.Ldrop_from_el3:
    /* Make the physical timer PPI visible after entering non-secure EL1. */
    ldr x0, =0xff841080
    ldr w1, [x0]
    orr w1, w1, #(1 << 30)
    str w1, [x0]
    dsb sy

    /* AArch64 at lower exception levels, in the non-secure world. */
    mov x0, #0x411
    msr scr_el3, x0

    /* Do not trap the EL1 architectural timer or AArch64 execution to EL2. */
    mov x0, #3
    msr cnthctl_el2, x0
    msr cntvoff_el2, xzr
    mov x0, #(1 << 31)
    msr hcr_el2, x0
    msr cptr_el2, xzr

    adr x0, .Lenter_el1
    msr elr_el3, x0
    mov x0, #0x3c5
    msr spsr_el3, x0
    eret

.Ldrop_from_el2:
    mov x0, #3
    msr cnthctl_el2, x0
    msr cntvoff_el2, xzr
    mov x0, #(1 << 31)
    msr hcr_el2, x0
    msr cptr_el2, xzr

    adr x0, .Lenter_el1
    msr elr_el2, x0
    mov x0, #0x3c5
    msr spsr_el2, x0
    eret

.Lenter_el1:
    msr spsel, #1
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
