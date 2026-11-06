//! The C interface of `csnes.h`.

use crate::write_error;
use oscilla_snes::{EndState, Machine, Observer, RestoreError, Snapshot, Spc};
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
    spc: Spc,
    song: CString,
    game: CString,
    artist: CString,
    dumper: CString,
    comments: CString,
}

fn c_text(bytes: &[u8]) -> CString {
    // The bytes stop at the first NUL, so there is none inside.
    CString::new(bytes).unwrap_or_default()
}

fn make_image(bytes: &[u8], error: *mut c_char, error_size: usize) -> Result<Box<CImage>, c_int> {
    match Spc::parse(bytes) {
        Ok(spc) => Ok(Box::new(CImage {
            song: c_text(spc.song()),
            game: c_text(spc.game()),
            artist: c_text(spc.artist()),
            dumper: c_text(spc.dumper()),
            comments: c_text(spc.comments()),
            spc,
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
pub unsafe extern "C" fn csnes_image_load_spc(
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
        f.take(oscilla_snes::MAX_FILE as u64 + 1)
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
pub unsafe extern "C" fn csnes_image_from_spc(
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
        write_error(error, error_size, b"Not an SPC file.");
        return FORMAT;
    }
    if size > oscilla_snes::MAX_FILE {
        write_error(error, error_size, b"The file is larger than 1 MiB.");
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
pub unsafe extern "C" fn csnes_image_free(image: *mut CImage) {
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

text_getter!(csnes_image_song, song);
text_getter!(csnes_image_game, game);
text_getter!(csnes_image_artist, artist);
text_getter!(csnes_image_dumper, dumper);
text_getter!(csnes_image_comments, comments);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_image_time_ms(image: *const CImage) -> i64 {
    if image.is_null() {
        return -1;
    }
    unsafe { &*image }.spc.time_ms()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_image_fade_ms(image: *const CImage) -> i64 {
    if image.is_null() {
        return -1;
    }
    unsafe { &*image }.spc.fade_ms()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_image_length_ms(image: *const CImage) -> i64 {
    if image.is_null() {
        return -1;
    }
    let spc = &unsafe { &*image }.spc;
    match (spc.time_ms(), spc.fade_ms()) {
        (-1, _) => -1,
        (t, -1) => t,
        (t, f) => t + f,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_image_muted_voices(image: *const CImage) -> u32 {
    if image.is_null() {
        return 0;
    }
    u32::from(unsafe { &*image }.spc.muted())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_image_amplification(image: *const CImage) -> u32 {
    if image.is_null() {
        return 65536;
    }
    unsafe { &*image }.spc.xid6.amplification.unwrap_or(65536)
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
    dsp_write: Option<unsafe extern "C" fn(*mut c_void, u32, u32)>,
    port_write: Option<unsafe extern "C" fn(*mut c_void, u32, u32)>,
    ram_write: Option<unsafe extern "C" fn(*mut c_void, u32, u32)>,
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
    fn dsp_write(&mut self, cycle: u64, register: u8, value: u8) {
        if let Some(f) = self.observer.dsp_write {
            self.enter(cycle);
            unsafe { f(self.observer.context, u32::from(register), u32::from(value)) };
            self.leave();
        }
    }

    fn port_write(&mut self, cycle: u64, port: u8, value: u8) {
        if let Some(f) = self.observer.port_write {
            self.enter(cycle);
            unsafe { f(self.observer.context, u32::from(port), u32::from(value)) };
            self.leave();
        }
    }

    fn ram_write(&mut self, cycle: u64, address: u16, value: u8) {
        if let Some(f) = self.observer.ram_write {
            self.enter(cycle);
            unsafe { f(self.observer.context, u32::from(address), u32::from(value)) };
            self.leave();
        }
    }
}

fn wrap(machine: Option<Machine>) -> *mut CMachine {
    match machine {
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
pub unsafe extern "C" fn csnes_new(image: *const CImage, output_rate: c_int) -> *mut CMachine {
    if image.is_null() {
        return ptr::null_mut();
    }
    wrap(Machine::new(&unsafe { &*image }.spc, output_rate))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_free(machine: *mut CMachine) {
    if machine.is_null() || unsafe { in_callback(machine) } {
        return;
    }
    drop(unsafe { Box::from_raw(machine) });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_reset(machine: *mut CMachine) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.reset();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_render_s16(machine: *mut CMachine, out: *mut i16, frames: usize) {
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
pub unsafe extern "C" fn csnes_render_f32(machine: *mut CMachine, out: *mut f32, frames: usize) {
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
pub unsafe extern "C" fn csnes_cycles(machine: *const CMachine) -> u64 {
    let callback = unsafe { &*ptr::addr_of!((*machine).callback) };
    if callback.active.load(Ordering::Relaxed) {
        return callback.cycle.load(Ordering::Relaxed);
    }
    unsafe { &*ptr::addr_of!((*machine).machine) }.cycles()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_samples(machine: *const CMachine) -> u64 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.samples()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_set_observer(machine: *mut CMachine, observer: *const CObserver) {
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
pub unsafe extern "C" fn csnes_watch_ram(machine: *mut CMachine, first: u32, last: u32) {
    if unsafe { in_callback(machine) } {
        return;
    }
    let m = unsafe { &mut (*machine).machine };
    if first > 0xFFFF || first > last {
        m.watch_ram(1, 0);
    } else {
        m.watch_ram(first as u16, last.min(0xFFFF) as u16);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_run_cycles(machine: *mut CMachine, cycles: u64) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.run_cycles(cycles);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_register(machine: *const CMachine, index: c_int) -> u32 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    let (pc, a, x, y, psw, sp) = unsafe { &(*machine).machine }.cpu_state();
    match index {
        0 => u32::from(a),
        1 => u32::from(x),
        2 => u32::from(y),
        3 => u32::from(psw),
        4 => u32::from(sp),
        5 => u32::from(pc),
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_peek(machine: *const CMachine, address: u32) -> u32 {
    if unsafe { in_callback(machine) } || address > 0xFFFF {
        return 0;
    }
    u32::from(unsafe { &(*machine).machine }.peek(address as u16))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_ram_copy(machine: *const CMachine, out: *mut u8) {
    if unsafe { in_callback(machine) } || out.is_null() {
        return;
    }
    let ram = unsafe { &(*machine).machine }.ram();
    unsafe { std::ptr::copy_nonoverlapping(ram.as_ptr(), out, ram.len()) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_dsp_register(machine: *const CMachine, register: u32) -> u32 {
    if unsafe { in_callback(machine) } || register > 0xFF {
        return 0;
    }
    u32::from(unsafe { &(*machine).machine }.dsp_register(register as u8))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_write_port(machine: *mut CMachine, port: c_int, value: u32) {
    if unsafe { in_callback(machine) } || !(0..4).contains(&port) {
        return;
    }
    unsafe { &mut (*machine).machine }.write_port(port as usize, value as u8);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_read_port(machine: *const CMachine, port: c_int) -> u32 {
    if unsafe { in_callback(machine) } || !(0..4).contains(&port) {
        return 0;
    }
    u32::from(unsafe { &(*machine).machine }.read_port(port as usize))
}

// ---- Snapshots, voices and the end of a song ---------------------------

pub struct CSnapshot(Snapshot);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_snapshot_take(machine: *const CMachine) -> *mut CSnapshot {
    if unsafe { in_callback(machine) } {
        return ptr::null_mut();
    }
    Box::into_raw(Box::new(CSnapshot(
        unsafe { &(*machine).machine }.snapshot(),
    )))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_snapshot_restore(
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
pub unsafe extern "C" fn csnes_snapshot_size(snapshot: *const CSnapshot) -> usize {
    if snapshot.is_null() {
        return 0;
    }
    unsafe { &*snapshot }.0.size()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_snapshot_free(snapshot: *mut CSnapshot) {
    if !snapshot.is_null() {
        drop(unsafe { Box::from_raw(snapshot) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_voice_levels_get(machine: *const CMachine, out: *mut i32) {
    if out.is_null() || unsafe { in_callback(machine) } {
        return;
    }
    let l = unsafe { &(*machine).machine }.voice_levels();
    unsafe { ptr::copy_nonoverlapping(l.as_ptr(), out, 8) };
}

/// One voice as a player shows it. The layout is the same in every core's
/// header (`cgenesis_voice`, `cpce_voice` and the others follow it), so a viewer
/// can treat them alike.
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
    reserved: [c_int; 4],
}

fn make_voice(v: oscilla_snes::VoiceInfo) -> CVoice {
    let envelope = v.envelope as f32 / 2047.0;
    let (l, r) = (f32::from(v.volume.0) / 128.0, f32::from(v.volume.1) / 128.0);
    let loud = l.abs().max(r.abs());
    CVoice {
        active: c_int::from(v.envelope > 0),
        key_on: c_int::from(v.phase < 3),
        kind: if v.noise { 1 } else { 2 },
        instrument: c_int::from(v.source),
        frequency_hz: if v.noise {
            0.0
        } else {
            32_000.0 * f32::from(v.pitch) / 4096.0
        },
        volume: loud * envelope,
        pan: if l.abs() + r.abs() > 0.0 {
            (r.abs() - l.abs()) / (l.abs() + r.abs())
        } else {
            0.0
        },
        envelope,
        left: l * envelope,
        right: r * envelope,
        envelope_phase: c_int::from(v.phase),
        pitch_modulated: c_int::from(v.pitch_modulated),
        echo: c_int::from(v.echo),
        ended: c_int::from(v.ended),
        reserved: [0; 4],
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_voice_count(machine: *const CMachine) -> c_int {
    if machine.is_null() || unsafe { in_callback(machine) } {
        return 0;
    }
    8
}

/// Writes at most `max` voices and returns how many.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_voices_get(
    machine: *const CMachine,
    out: *mut CVoice,
    max: c_int,
) -> c_int {
    if machine.is_null() || out.is_null() || max <= 0 || unsafe { in_callback(machine) } {
        return 0;
    }
    let m = unsafe { &(*machine).machine };
    let n = max.min(8);
    for i in 0..n {
        unsafe { *out.add(i as usize) = make_voice(m.voice_info(i as usize)) };
    }
    n
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_voice_name(machine: *const CMachine, index: c_int) -> *const c_char {
    const NAMES: [&CStr; 8] = [
        c"Voice 1", c"Voice 2", c"Voice 3", c"Voice 4", c"Voice 5", c"Voice 6", c"Voice 7",
        c"Voice 8",
    ];
    if machine.is_null() || unsafe { in_callback(machine) } {
        return ptr::null();
    }
    usize::try_from(index)
        .ok()
        .and_then(|i| NAMES.get(i))
        .map_or(ptr::null(), |n| n.as_ptr())
}

/// Fills `out` with the last `frames` frames of a voice. Returns `frames`, or 0
/// for a voice outside 0 to 7, a NULL buffer, more than 16,384 frames or a call
/// from a callback.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_scope_get(
    machine: *const CMachine,
    voice: c_int,
    out: *mut f32,
    frames: usize,
) -> usize {
    if machine.is_null()
        || out.is_null()
        || !(0..8).contains(&voice)
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
pub unsafe extern "C" fn csnes_set_access_tracking(machine: *mut CMachine, on: c_int) {
    if machine.is_null() || unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.set_access_tracking(on != 0);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_access_tracking(machine: *const CMachine) -> c_int {
    if machine.is_null() || unsafe { in_callback(machine) } {
        return 0;
    }
    c_int::from(unsafe { &(*machine).machine }.access_tracking())
}

/// Copies the map and returns 1, or fills `out` with zeros and returns 0 when
/// tracking is off.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_access_map_get(machine: *const CMachine, out: *mut u8) -> c_int {
    if machine.is_null() || out.is_null() || unsafe { in_callback(machine) } {
        return 0;
    }
    match unsafe { &(*machine).machine }.access_map() {
        Some(map) => {
            unsafe { ptr::copy_nonoverlapping(map.as_ptr(), out, 65536) };
            1
        }
        None => {
            unsafe { ptr::write_bytes(out, 0, 65536) };
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_access_map_clear(machine: *mut CMachine) {
    if machine.is_null() || unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.clear_access_map();
}

/// Copies the map and clears it, so that each call returns the accesses since
/// the one before. Returns 1, or 0 with zeros when tracking is off.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_access_map_take(machine: *mut CMachine, out: *mut u8) -> c_int {
    let got = unsafe { csnes_access_map_get(machine, out) };
    if got == 1 {
        unsafe { &mut (*machine).machine }.clear_access_map();
    }
    got
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_set_channel_mask(machine: *mut CMachine, mask: u64) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.set_channel_mask(mask);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_channel_mask(machine: *const CMachine) -> u64 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.channel_mask()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_end_state_get(machine: *const CMachine) -> c_int {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    match unsafe { &(*machine).machine }.end_state() {
        EndState::Running => 0,
        EndState::Stalled => 1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn csnes_quiet_cycles(machine: *const CMachine) -> u64 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.quiet_cycles()
}
