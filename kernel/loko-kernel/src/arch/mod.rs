//! Architecture-specific kernel bring-up.
//!
//! Only x86_64 exists today. The module boundary is here so that adding aarch64
//! later is a matter of writing a sibling module, not of unpicking assumptions
//! that have spread through the rest of the kernel.

pub mod gdt;
pub mod idt;

/// Sets up the CPU: descriptor tables, then interrupt handling.
///
/// Order matters. The IDT names a code selector from the GDT, so the GDT has to
/// be live first.
pub fn init() {
    gdt::init();
    idt::init();
}

/// Stops this CPU permanently, with interrupts disabled.
///
/// Used by the panic path. `hlt` in a loop rather than a spin so that a stopped
/// machine does not sit at 100% of a core, which matters on a laptop that a
/// user may not notice has stopped until the fan tells them.
pub fn halt_forever() -> ! {
    x86_64::instructions::interrupts::disable();
    loop {
        x86_64::instructions::hlt();
    }
}
