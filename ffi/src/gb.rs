//! The C interface of `cgb.h`.

use crate::write_error;
use oscilla_gb::{EndState, Machine, Observer, RestoreError, Snapshot};
use oscilla_gbs::Gbs;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::{ptr, slice};

const OK: c_int = 0;
const IO: c_int = 1;
const FORMAT: c_int = 2;

/// A parsed file with its text fields as C strings.
pub struct CImage {
    gbs: Gbs,
    title: CString,
    author: CString,
    copyright: CString,
}

fn c_text(bytes: &[u8]) -> CString {
    // The bytes stop at the first NUL, so there is none inside.
    CString::new(bytes).unwrap_or_default()
}

fn make_image(bytes: &[u8], error: *mut c_char, error_size: usize) -> Result<Box<CImage>, c_int> {
    match Gbs::parse(bytes) {
        Ok(gbs) => Ok(Box::new(CImage {
            title: c_text(gbs.title()),
            author: c_text(gbs.author()),
            copyright: c_text(gbs.copyright()),
            gbs,
        })),
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
pub unsafe extern "C" fn cgb_image_load_gbs(
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
    let read = open(path).and_then(|f| {
        f.take(oscilla_gbs::MAX_FILE as u64 + 1)
            .read_to_end(&mut bytes)
    });
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
pub unsafe extern "C" fn cgb_image_from_gbs(
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
        write_error(error, error_size, b"Not a GBS file.");
        return FORMAT;
    }
    if size > oscilla_gbs::MAX_FILE {
        write_error(error, error_size, b"The file is larger than 4 MiB.");
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
pub unsafe extern "C" fn cgb_image_free(image: *mut CImage) {
    if !image.is_null() {
        drop(unsafe { Box::from_raw(image) });
    }
}

#[repr(C)]
pub struct CHeader {
    songs: c_int,
    first_song: c_int,
    load: u16,
    init: u16,
    play: u16,
    sp: u16,
    tma: u8,
    tac: u8,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_image_header(image: *const CImage, out: *mut CHeader) {
    if image.is_null() || out.is_null() {
        return;
    }
    let g = &unsafe { &*image }.gbs;
    unsafe {
        *out = CHeader {
            songs: c_int::from(g.songs),
            first_song: c_int::from(g.first_song),
            load: g.load,
            init: g.init,
            play: g.play,
            sp: g.sp,
            tma: g.tma,
            tac: g.tac,
        }
    };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_image_title(image: *const CImage) -> *const c_char {
    if image.is_null() {
        return ptr::null();
    }
    unsafe { &*image }.title.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_image_author(image: *const CImage) -> *const c_char {
    if image.is_null() {
        return ptr::null();
    }
    unsafe { &*image }.author.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_image_copyright(image: *const CImage) -> *const c_char {
    if image.is_null() {
        return ptr::null();
    }
    unsafe { &*image }.copyright.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_image_tick_clocks(image: *const CImage) -> u32 {
    if image.is_null() {
        return 0;
    }
    unsafe { &*image }.gbs.tick_clocks()
}

// ---- Machines ---------------------------------------------------------

/// The cycle that a running observer callback should see; see the same type
/// in the GBA interface.
struct Callback {
    active: AtomicBool,
    cycle: AtomicU64,
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
    sound_write: Option<unsafe extern "C" fn(*mut c_void, u32, u32)>,
    play_call: Option<unsafe extern "C" fn(*mut c_void, i64)>,
}

struct Forward {
    observer: CObserver,
    callback: Arc<Callback>,
}

impl Forward {
    fn enter(&self, cycle: u64) {
        self.callback.cycle.store(cycle, Ordering::Relaxed);
        self.callback.active.store(true, Ordering::Relaxed);
    }

    fn leave(&self) {
        self.callback.active.store(false, Ordering::Relaxed);
    }
}

impl Observer for Forward {
    fn sound_write(&mut self, cycle: u64, address: u16, value: u8) {
        if let Some(f) = self.observer.sound_write {
            self.enter(cycle);
            unsafe { f(self.observer.context, u32::from(address), u32::from(value)) };
            self.leave();
        }
    }

    fn play_call(&mut self, cycle: u64, frame: i64) {
        if let Some(f) = self.observer.play_call {
            self.enter(cycle);
            unsafe { f(self.observer.context, frame) };
            self.leave();
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_new(image: *const CImage, output_rate: c_int) -> *mut CMachine {
    if image.is_null() {
        return ptr::null_mut();
    }
    match Machine::new(&unsafe { &*image }.gbs, output_rate) {
        Some(machine) => Box::into_raw(Box::new(CMachine {
            machine,
            callback: Arc::new(Callback {
                active: AtomicBool::new(false),
                cycle: AtomicU64::new(0),
            }),
        })),
        None => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_free(machine: *mut CMachine) {
    if machine.is_null() || unsafe { in_callback(machine) } {
        return;
    }
    drop(unsafe { Box::from_raw(machine) });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_select_song(machine: *mut CMachine, song: c_int) -> c_int {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    match u8::try_from(song) {
        Ok(n) => c_int::from(unsafe { &mut (*machine).machine }.select_song(n)),
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_reset(machine: *mut CMachine) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.reset();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_song(machine: *const CMachine) -> c_int {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    c_int::from(unsafe { &(*machine).machine }.song())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_render_s16(machine: *mut CMachine, out: *mut i16, frames: usize) {
    if unsafe { in_callback(machine) } {
        return;
    }
    if out.is_null() || frames > isize::MAX as usize / 8 {
        return;
    }
    let out = unsafe { slice::from_raw_parts_mut(out, frames * 2) };
    unsafe { &mut (*machine).machine }.render_s16(out);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_render_f32(machine: *mut CMachine, out: *mut f32, frames: usize) {
    if unsafe { in_callback(machine) } {
        return;
    }
    if out.is_null() || frames > isize::MAX as usize / 8 {
        return;
    }
    let out = unsafe { slice::from_raw_parts_mut(out, frames * 2) };
    unsafe { &mut (*machine).machine }.render_f32(out);
}

/// Callable from an observer callback while a render is running, so it reads
/// the machine only when no callback is active, and otherwise only the cell
/// that holds the callback's cycle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_cycles(machine: *const CMachine) -> u64 {
    let callback = unsafe { &*ptr::addr_of!((*machine).callback) };
    if callback.active.load(Ordering::Relaxed) {
        return callback.cycle.load(Ordering::Relaxed);
    }
    unsafe { &*ptr::addr_of!((*machine).machine) }.cycles()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_frames(machine: *const CMachine) -> u64 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.frames()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_set_observer(machine: *mut CMachine, observer: *const CObserver) {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_run_cycles(machine: *mut CMachine, cycles: u64) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.run_cycles(cycles);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_register(machine: *const CMachine, index: c_int) -> u32 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.register(index)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_next_pc(machine: *const CMachine) -> u32 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    u32::from(unsafe { &(*machine).machine }.next_pc())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_peek(machine: *const CMachine, address: u32) -> u32 {
    if unsafe { in_callback(machine) } || address > 0xFFFF {
        return 0;
    }
    u32::from(unsafe { &(*machine).machine }.peek(address as u16))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_warnings(machine: *const CMachine) -> *const c_char {
    if unsafe { in_callback(machine) } {
        return c"".as_ptr();
    }
    unsafe { &(*machine).machine }
        .warnings_nul()
        .as_ptr()
        .cast()
}

// ---- Snapshots, channels and the end of a song -------------------------

pub struct CSnapshot(Snapshot);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_snapshot_take(machine: *const CMachine) -> *mut CSnapshot {
    if unsafe { in_callback(machine) } {
        return ptr::null_mut();
    }
    Box::into_raw(Box::new(CSnapshot(
        unsafe { &(*machine).machine }.snapshot(),
    )))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_snapshot_restore(
    machine: *mut CMachine,
    snapshot: *const CSnapshot,
) -> c_int {
    if unsafe { in_callback(machine) } {
        return 3;
    }
    if snapshot.is_null() {
        return 2;
    }
    match unsafe { &mut (*machine).machine }.restore(&unsafe { &*snapshot }.0) {
        Ok(()) => 0,
        Err(RestoreError::Rate) => 1,
        Err(RestoreError::Image) => 2,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_snapshot_size(snapshot: *const CSnapshot) -> usize {
    if snapshot.is_null() {
        return 0;
    }
    unsafe { &*snapshot }.0.size()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_snapshot_free(snapshot: *mut CSnapshot) {
    if !snapshot.is_null() {
        drop(unsafe { Box::from_raw(snapshot) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_channel_levels_get(machine: *const CMachine, out: *mut i32) {
    if out.is_null() || unsafe { in_callback(machine) } {
        return;
    }
    let l = unsafe { &(*machine).machine }.channel_levels();
    unsafe { ptr::copy_nonoverlapping(l.as_ptr(), out, 4) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_set_channel_mask(machine: *mut CMachine, mask: u32) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.set_channel_mask(mask as u8);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_channel_mask(machine: *const CMachine) -> u32 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    u32::from(unsafe { &(*machine).machine }.channel_mask())
}

// ---- Voices and scopes -------------------------------------------------

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

fn make_voice(v: &oscilla_gb::Voice) -> CVoice {
    let sides = v.left + v.right;
    CVoice {
        active: c_int::from(v.active),
        key_on: c_int::from(v.key_on),
        kind: v.kind as c_int,
        instrument: v.instrument,
        frequency_hz: v.frequency_hz,
        volume: v.volume,
        pan: if sides > 0.0 {
            (v.right - v.left) / sides
        } else {
            0.0
        },
        envelope: v.envelope,
        left: v.left,
        right: v.right,
        envelope_phase: v.phase,
        pitch_modulated: 0,
        echo: 0,
        ended: 0,
        mask_bit: v.mask_bit as c_int,
        reserved: [0; 3],
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_voice_count(machine: *const CMachine) -> c_int {
    if machine.is_null() || unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.voice_count() as c_int
}

/// Writes at most `max` voices and returns how many.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_voices_get(
    machine: *const CMachine,
    out: *mut CVoice,
    max: c_int,
) -> c_int {
    if machine.is_null() || out.is_null() || max <= 0 || unsafe { in_callback(machine) } {
        return 0;
    }
    let m = unsafe { &(*machine).machine };
    let mut n = 0;
    for i in 0..(max as usize).min(m.voice_count()) {
        if let Some(v) = m.voice(i) {
            unsafe { *out.add(i) = make_voice(&v) };
            n += 1;
        }
    }
    n
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_voice_name(machine: *const CMachine, index: c_int) -> *const c_char {
    const NAMES: [&CStr; 4] = [c"Pulse 1", c"Pulse 2", c"Wave", c"Noise"];
    if machine.is_null() || unsafe { in_callback(machine) } {
        return ptr::null();
    }
    usize::try_from(index)
        .ok()
        .and_then(|i| NAMES.get(i))
        .map_or(ptr::null(), |n| n.as_ptr())
}

/// Fills `out` with the last `frames` frames of a channel. Returns `frames`, or
/// 0 for a voice outside 0 to 3, a NULL buffer, 0 frames, more than 16,384
/// frames or a call from a callback.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_scope_get(
    machine: *const CMachine,
    voice: c_int,
    out: *mut f32,
    frames: usize,
) -> usize {
    if machine.is_null()
        || out.is_null()
        || voice < 0
        || frames == 0
        || frames > 16_384
        || unsafe { in_callback(machine) }
    {
        return 0;
    }
    let out = unsafe { slice::from_raw_parts_mut(out, frames) };
    if unsafe { &(*machine).machine }.scope(voice as usize, out) {
        frames
    } else {
        0
    }
}

/// 1 when the cutoff is in range and applied, 0 otherwise (also from inside a
/// callback).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_set_highpass_hz(machine: *mut CMachine, hz: f64) -> c_int {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    c_int::from(unsafe { &mut (*machine).machine }.set_highpass_hz(hz))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_use_console_highpass(machine: *mut CMachine) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.use_console_highpass();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_highpass_hz(machine: *const CMachine) -> f64 {
    if unsafe { in_callback(machine) } {
        return 0.0;
    }
    unsafe { &(*machine).machine }.highpass_hz()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_end_state_get(machine: *const CMachine) -> c_int {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    match unsafe { &(*machine).machine }.end_state() {
        EndState::Running => 0,
        EndState::ChannelsOff => 1,
        EndState::Stalled => 2,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgb_quiet_cycles(machine: *const CMachine) -> u64 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.quiet_cycles()
}
