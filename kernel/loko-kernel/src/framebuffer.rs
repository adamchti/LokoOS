//! The early-boot framebuffer.
//!
//! This is not the LokoOS compositor. It is the fallback surface the kernel
//! paints on when something has gone wrong before the graphics stack exists, so
//! that a failure is visible to someone looking at the screen rather than only
//! to someone holding a serial cable.
//!
//! ## What is and is not implemented
//!
//! Implemented: mapping the firmware framebuffer, clearing it, filling
//! rectangles, and the LokoOS stop screen.
//!
//! **Not implemented: text rendering.** There is no glyph rasteriser here yet,
//! so the stop screen is a colour and a shape, and the message text goes to the
//! serial log. Requirement 64 wants that message on the screen, and it will be
//! once a font is in place — tracked in `documentation/STATUS.md` rather than
//! papered over here.

use loko_boot_protocol::{Framebuffer, PixelFormat};
use spin::Mutex;

/// A colour in the kernel's own space, converted per framebuffer format.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Colour {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Colour {
    /// The LokoOS stop-screen background: a deep, calm blue-grey. Chosen to
    /// read as "something went wrong" without the alarm of a red screen, which
    /// tends to make people power-cycle before reading anything.
    pub const STOP_BACKGROUND: Colour = Colour {
        r: 18,
        g: 24,
        b: 38,
    };
    /// The accent used for the stop-screen indicator.
    pub const STOP_ACCENT: Colour = Colour {
        r: 94,
        g: 148,
        b: 255,
    };
    /// Plain black. Used by the compositor hand-off, which does not exist yet.
    #[allow(dead_code)]
    pub const BLACK: Colour = Colour { r: 0, g: 0, b: 0 };

    /// Packs into the four bytes this framebuffer expects.
    #[must_use]
    pub const fn to_bytes(self, format: PixelFormat) -> [u8; 4] {
        match format {
            PixelFormat::Bgrx8888 => [self.b, self.g, self.r, 0],
            PixelFormat::Rgbx8888 => [self.r, self.g, self.b, 0],
        }
    }
}

/// The framebuffer description, once the kernel has one.
static SURFACE: Mutex<Option<Framebuffer>> = Mutex::new(None);

/// Records the framebuffer the bootloader set up.
///
/// # Safety
///
/// `framebuffer.base` must be a valid virtual address in the kernel's address
/// space, mapped writable, for at least `framebuffer.size` bytes.
pub unsafe fn init(framebuffer: Framebuffer) {
    if !framebuffer.is_present() {
        return;
    }
    *SURFACE.lock() = Some(framebuffer);
}

/// Whether there is a framebuffer to draw on.
///
/// Read by the graphics stack when it starts, to decide whether to take over
/// the firmware surface or bring up a driver from nothing.
#[allow(dead_code)]
#[must_use]
pub fn is_available() -> bool {
    SURFACE.lock().is_some()
}

/// Writes one pixel, doing nothing if it falls outside the surface.
///
/// # Safety
///
/// [`init`] must have been called with a valid, writable mapping.
unsafe fn put_pixel(surface: &Framebuffer, x: u32, y: u32, colour: Colour) {
    // `pixel_offset` is the single place the firmware-supplied stride and size
    // are trusted, and it bounds-checks both.
    let Some(offset) = surface.pixel_offset(x, y) else {
        return;
    };
    let bytes = colour.to_bytes(surface.format);
    // SAFETY: the offset is within `size`, which `init`'s caller guarantees is
    // mapped and writable.
    unsafe {
        let base = surface.base as *mut u8;
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), base.add(offset), 4);
    }
}

/// Fills a rectangle, clipped to the surface.
pub fn fill_rect(x: u32, y: u32, width: u32, height: u32, colour: Colour) {
    let guard = SURFACE.lock();
    let Some(surface) = guard.as_ref() else {
        return;
    };
    for row in y..y.saturating_add(height) {
        for column in x..x.saturating_add(width) {
            // SAFETY: the surface was validated at init and `put_pixel` bounds
            // checks every write against it.
            unsafe { put_pixel(surface, column, row, colour) };
        }
    }
}

/// Fills the whole surface.
pub fn clear(colour: Colour) {
    let (width, height) = {
        let guard = SURFACE.lock();
        match guard.as_ref() {
            Some(s) => (s.width, s.height),
            None => return,
        }
    };
    fill_rect(0, 0, width, height, colour);
}

/// Paints the LokoOS stop screen.
///
/// The `message` is not drawn — see the module docs — but it is taken so that
/// the signature does not change when glyph rendering lands, and so that every
/// caller is already passing the text it will need.
pub fn stop_screen(_message: &str) {
    let (width, height) = {
        let guard = SURFACE.lock();
        match guard.as_ref() {
            Some(s) => (s.width, s.height),
            None => return,
        }
    };

    clear(Colour::STOP_BACKGROUND);

    // A single horizontal accent bar at the upper third. Deliberately not a
    // loading bar or a spinner: nothing is in progress, and a moving indicator
    // would suggest otherwise.
    let bar_width = width / 3;
    let bar_height = (height / 120).max(3);
    fill_rect(
        width.saturating_sub(bar_width) / 2,
        height / 3,
        bar_width,
        bar_height,
        Colour::STOP_ACCENT,
    );
}
