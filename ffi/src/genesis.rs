//! The C interface of `cgenesis.h`.

use crate::write_error;
use oscilla_genesis::{Chip, EndState, Machine, Observer};
use oscilla_gym::Gym;
use oscilla_vgm::{MAX_FILE, SAMPLE_RATE, Vgm};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::{ptr, slice};

const OK: c_int = 0;
const IO: c_int = 1;
const FORMAT: c_int = 2;

/// A parsed file with its tag fields as C strings.
pub struct CImage {
    vgm: Vgm,
    title: CString,
    title_jp: CString,
    game: CString,
    game_jp: CString,
    system: CString,
    system_jp: CString,
    author: CString,
    author_jp: CString,
    date: CString,
    ripper: CString,
    notes: CString,
}

fn c_text(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

/// The format of a file, and for a GYM file whether it came from a PAL console.
#[derive(Clone, Copy)]
enum Source {
    Vgm,
    Gym { pal: bool },
}

fn make_image(
    bytes: &[u8],
    source: Source,
    error: *mut c_char,
    error_size: usize,
) -> Result<Box<CImage>, c_int> {
    let parsed = match source {
        Source::Vgm => Vgm::parse(bytes).map_err(|e| e.to_string()),
        Source::Gym { pal } => Gym::parse(bytes)
            .map_err(|e| e.to_string())
            .and_then(|g| Vgm::parse(&g.to_vgm(pal)).map_err(|e| e.to_string())),
    };
    match parsed {
        Ok(vgm) => {
            let tag = vgm.gd3.clone().unwrap_or_default();
            Ok(Box::new(CImage {
                title: c_text(&tag.title),
                title_jp: c_text(&tag.title_jp),
                game: c_text(&tag.game),
                game_jp: c_text(&tag.game_jp),
                system: c_text(&tag.system),
                system_jp: c_text(&tag.system_jp),
                author: c_text(&tag.author),
                author_jp: c_text(&tag.author_jp),
                date: c_text(&tag.date),
                ripper: c_text(&tag.ripper),
                notes: c_text(&tag.notes),
                vgm,
            }))
        }
        Err(e) => {
            write_error(error, error_size, e.as_bytes());
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

unsafe fn load(
    path: *const c_char,
    source: Source,
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
    match make_image(&bytes, source, error, error_size) {
        Ok(img) => {
            unsafe { *out = Box::into_raw(img) };
            OK
        }
        Err(code) => code,
    }
}

unsafe fn from_bytes(
    bytes: *const u8,
    size: usize,
    source: Source,
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
        let text: &[u8] = match source {
            Source::Vgm => b"Not a VGM file.",
            Source::Gym { .. } => b"Not a GYM file.",
        };
        write_error(error, error_size, text);
        return FORMAT;
    }
    if size > MAX_FILE {
        write_error(error, error_size, b"The file is larger than 64 MiB.");
        return FORMAT;
    }
    match make_image(
        unsafe { slice::from_raw_parts(bytes, size) },
        source,
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
pub unsafe extern "C" fn cgenesis_image_load_vgm(
    path: *const c_char,
    out: *mut *mut CImage,
    error: *mut c_char,
    error_size: usize,
) -> c_int {
    unsafe { load(path, Source::Vgm, out, error, error_size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_from_vgm(
    bytes: *const u8,
    size: usize,
    out: *mut *mut CImage,
    error: *mut c_char,
    error_size: usize,
) -> c_int {
    unsafe { from_bytes(bytes, size, Source::Vgm, out, error, error_size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_load_gym(
    path: *const c_char,
    pal: c_int,
    out: *mut *mut CImage,
    error: *mut c_char,
    error_size: usize,
) -> c_int {
    let source = Source::Gym { pal: pal != 0 };
    unsafe { load(path, source, out, error, error_size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_from_gym(
    bytes: *const u8,
    size: usize,
    pal: c_int,
    out: *mut *mut CImage,
    error: *mut c_char,
    error_size: usize,
) -> c_int {
    let source = Source::Gym { pal: pal != 0 };
    unsafe { from_bytes(bytes, size, source, out, error, error_size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_free(image: *mut CImage) {
    if !image.is_null() {
        drop(unsafe { Box::from_raw(image) });
    }
}

macro_rules! text_getter {
    ($name:ident, $field:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(image: *const CImage) -> *const c_char {
            if image.is_null() {
                return ptr::null();
            }
            unsafe { &*image }.$field.as_ptr()
        }
    };
}

text_getter!(cgenesis_image_title, title);
text_getter!(cgenesis_image_title_jp, title_jp);
text_getter!(cgenesis_image_game, game);
text_getter!(cgenesis_image_game_jp, game_jp);
text_getter!(cgenesis_image_system, system);
text_getter!(cgenesis_image_system_jp, system_jp);
text_getter!(cgenesis_image_author, author);
text_getter!(cgenesis_image_author_jp, author_jp);
text_getter!(cgenesis_image_date, date);
text_getter!(cgenesis_image_ripper, ripper);
text_getter!(cgenesis_image_notes, notes);

/// Bit 0 is the SN76489, bit 1 the YM2612 and bit 2 the YM2413.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_chips(image: *const CImage) -> u32 {
    if image.is_null() {
        return 0;
    }
    let v = &unsafe { &*image }.vgm;
    u32::from(v.chip(oscilla_vgm::SN76489).is_some())
        | u32::from(v.chip(oscilla_vgm::YM2612).is_some()) << 1
        | u32::from(v.chip(oscilla_vgm::YM2413).is_some()) << 2
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_total_samples(image: *const CImage) -> u32 {
    if image.is_null() {
        return 0;
    }
    unsafe { &*image }.vgm.total_samples
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_loop_samples(image: *const CImage) -> u32 {
    if image.is_null() {
        return 0;
    }
    unsafe { &*image }.vgm.loop_samples
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_has_loop(image: *const CImage) -> c_int {
    if image.is_null() {
        return 0;
    }
    c_int::from(unsafe { &*image }.vgm.loop_at().is_some())
}

/// Samples before the loop point at 44,100 Hz, or the whole file's length
/// when it has no loop.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_intro_samples(image: *const CImage) -> u32 {
    if image.is_null() {
        return 0;
    }
    let v = &unsafe { &*image }.vgm;
    if v.loop_at().is_some() {
        v.total_samples.saturating_sub(v.loop_samples)
    } else {
        v.total_samples
    }
}

fn to_rate(samples: u32, rate: c_int) -> i64 {
    if rate <= 0 {
        return -1;
    }
    (i64::from(samples) * i64::from(rate) + i64::from(SAMPLE_RATE) / 2) / i64::from(SAMPLE_RATE)
}

/// The intro in frames of `rate`, rounded to the nearest frame.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_intro_frames(image: *const CImage, rate: c_int) -> i64 {
    if image.is_null() {
        return -1;
    }
    to_rate(unsafe { cgenesis_image_intro_samples(image) }, rate)
}

/// The loop in frames of `rate`, or 0 when the file does not loop.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_loop_frames(image: *const CImage, rate: c_int) -> i64 {
    if image.is_null() {
        return -1;
    }
    if rate <= 0 {
        return -1;
    }
    let v = &unsafe { &*image }.vgm;
    if v.loop_at().is_none() {
        return 0;
    }
    to_rate(v.loop_samples, rate)
}

/// The passes through the loop that the file's loop base and modifier ask for
/// when a player would play `passes` of them. At least 1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_loop_count(image: *const CImage, passes: u32) -> u32 {
    if image.is_null() {
        return 1;
    }
    unsafe { &*image }.vgm.loop_count(passes)
}

/// The gain that the file's volume modifier asks for, as a multiplier. The
/// machine applies it to its output already.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_gain(image: *const CImage) -> f64 {
    if image.is_null() {
        return 1.0;
    }
    unsafe { &*image }.vgm.gain()
}

/// The frame rate of the console that the file was recorded from (50 or 60),
/// or 0 when the header does not say.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_image_frame_rate(image: *const CImage) -> u32 {
    if image.is_null() {
        return 0;
    }
    unsafe { &*image }.vgm.rate
}

// ---- Machines ---------------------------------------------------------

/// The sample that a running observer callback should see. A callback may call
/// `cgenesis_samples`, which then returns the time of the write, and no other
/// call that takes the machine does anything while a callback runs.
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
    /// The names of the voices, which the file fixes.
    names: Vec<CString>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CObserver {
    context: *mut c_void,
    register_write: Option<unsafe extern "C" fn(*mut c_void, c_int, u32, u32, u64)>,
}

struct Forward {
    observer: CObserver,
    callback: Arc<Callback>,
}

impl Observer for Forward {
    fn register_write(&mut self, chip: Chip, address: u16, value: u8, sample: u64) {
        if let Some(f) = self.observer.register_write {
            self.callback.sample.store(sample, Ordering::Relaxed);
            self.callback.active.store(true, Ordering::Relaxed);
            let chip = match chip {
                Chip::Sn76489 => 0,
                Chip::Sn76489Second => 1,
                Chip::Ym2612 => 2,
                Chip::Ym2612Second => 3,
                Chip::Ym2413 => 4,
                Chip::Ym2413Second => 5,
            };
            unsafe {
                f(
                    self.observer.context,
                    chip,
                    u32::from(address),
                    u32::from(value),
                    sample,
                )
            };
            self.callback.active.store(false, Ordering::Relaxed);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_new(image: *const CImage, output_rate: c_int) -> *mut CMachine {
    if image.is_null() {
        return ptr::null_mut();
    }
    match Machine::new(&unsafe { &*image }.vgm, output_rate) {
        Ok(machine) => Box::into_raw(Box::new(CMachine {
            names: (0..machine.voice_count())
                .filter_map(|i| machine.voice(i))
                .map(|v| c_text(&v.name))
                .collect(),
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
pub unsafe extern "C" fn cgenesis_free(machine: *mut CMachine) {
    if !machine.is_null() && !unsafe { in_callback(machine) } {
        drop(unsafe { Box::from_raw(machine) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_reset(machine: *mut CMachine) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.reset();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_render_s16(machine: *mut CMachine, out: *mut i16, frames: usize) {
    if unsafe { in_callback(machine) } || out.is_null() || frames > isize::MAX as usize / 8 {
        return;
    }
    let out = unsafe { slice::from_raw_parts_mut(out, frames * 2) };
    unsafe { &mut (*machine).machine }.render_s16(out);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_render_f32(machine: *mut CMachine, out: *mut f32, frames: usize) {
    if unsafe { in_callback(machine) } || out.is_null() || frames > isize::MAX as usize / 8 {
        return;
    }
    let out = unsafe { slice::from_raw_parts_mut(out, frames * 2) };
    unsafe { &mut (*machine).machine }.render_f32(out);
}

/// Time played so far, in samples at 44,100 Hz.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_samples(machine: *const CMachine) -> u64 {
    let callback = unsafe { &*ptr::addr_of!((*machine).callback) };
    if callback.active.load(Ordering::Relaxed) {
        return callback.sample.load(Ordering::Relaxed);
    }
    unsafe { &*ptr::addr_of!((*machine).machine) }.samples()
}

/// Passes through the loop point so far.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_loops(machine: *const CMachine) -> u32 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.loops()
}

/// 0 while the stream runs and 1 once it has ended without a loop.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_end_state_get(machine: *const CMachine) -> c_int {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    match unsafe { &(*machine).machine }.end_state() {
        EndState::Playing => 0,
        EndState::Finished => 1,
    }
}

/// One voice as a player shows it. The layout is the same in every core's
/// header (`csnes_voice`, `cpce_voice` and the others follow it).
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

fn make_voice(v: &oscilla_genesis::Voice) -> CVoice {
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

/// The number of voices that the file's chips have.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_voice_count(machine: *const CMachine) -> c_int {
    if machine.is_null() || unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.voice_count() as c_int
}

/// Writes at most `max` voices and returns how many.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_voices_get(
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

/// The name of voice `index`, which lives as long as the machine, or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_voice_name(
    machine: *const CMachine,
    index: c_int,
) -> *const c_char {
    if machine.is_null() || unsafe { in_callback(machine) } {
        return ptr::null();
    }
    let names = unsafe { &*ptr::addr_of!((*machine).names) };
    usize::try_from(index)
        .ok()
        .and_then(|i| names.get(i))
        .map_or(ptr::null(), |n| n.as_ptr())
}

/// Fills `out` with the last `frames` frames of a voice. Returns `frames`, or
/// 0 for a voice past the last, a NULL buffer, more than 16,384 frames or a
/// call from a callback.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_scope_get(
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_channel_levels_get(machine: *const CMachine, out: *mut i32) {
    if machine.is_null() || out.is_null() || unsafe { in_callback(machine) } {
        return;
    }
    let l = unsafe { &(*machine).machine }.channel_levels();
    unsafe { ptr::copy_nonoverlapping(l.as_ptr(), out, 10) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_set_channel_mask(machine: *mut CMachine, mask: u64) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.set_channel_mask(mask);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_channel_mask(machine: *const CMachine) -> u64 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.channel_mask()
}

/// Returns 1 and sets the cutoff, or returns 0 and changes nothing for a value
/// outside 0 to 1000 Hz.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_set_highpass_hz(machine: *mut CMachine, hz: f64) -> c_int {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    c_int::from(unsafe { &mut (*machine).machine }.set_highpass_hz(hz))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_highpass_hz(machine: *const CMachine) -> f64 {
    if unsafe { in_callback(machine) } {
        return 0.0;
    }
    unsafe { &(*machine).machine }.highpass_hz()
}

/// Copies the observer in. NULL stops observing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_set_observer(machine: *mut CMachine, observer: *const CObserver) {
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
pub unsafe extern "C" fn cgenesis_snapshot_take(machine: *const CMachine) -> *mut CSnapshot {
    if unsafe { in_callback(machine) } {
        return ptr::null_mut();
    }
    Box::into_raw(Box::new(CSnapshot(unsafe { &(*machine).machine }.clone())))
}

/// 0 on success and 2 when the snapshot is not from this machine's file and
/// rate.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgenesis_snapshot_restore(
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
pub unsafe extern "C" fn cgenesis_snapshot_free(snapshot: *mut CSnapshot) {
    if !snapshot.is_null() {
        drop(unsafe { Box::from_raw(snapshot) });
    }
}
