//! The Interrupt Descriptor Table and CPU exception handlers.
//!
//! Every handler here follows the same rule: say what happened, in terms a
//! person can act on, and then stop. A kernel that continues after an
//! unexpected exception is a kernel that will corrupt something before it
//! finally fails, at which point the original cause is unrecoverable.
//!
//! The messages are written for whoever is looking at the screen, which per
//! requirement 64 means no bare hex codes as the headline. The hex is still
//! printed — underneath, on the serial log, where it is useful.

use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

use super::gdt;
use crate::{error, panic_screen};

use spin::Lazy;

static IDT: Lazy<InterruptDescriptorTable> = Lazy::new(|| {
    let mut idt = InterruptDescriptorTable::new();

    idt.divide_error.set_handler_fn(divide_error);
    idt.debug.set_handler_fn(debug_exception);
    idt.invalid_opcode.set_handler_fn(invalid_opcode);
    idt.general_protection_fault
        .set_handler_fn(general_protection_fault);

    // SAFETY: these IST indices are the ones `gdt` filled in, and each names a
    // distinct stack. Pointing two handlers that can nest at the same IST slot
    // would let the second overwrite the first's frame.
    unsafe {
        idt.non_maskable_interrupt
            .set_handler_fn(non_maskable_interrupt)
            .set_stack_index(gdt::NMI_IST_INDEX);
        idt.double_fault
            .set_handler_fn(double_fault)
            .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        idt.page_fault
            .set_handler_fn(page_fault)
            .set_stack_index(gdt::PAGE_FAULT_IST_INDEX);
    }

    idt
});

/// Loads the IDT. [`gdt::init`] must have run first.
pub fn init() {
    IDT.load();
}

extern "x86-interrupt" fn divide_error(frame: InterruptStackFrame) {
    error!("cpu", "divide error at {:#x}", frame.instruction_pointer);
    panic_screen("LokoOS stopped because a program divided by zero inside the system.");
}

extern "x86-interrupt" fn debug_exception(frame: InterruptStackFrame) {
    // A debug exception with no debugger attached means something set a
    // breakpoint register that nothing owns. Worth a line, not worth stopping.
    error!(
        "cpu",
        "unexpected debug exception at {:#x}", frame.instruction_pointer
    );
}

extern "x86-interrupt" fn invalid_opcode(frame: InterruptStackFrame) {
    error!("cpu", "invalid opcode at {:#x}", frame.instruction_pointer);
    panic_screen(
        "LokoOS stopped because it tried to run an instruction this processor doesn't support.",
    );
}

extern "x86-interrupt" fn non_maskable_interrupt(frame: InterruptStackFrame) {
    // An NMI is usually a hardware error: memory parity, a watchdog, a machine
    // check on its way. It is not safe to assume anything about system state.
    error!(
        "cpu",
        "non-maskable interrupt at {:#x}", frame.instruction_pointer
    );
    panic_screen("LokoOS stopped because this device's hardware reported a fault.");
}

extern "x86-interrupt" fn general_protection_fault(frame: InterruptStackFrame, code: u64) {
    error!(
        "cpu",
        "general protection fault at {:#x}, selector {:#x}", frame.instruction_pointer, code
    );
    panic_screen("LokoOS stopped because a program tried to do something it isn't allowed to do.");
}

extern "x86-interrupt" fn page_fault(frame: InterruptStackFrame, code: PageFaultErrorCode) {
    // Cr2 holds the address that was touched. Read it first: any later fault
    // would overwrite it.
    let address = Cr2::read();

    error!(
        "memory",
        "page fault accessing {:?} at {:#x}, cause {:?}", address, frame.instruction_pointer, code
    );

    // Once there is a userland, a fault from ring 3 becomes that process's
    // problem rather than the system's. Until then, every page fault is a
    // kernel bug and stopping is correct.
    if code.contains(PageFaultErrorCode::USER_MODE) {
        panic_screen("LokoOS stopped an app that tried to use memory that isn't its own.");
    } else {
        panic_screen("LokoOS stopped because of a problem with this device's memory.");
    }
}

extern "x86-interrupt" fn double_fault(frame: InterruptStackFrame, _code: u64) -> ! {
    // A double fault means a fault happened while handling a fault. The next
    // step after this is a triple fault, which resets the machine with no
    // message at all, so this handler must not fail. It formats nothing
    // complicated and touches no locks.
    error!("cpu", "double fault at {:#x}", frame.instruction_pointer);
    panic_screen("LokoOS stopped because it ran into a problem it couldn't recover from.");
}
