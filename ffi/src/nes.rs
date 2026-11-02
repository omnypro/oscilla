//! The C interface of `cnes.h`.

use crate::write_error;
use oscilla_nes::{EndState, Machine, Observer, Region, RestoreError, Snapshot};
use oscilla_nsf::Nsf;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::{ptr, slice};

const OK: c_int = 0;
const IO: c_int = 1;
const FORMAT: c_int = 2;

const NTSC: c_int = 0;
const PAL: c_int = 1;

/// A parsed file with its text fields as C strings.
pub struct CImage {
    nsf: Nsf,
    title: CString,
    author: CString,
    copyright: CString,
}

fn c_text(bytes: &[u8]) -> CString {
    // The bytes stop at the first NUL, so there is none inside.
    CString::new(bytes).unwrap_or_default()
}

fn make_image(bytes: &[u8], error: *mut c_char, error_size: usize) -> Result<Box<CImage>, c_int> {
    match Nsf::parse(bytes) {
        Ok(nsf) => Ok(Box::new(CImage {
            title: c_text(nsf.title()),
            author: c_text(nsf.author()),
            copyright: c_text(nsf.copyright()),
            nsf,
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
pub unsafe extern "C" fn cnes_image_load_nsf(
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
        f.take(oscilla_nsf::MAX_FILE as u64 + 1)
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
pub unsafe extern "C" fn cnes_image_from_nsf(
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
        write_error(error, error_size, b"Not an NSF file.");
        return FORMAT;
    }
    if size > oscilla_nsf::MAX_FILE {
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
pub unsafe extern "C" fn cnes_image_free(image: *mut CImage) {
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
    ntsc_us: u16,
    pal_us: u16,
    banks: [u8; 8],
    region_bits: u8,
    chips: u8,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_image_header(image: *const CImage, out: *mut CHeader) {
    if image.is_null() || out.is_null() {
        return;
    }
    let n = &unsafe { &*image }.nsf;
    unsafe {
        *out = CHeader {
            songs: c_int::from(n.songs),
            first_song: c_int::from(n.first_song),
            load: n.load,
            init: n.init,
            play: n.play,
            ntsc_us: n.ntsc_us,
            pal_us: n.pal_us,
            banks: n.banks,
            region_bits: n.region_bits,
            chips: n.chips,
        }
    };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_image_title(image: *const CImage) -> *const c_char {
    if image.is_null() {
        return ptr::null();
    }
    unsafe { &*image }.title.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_image_author(image: *const CImage) -> *const c_char {
    if image.is_null() {
        return ptr::null();
    }
    unsafe { &*image }.author.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_image_copyright(image: *const CImage) -> *const c_char {
    if image.is_null() {
        return ptr::null();
    }
    unsafe { &*image }.copyright.as_ptr()
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
pub unsafe extern "C" fn cnes_new(image: *const CImage, output_rate: c_int) -> *mut CMachine {
    if image.is_null() {
        return ptr::null_mut();
    }
    wrap(Machine::new(&unsafe { &*image }.nsf, output_rate))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_new_for(
    image: *const CImage,
    output_rate: c_int,
    region: c_int,
) -> *mut CMachine {
    if image.is_null() {
        return ptr::null_mut();
    }
    let region = match region {
        NTSC => Region::Ntsc,
        PAL => Region::Pal,
        _ => return ptr::null_mut(),
    };
    wrap(Machine::new_for(
        &unsafe { &*image }.nsf,
        output_rate,
        region,
    ))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_free(machine: *mut CMachine) {
    if machine.is_null() || unsafe { in_callback(machine) } {
        return;
    }
    drop(unsafe { Box::from_raw(machine) });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_select_song(machine: *mut CMachine, song: c_int) -> c_int {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    match u8::try_from(song) {
        Ok(n) => c_int::from(unsafe { &mut (*machine).machine }.select_song(n)),
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_reset(machine: *mut CMachine) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.reset();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_song(machine: *const CMachine) -> c_int {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    c_int::from(unsafe { &(*machine).machine }.song())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_region_get(machine: *const CMachine) -> c_int {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    match unsafe { &(*machine).machine }.region() {
        Region::Ntsc => NTSC,
        Region::Pal => PAL,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_render_s16(machine: *mut CMachine, out: *mut i16, frames: usize) {
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
pub unsafe extern "C" fn cnes_render_f32(machine: *mut CMachine, out: *mut f32, frames: usize) {
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
pub unsafe extern "C" fn cnes_cycles(machine: *const CMachine) -> u64 {
    let callback = unsafe { &*ptr::addr_of!((*machine).callback) };
    if callback.active.load(Ordering::Relaxed) {
        return callback.cycle.load(Ordering::Relaxed);
    }
    unsafe { &*ptr::addr_of!((*machine).machine) }.cycles()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_frames(machine: *const CMachine) -> u64 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.frames()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_set_observer(machine: *mut CMachine, observer: *const CObserver) {
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
pub unsafe extern "C" fn cnes_run_cycles(machine: *mut CMachine, cycles: u64) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.run_cycles(cycles);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_register(machine: *const CMachine, index: c_int) -> u32 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.register(index)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_next_pc(machine: *const CMachine) -> u32 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    u32::from(unsafe { &(*machine).machine }.next_pc())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_peek(machine: *const CMachine, address: u32) -> u32 {
    if unsafe { in_callback(machine) } || address > 0xFFFF {
        return 0;
    }
    u32::from(unsafe { &(*machine).machine }.peek(address as u16))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_warnings(machine: *const CMachine) -> *const c_char {
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
pub unsafe extern "C" fn cnes_snapshot_take(machine: *const CMachine) -> *mut CSnapshot {
    if unsafe { in_callback(machine) } {
        return ptr::null_mut();
    }
    Box::into_raw(Box::new(CSnapshot(
        unsafe { &(*machine).machine }.snapshot(),
    )))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_snapshot_restore(
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
pub unsafe extern "C" fn cnes_snapshot_size(snapshot: *const CSnapshot) -> usize {
    if snapshot.is_null() {
        return 0;
    }
    unsafe { &*snapshot }.0.size()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_snapshot_free(snapshot: *mut CSnapshot) {
    if !snapshot.is_null() {
        drop(unsafe { Box::from_raw(snapshot) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_channel_levels_get(machine: *const CMachine, out: *mut i32) {
    if out.is_null() || unsafe { in_callback(machine) } {
        return;
    }
    let l = unsafe { &(*machine).machine }.channel_levels();
    unsafe { ptr::copy_nonoverlapping(l.as_ptr(), out, 5) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_expansion_levels_get(machine: *const CMachine, out: *mut i32) {
    if out.is_null() || unsafe { in_callback(machine) } {
        return;
    }
    let l = unsafe { &(*machine).machine }.expansion_levels();
    unsafe { ptr::copy_nonoverlapping(l.as_ptr(), out, 8) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_set_channel_mask(machine: *mut CMachine, mask: u32) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.set_channel_mask((mask & 0xFF) as u8);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_channel_mask(machine: *const CMachine) -> u32 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    u32::from(unsafe { &(*machine).machine }.channel_mask())
}

/// 1 when the cutoff is in range and applied, 0 otherwise (also from inside a
/// callback).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_set_highpass_hz(machine: *mut CMachine, hz: f64) -> c_int {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    c_int::from(unsafe { &mut (*machine).machine }.set_highpass_hz(hz))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_use_console_highpass(machine: *mut CMachine) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.use_console_highpass();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_highpass_hz(machine: *const CMachine) -> f64 {
    if unsafe { in_callback(machine) } {
        return 0.0;
    }
    unsafe { &(*machine).machine }.highpass_hz()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cnes_end_state_get(machine: *const CMachine) -> c_int {
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
pub unsafe extern "C" fn cnes_quiet_cycles(machine: *const CMachine) -> u64 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.quiet_cycles()
}
