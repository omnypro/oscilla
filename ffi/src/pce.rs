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
    let Ok(song) = u8::try_from(song) else {
        return ptr::null_mut();
    };
    if image.is_null() {
        return ptr::null_mut();
    }
    match Machine::new(&unsafe { &*image }.hes, output_rate, song) {
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

/// One channel as a player shows it. The layout is the same in every core's
/// header (`csnes_voice`, `cgenesis_voice` and the others follow it).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CVoice {
    active: c_int,
    key_on: c_int,
    kind: c_int,
    instrument: c_int,
    frequency_hz: f32,
    volume: f32,
    pan: f32,
    envelope: f32,
    left: f32,
    right: f32,
    envelope_phase: c_int,
    pitch_modulated: c_int,
    echo: c_int,
    ended: c_int,
    mask_bit: c_int,
    reserved: [c_int; 3],
}

fn make_voice(index: usize, v: oscilla_pce::PsgVoice) -> CVoice {
    let (l, r) = (v.gain.0 as f32 / 65_536.0, v.gain.1 as f32 / 65_536.0);
    let period = if v.frequency == 0 {
        4096
    } else {
        u32::from(v.frequency)
    };
    let tone = !v.noise && !v.direct;
    CVoice {
        active: c_int::from(v.on && (l > 0.0 || r > 0.0)),
        key_on: c_int::from(v.on),
        kind: if v.noise {
            1
        } else if v.direct {
            2
        } else {
            0
        },
        instrument: if tone { v.wave_id as c_int } else { -1 },
        frequency_hz: if tone {
            3_579_545.0 / (32.0 * period as f32)
        } else {
            0.0
        },
        volume: l.max(r),
        pan: if l + r > 0.0 { (r - l) / (l + r) } else { 0.0 },
        envelope: -1.0,
        left: l,
        right: r,
        envelope_phase: 0,
        pitch_modulated: c_int::from(v.lfo),
        echo: 0,
        ended: 0,
        mask_bit: index as c_int,
        reserved: [0; 3],
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_voice_count(machine: *const CMachine) -> c_int {
    if machine.is_null() || unsafe { in_callback(machine) } {
        return 0;
    }
    6
}

/// Writes at most `max` voices and returns how many.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_voices_get(
    machine: *const CMachine,
    out: *mut CVoice,
    max: c_int,
) -> c_int {
    if machine.is_null() || out.is_null() || max <= 0 || unsafe { in_callback(machine) } {
        return 0;
    }
    let m = unsafe { &(*machine).machine };
    let n = max.min(6);
    for i in 0..n {
        unsafe { *out.add(i as usize) = make_voice(i as usize, m.voice_info(i as usize)) };
    }
    n
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_voice_name(machine: *const CMachine, index: c_int) -> *const c_char {
    const NAMES: [&CStr; 6] = [
        c"Wave 1",
        c"Wave 2",
        c"Wave 3",
        c"Wave 4",
        c"Wave/Noise 5",
        c"Wave/Noise 6",
    ];
    if machine.is_null() || unsafe { in_callback(machine) } {
        return ptr::null();
    }
    usize::try_from(index)
        .ok()
        .and_then(|i| NAMES.get(i))
        .map_or(ptr::null(), |n| n.as_ptr())
}

/// Fills `out` with the last `frames` frames of a channel. Returns `frames`, or
/// 0 for a channel outside 0 to 5, a NULL buffer, more than 16,384 frames or a
/// call from a callback.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cpce_scope_get(
    machine: *const CMachine,
    voice: c_int,
    out: *mut f32,
    frames: usize,
) -> usize {
    if machine.is_null()
        || out.is_null()
        || !(0..6).contains(&voice)
        || frames == 0
        || frames > 16_384
        || unsafe { in_callback(machine) }
    {
        return 0;
    }
    let out = unsafe { slice::from_raw_parts_mut(out, frames) };
    unsafe { &(*machine).machine }.scope(voice as usize, out);
    frames
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
