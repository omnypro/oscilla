//! The C interface of `cpce.h`.

use crate::write_error;
use oscilla_hes::{Hes, MAX_FILE};
use oscilla_pce::{Machine, Observer};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::{ptr, slice};

const OK: c_int = 0;
const IO: c_int = 1;
const FORMAT: c_int = 2;

/// A parsed file.
pub struct CImage {
    hes: Hes,
}

fn make_image(bytes: &[u8], error: *mut c_char, error_size: usize) -> Result<Box<CImage>, c_int> {
    match Hes::parse(bytes) {
        Ok(hes) => Ok(Box::new(CImage { hes })),
        Err(e) => {
            write_error(error, error_size, e.to_string().as_bytes());
            Err(FORMAT)
        }
    }
}

#[cfg(unix)]
fn open(path: &[u8]) -> std::io::Result<std::fs::File> {
    use std::os::unix::ffi::OsStrExt;
    std::fs::File::open(std::ffi::OsStr::from_bytes(path))
}

#[cfg(not(unix))]
fn open(path: &[u8]) -> std::io::Result<std::fs::File> {
    match std::str::from_utf8(path) {
        Ok(text) => std::fs::File::open(text),
        Err(_) => Err(std::io::ErrorKind::InvalidInput.into()),
    }
}

#[cfg(unix)]
fn is_regular_file(path: &[u8]) -> bool {
    use std::os::unix::ffi::OsStrExt;
    std::fs::metadata(std::ffi::OsStr::from_bytes(path)).is_ok_and(|m| m.is_file())
}

