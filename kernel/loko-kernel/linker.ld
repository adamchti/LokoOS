/* LokoOS kernel link script.
 *
 * The kernel is linked for the top 2 GiB of the canonical higher half. Two
 * reasons: every user address is then unambiguously "not a kernel address"
 * with a single sign test, and the -2 GiB region lets the compiler use the
 * `kernel` code model, where all intra-kernel references fit in a 32-bit
 * signed displacement. That is smaller and faster than the alternatives, and
 * it matters here because this code runs on every system call.
 *
 * Sections are separated onto their own pages so that the page tables can be
 * given real permissions: executable code is not writable, and writable data is
 * not executable. A kernel that maps itself RWX has given away W^X before
 * userland has even started.
 */

ENTRY(_start)
OUTPUT_FORMAT(elf64-x86-64)

KERNEL_VMA = 0xFFFFFFFF80000000;

PHDRS
{
    text    PT_LOAD FLAGS((1 << 2) | (1 << 0));  /* R + X */
    rodata  PT_LOAD FLAGS((1 << 2));             /* R     */
    data    PT_LOAD FLAGS((1 << 2) | (1 << 1));  /* R + W */
}

SECTIONS
{
    . = KERNEL_VMA;
    __kernel_start = .;

    .text ALIGN(4K) : {
        *(.text.boot)
        *(.text .text.*)
    } :text

    . = ALIGN(4K);
    .rodata ALIGN(4K) : {
        *(.rodata .rodata.*)
        *(.gcc_except_table .gcc_except_table.*)
    } :rodata

    . = ALIGN(4K);
    .data ALIGN(4K) : {
        *(.data .data.*)
        *(.got .got.plt)
    } :data

    .bss ALIGN(4K) : {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(4K);
        __bss_end = .;
    } :data

    . = ALIGN(4K);
    __kernel_end = .;

    /* Unwind tables are dead weight in a panic=abort kernel, and the note and
     * comment sections only carry toolchain metadata. Dropping them keeps the
     * image smaller; `cargo xtask size` reports the result against the base-OS
     * budget from requirement 2. */
    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note .note.*)
        *(.comment)
    }
}
