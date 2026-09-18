//! The Global Descriptor Table and Task State Segment.
//!
//! In long mode the GDT barely does segmentation any more, but it still decides
//! two things that matter: the privilege level code runs at, and — through the
//! TSS — which stack the CPU switches to for specific faults.
//!
//! That second part is why this module exists before anything else. A kernel
//! that takes a page fault on its own stack because the stack itself is the
//! problem will take a double fault trying to push the fault frame, and then a
//! triple fault, which on x86 means the machine silently resets. A dedicated
//! Interrupt Stack Table entry is what turns that instant reboot into a
//! diagnosable error message.

use core::ptr::addr_of;
use spin::Lazy;
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

/// IST slot for the double-fault handler.
pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;
/// IST slot for the page-fault handler.
pub const PAGE_FAULT_IST_INDEX: u16 = 1;
/// IST slot for the non-maskable interrupt handler.
pub const NMI_IST_INDEX: u16 = 2;

/// 20 KiB per emergency stack. Enough for a fault frame and a backtrace, small
/// enough that three of them do not meaningfully dent the kernel image.
const EMERGENCY_STACK_SIZE: usize = 4096 * 5;

/// A naturally aligned emergency stack.
///
/// The alignment is required: the SysV ABI wants a 16-byte aligned stack
/// pointer at a function boundary, and the CPU does not fix this up for us when
/// it switches stacks via the IST.
/// The bytes are never read through this type — only the CPU touches them,
/// via the address recorded in the TSS.
#[repr(align(16))]
struct EmergencyStack(#[allow(dead_code)] [u8; EMERGENCY_STACK_SIZE]);

static mut DOUBLE_FAULT_STACK: EmergencyStack = EmergencyStack([0; EMERGENCY_STACK_SIZE]);
static mut PAGE_FAULT_STACK: EmergencyStack = EmergencyStack([0; EMERGENCY_STACK_SIZE]);
static mut NMI_STACK: EmergencyStack = EmergencyStack([0; EMERGENCY_STACK_SIZE]);

/// The top of `stack`, which is where x86 stacks begin.
///
/// # Safety
///
/// `stack` must point to a live `EmergencyStack` that nothing else uses.
unsafe fn stack_top(stack: *const EmergencyStack) -> VirtAddr {
    let base = stack as u64;
    // Stacks grow downward, so the CPU is given the address one past the end.
    VirtAddr::new(base + EMERGENCY_STACK_SIZE as u64)
}

static TSS: Lazy<TaskStateSegment> = Lazy::new(|| {
    let mut tss = TaskStateSegment::new();
    // SAFETY: these statics are written only here, once, during single-threaded
    // early boot, and are never referenced as Rust references afterwards — the
    // CPU reaches them through the TSS, not through this code.
    unsafe {
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] =
            stack_top(addr_of!(DOUBLE_FAULT_STACK));
        tss.interrupt_stack_table[PAGE_FAULT_IST_INDEX as usize] =
            stack_top(addr_of!(PAGE_FAULT_STACK));
        tss.interrupt_stack_table[NMI_IST_INDEX as usize] = stack_top(addr_of!(NMI_STACK));
    }
    tss
});

/// The selectors the rest of the kernel needs to know about.
///
/// The user selectors are set up now rather than later because `syscall` and
/// `sysret` read both from one MSR field and require a specific adjacency, so
/// the ordering decision belongs with the table that encodes it. Nothing reads
/// them until there is a userland to return to.
#[allow(dead_code)]
pub struct Selectors {
    /// Ring 0 code.
    pub kernel_code: SegmentSelector,
    /// Ring 0 data.
    pub kernel_data: SegmentSelector,
    /// Ring 3 code.
    pub user_code: SegmentSelector,
    /// Ring 3 data.
    pub user_data: SegmentSelector,
    /// The TSS.
    pub tss: SegmentSelector,
}

static GDT: Lazy<(GlobalDescriptorTable, Selectors)> = Lazy::new(|| {
    let mut gdt = GlobalDescriptorTable::new();
    let kernel_code = gdt.append(Descriptor::kernel_code_segment());
    let kernel_data = gdt.append(Descriptor::kernel_data_segment());
    // Ordered user data before user code because `syscall`/`sysret` derive both
    // user selectors from a single MSR field and require exactly this order.
    // Getting it wrong produces a general protection fault on the first return
    // to userland, a long way from here.
    let user_data = gdt.append(Descriptor::user_data_segment());
    let user_code = gdt.append(Descriptor::user_code_segment());
    let tss = gdt.append(Descriptor::tss_segment(&TSS));
    (
        gdt,
        Selectors {
            kernel_code,
            kernel_data,
            user_code,
            user_data,
            tss,
        },
    )
});

/// Loads the GDT and reloads every segment register.
///
/// Must run before [`super::idt::init`]: the IDT's entries name a code segment
/// selector, and that selector has to be valid by the time an interrupt fires.
pub fn init() {
    let (table, selectors) = &*GDT;
    table.load();

    // SAFETY: the selectors come from the table just loaded, so each one indexes
    // a descriptor that exists and has the privilege level being set.
    unsafe {
        CS::set_reg(selectors.kernel_code);
        DS::set_reg(selectors.kernel_data);
        ES::set_reg(selectors.kernel_data);
        SS::set_reg(selectors.kernel_data);
        load_tss(selectors.tss);
    }
}

/// The selectors, for the syscall entry path and for building user threads.
#[allow(dead_code)]
#[must_use]
pub fn selectors() -> &'static Selectors {
    &GDT.1
}