#[cfg(not(unix))]
fn is_regular_file(path: &[u8]) -> bool {
    std::str::from_utf8(path).is_ok_and(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_image_load_hes(
    path: *const c_char,
    out: *mut *mut CImage,
    error: *mut c_char,
    error_size: usize,
) -> c_int {
    if out.is_null() {
        return IO;
    }
    unsafe { *out = ptr::null_mut() };
    write_error(error, error_size, b"");
    if path.is_null() {
        return IO;
    }
    let path = unsafe { CStr::from_ptr(path) }.to_bytes();
    // A pipe or a device would block or never end, so only a regular file loads.
    if !is_regular_file(path) {
        write_error(error, error_size, b"Not a regular file.");
        return IO;
    }
    let mut bytes = Vec::new();
    // Reads one byte more than the largest file, so that a larger one fails
    // the size check and no file allocates without a limit.
    let read = open(path).and_then(|f| f.take(MAX_FILE as u64 + 1).read_to_end(&mut bytes));
    if read.is_err() {
        write_error(error, error_size, b"Could not read the file.");
        return IO;
    }
    match make_image(&bytes, error, error_size) {
        Ok(img) => {
            unsafe { *out = Box::into_raw(img) };
            OK
        }
        Err(code) => code,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_image_from_hes(
    bytes: *const u8,
    size: usize,
    out: *mut *mut CImage,
    error: *mut c_char,
    error_size: usize,
) -> c_int {
    if out.is_null() {
        return IO;
    }
    unsafe { *out = ptr::null_mut() };
    write_error(error, error_size, b"");
    if bytes.is_null() || size == 0 {
        write_error(error, error_size, b"Not an HES file.");
        return FORMAT;
    }
    if size > MAX_FILE {
        write_error(error, error_size, b"The file is larger than 16 MiB.");
        return FORMAT;
    }
    match make_image(
        unsafe { slice::from_raw_parts(bytes, size) },
        error,
        error_size,
    ) {
        Ok(img) => {
            unsafe { *out = Box::into_raw(img) };
            OK
        }
        Err(code) => code,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_image_free(image: *mut CImage) {
    if !image.is_null() {
        drop(unsafe { Box::from_raw(image) });
    }
}

/// The song that the header names as the first.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_image_start_song(image: *const CImage) -> u32 {
    if image.is_null() {
        return 0;
    }
    u32::from(unsafe { &*image }.hes.start_song)
}

// ---- Machines ---------------------------------------------------------

/// The sample that a running observer callback should see. A callback may call
/// `cpce_samples`, which then returns the time of the store, and no other call
/// that takes the machine does anything while a callback runs.
struct Callback {
    active: AtomicBool,
    sample: AtomicU64,
}

unsafe fn in_callback(machine: *const CMachine) -> bool {
    unsafe { &*ptr::addr_of!((*machine).callback) }
        .active
        .load(Ordering::Relaxed)
}

pub struct CMachine {
    machine: Machine,
    callback: Arc<Callback>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CObserver {
    context: *mut c_void,
    psg_write: Option<unsafe extern "C" fn(*mut c_void, c_int, u32, u32, u64)>,
}

struct Forward {
    observer: CObserver,
    callback: Arc<Callback>,
}

impl Observer for Forward {
    fn psg_write(&mut self, channel: u8, register: u8, value: u8, sample: u64) {
        if let Some(f) = self.observer.psg_write {
            self.callback.sample.store(sample, Ordering::Relaxed);
            self.callback.active.store(true, Ordering::Relaxed);
            unsafe {
                f(
                    self.observer.context,
                    c_int::from(channel),
                    u32::from(register),
                    u32::from(value),
                    sample,
                )
            };
            self.callback.active.store(false, Ordering::Relaxed);
        }
    }
}

/// Starts the program at `song`. Returns NULL for an output rate outside 8000
/// to 192000 Hz.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_new(
    image: *const CImage,
    output_rate: c_int,
    song: c_int,
) -> *mut CMachine {
    if image.is_null() {
        return ptr::null_mut();
    }
    match Machine::new(&unsafe { &*image }.hes, output_rate, song as u8) {
        Ok(machine) => Box::into_raw(Box::new(CMachine {
            machine,
            callback: Arc::new(Callback {
                active: AtomicBool::new(false),
                sample: AtomicU64::new(0),
            }),
        })),
        Err(_) => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_free(machine: *mut CMachine) {
    if !machine.is_null() && !unsafe { in_callback(machine) } {
        drop(unsafe { Box::from_raw(machine) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_reset(machine: *mut CMachine) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.reset();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_render_s16(machine: *mut CMachine, out: *mut i16, frames: usize) {
    if unsafe { in_callback(machine) } || out.is_null() || frames > isize::MAX as usize / 8 {
        return;
    }
    let out = unsafe { slice::from_raw_parts_mut(out, frames * 2) };
    unsafe { &mut (*machine).machine }.render_s16(out);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_render_f32(machine: *mut CMachine, out: *mut f32, frames: usize) {
    if unsafe { in_callback(machine) } || out.is_null() || frames > isize::MAX as usize / 8 {
        return;
    }
    let out = unsafe { slice::from_raw_parts_mut(out, frames * 2) };
    unsafe { &mut (*machine).machine }.render_f32(out);
}

/// Time played so far, in samples at 44,100 Hz.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_samples(machine: *const CMachine) -> u64 {
    let callback = unsafe { &*ptr::addr_of!((*machine).callback) };
    if callback.active.load(Ordering::Relaxed) {
        return callback.sample.load(Ordering::Relaxed);
    }
    unsafe { &*ptr::addr_of!((*machine).machine) }.samples()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_channel_levels_get(machine: *const CMachine, out: *mut i32) {
    if machine.is_null() || out.is_null() || unsafe { in_callback(machine) } {
        return;
    }
    let l = unsafe { &(*machine).machine }.channel_levels();
    unsafe { ptr::copy_nonoverlapping(l.as_ptr(), out, 6) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_set_channel_mask(machine: *mut CMachine, mask: u64) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.set_channel_mask(mask);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_channel_mask(machine: *const CMachine) -> u64 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.channel_mask()
}

/// Returns 1 and sets the cutoff, or returns 0 and changes nothing for a value
/// outside 0 to 1000 Hz.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_set_highpass_hz(machine: *mut CMachine, hz: f64) -> c_int {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    c_int::from(unsafe { &mut (*machine).machine }.set_highpass_hz(hz))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_highpass_hz(machine: *const CMachine) -> f64 {
    if unsafe { in_callback(machine) } {
        return 0.0;
    }
    unsafe { &(*machine).machine }.highpass_hz()
}

/// Copies the observer in. NULL stops observing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_set_observer(machine: *mut CMachine, observer: *const CObserver) {
    if unsafe { in_callback(machine) } {
        return;
    }
    let m = unsafe { &mut *machine };
    if observer.is_null() {
        m.machine.set_observer(None);
    } else {
        m.machine.set_observer(Some(Box::new(Forward {
            observer: unsafe { *observer },
            callback: Arc::clone(&m.callback),
        })));
    }
}

// ---- Snapshots --------------------------------------------------------

pub struct CSnapshot(Machine);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_snapshot_take(machine: *const CMachine) -> *mut CSnapshot {
    if unsafe { in_callback(machine) } {
        return ptr::null_mut();
    }
    Box::into_raw(Box::new(CSnapshot(unsafe { &(*machine).machine }.clone())))
}

/// 0 on success, 2 when the snapshot is not from this machine's file, song and
/// rate (or is NULL), and 3 when called from an observer callback.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_snapshot_restore(
    machine: *mut CMachine,
    snapshot: *const CSnapshot,
) -> c_int {
    if unsafe { in_callback(machine) } {
        return 3;
    }
    if snapshot.is_null() {
        return 2;
    }
    if unsafe { &mut (*machine).machine }.restore(&unsafe { &*snapshot }.0) {
        0
    } else {
        2
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_snapshot_free(snapshot: *mut CSnapshot) {
    if !snapshot.is_null() {
        drop(unsafe { Box::from_raw(snapshot) });
    }
}
