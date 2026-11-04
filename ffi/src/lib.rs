//! The C interfaces of `cgba.h`, `cgb.h`, `cnes.h` and `csnes.h`, over the Rust cores.
//! This is the only crate that exports symbols and the only one that holds
//! unsafe code. A cargo feature selects each core.

// The contract of each function is the comment on its declaration in the
// header.
#![allow(clippy::missing_safety_doc)]

#[cfg(feature = "gb")]
mod gb;
#[cfg(feature = "gba")]
mod gba;
#[cfg(feature = "nes")]
mod nes;
#[cfg(feature = "snes")]
mod snes;

/// Copies `text` into the caller's error buffer and ends it with a NUL.
#[cfg(any(feature = "gb", feature = "gba", feature = "nes", feature = "snes"))]
fn write_error(error: *mut std::ffi::c_char, size: usize, text: &[u8]) {
    if error.is_null() || size == 0 {
        return;
    }
    let n = text.len().min(size - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), error.cast::<u8>(), n);
        *error.add(n) = 0;
    }
}
