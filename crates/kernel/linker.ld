ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(5); /* read + execute */
    data PT_LOAD FLAGS(6); /* read + write */
}

SECTIONS
{
    /* Raspberry Pi 4 firmware loads the 64-bit kernel8.img at 0x80000. */
    . = 0x00080000;
    __kernel_start = .;

    .text : ALIGN(16)
    {
        KEEP(*(.text._start))
        *(.text .text.*)
    } :text

    .vectors : ALIGN(2048)
    {
        KEEP(*(.vectors))
    } :text

    .rodata : ALIGN(16)
    {
        *(.rodata .rodata.*)
        *(.gcc_except_table .gcc_except_table.*)
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
