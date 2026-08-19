ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(5); /* read + execute */
    data PT_LOAD FLAGS(6); /* read + write */
}

SECTIONS
{
    /* Raspberry Pi's 32-bit firmware loads kernel7.img at 0x8000. */
    . = 0x00008000;
    __kernel_start = .;

    .text : ALIGN(16)
    {
        KEEP(*(.text._start))
        *(.text .text.*)
    } :text

    .rodata : ALIGN(16)
    {
        *(.rodata .rodata.*)
        *(.ARM.extab .ARM.extab.*)
    } :text

    .ARM.exidx : ALIGN(16)
    {
        __exidx_start = .;
        *(.ARM.exidx .ARM.exidx.*)
        __exidx_end = .;
    } :text

    .data : ALIGN(16)
    {
        *(.data .data.*)
    } :data

    .bss (NOLOAD) : ALIGN(16)
    {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    } :data

    /* A temporary boot stack for the primary core. */
    .stack (NOLOAD) : ALIGN(16)
    {
        . += 64K;
        __stack_top = .;
    } :data

    __kernel_end = .;

    /DISCARD/ :
    {
        *(.comment)
        *(.eh_frame*)
    }
}
