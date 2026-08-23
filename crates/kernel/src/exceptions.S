.arch armv8-a

.section .vectors, "ax"
.global vector_table
.balign 2048
vector_table:
    b current_sp0_sync
    .balign 0x80
    b current_sp0_irq
    .balign 0x80
    b current_sp0_fiq
    .balign 0x80
    b current_sp0_serror
    .balign 0x80

    b current_spx_sync
    .balign 0x80
    b current_spx_irq
    .balign 0x80
    b current_spx_fiq
    .balign 0x80
    b current_spx_serror
    .balign 0x80

    b lower_a64_sync
    .balign 0x80
    b lower_a64_irq
    .balign 0x80
    b lower_a64_fiq
    .balign 0x80
    b lower_a64_serror
    .balign 0x80

    b lower_a32_sync
    .balign 0x80
    b lower_a32_irq
    .balign 0x80
    b lower_a32_fiq
    .balign 0x80
    b lower_a32_serror
    .balign 2048

.macro exception_entry vector
    sub sp, sp, #848
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]

    mrs x1, sp_el0
    str x1, [sp, #248]
    mrs x1, elr_el1
    str x1, [sp, #256]
    mrs x1, spsr_el1
    str x1, [sp, #264]
    mrs x1, esr_el1
    str x1, [sp, #272]
    mrs x1, far_el1
    str x1, [sp, #280]
    mov x1, #\vector
    str x1, [sp, #288]
    str xzr, [sp, #296]

    mrs x1, fpcr
    str x1, [sp, #304]
    mrs x1, fpsr
    str x1, [sp, #312]
    mrs x1, tpidr_el0
    str x1, [sp, #320]
    mrs x1, tpidrro_el0
    str x1, [sp, #328]
    stp q0, q1, [sp, #336]
    stp q2, q3, [sp, #368]
    stp q4, q5, [sp, #400]
    stp q6, q7, [sp, #432]
    stp q8, q9, [sp, #464]
    stp q10, q11, [sp, #496]
    stp q12, q13, [sp, #528]
    stp q14, q15, [sp, #560]
    stp q16, q17, [sp, #592]
    stp q18, q19, [sp, #624]
    stp q20, q21, [sp, #656]
    stp q22, q23, [sp, #688]
    stp q24, q25, [sp, #720]
    stp q26, q27, [sp, #752]
    stp q28, q29, [sp, #784]
    stp q30, q31, [sp, #816]

    /* Kernel state is not reentrant; preserve the prior mask in SPSR_EL1. */
    msr daifset, #2
    mov x0, sp
    bl exception_handler

    ldr x0, [sp, #248]
    msr sp_el0, x0
    ldr x0, [sp, #256]
    msr elr_el1, x0
    ldr x0, [sp, #264]
    msr spsr_el1, x0

    ldr x0, [sp, #304]
    msr fpcr, x0
    ldr x0, [sp, #312]
    msr fpsr, x0
    ldr x0, [sp, #320]
    msr tpidr_el0, x0
    ldr x0, [sp, #328]
    msr tpidrro_el0, x0
    ldp q30, q31, [sp, #816]
    ldp q28, q29, [sp, #784]
    ldp q26, q27, [sp, #752]
    ldp q24, q25, [sp, #720]
    ldp q22, q23, [sp, #688]
    ldp q20, q21, [sp, #656]
    ldp q18, q19, [sp, #624]
    ldp q16, q17, [sp, #592]
    ldp q14, q15, [sp, #560]
    ldp q12, q13, [sp, #528]
    ldp q10, q11, [sp, #496]
    ldp q8, q9, [sp, #464]
    ldp q6, q7, [sp, #432]
    ldp q4, q5, [sp, #400]
    ldp q2, q3, [sp, #368]
    ldp q0, q1, [sp, #336]

    ldr x30, [sp, #240]
    ldp x28, x29, [sp, #224]
    ldp x26, x27, [sp, #208]
    ldp x24, x25, [sp, #192]
    ldp x22, x23, [sp, #176]
    ldp x20, x21, [sp, #160]
    ldp x18, x19, [sp, #144]
    ldp x16, x17, [sp, #128]
    ldp x14, x15, [sp, #112]
    ldp x12, x13, [sp, #96]
    ldp x10, x11, [sp, #80]
    ldp x8, x9, [sp, #64]
    ldp x6, x7, [sp, #48]
    ldp x4, x5, [sp, #32]
    ldp x2, x3, [sp, #16]
    ldp x0, x1, [sp, #0]
    add sp, sp, #848
    eret
.endm

current_sp0_sync:    exception_entry 0
current_sp0_irq:     exception_entry 1
current_sp0_fiq:     exception_entry 2
current_sp0_serror:  exception_entry 3
current_spx_sync:    exception_entry 4
current_spx_irq:     exception_entry 5
current_spx_fiq:     exception_entry 6
current_spx_serror:  exception_entry 7
lower_a64_sync:      exception_entry 8
lower_a64_irq:       exception_entry 9
lower_a64_fiq:       exception_entry 10
lower_a64_serror:    exception_entry 11
lower_a32_sync:      exception_entry 12
lower_a32_irq:       exception_entry 13
lower_a32_fiq:       exception_entry 14
lower_a32_serror:    exception_entry 15
