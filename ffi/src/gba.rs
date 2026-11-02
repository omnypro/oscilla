//! The C interface of `cgba.h`.

use crate::write_error;
use oscilla_gba::image::{Image, status};
use oscilla_gba::{Machine, Observer};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::{ptr, slice};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_image_load_gsf(
    path: *const c_char,
    out: *mut *mut Image,
    error: *mut c_char,
    error_size: usize,
) -> c_int {
    if out.is_null() {
        return status::IO;
    }
    unsafe { *out = ptr::null_mut() };
    write_error(error, error_size, b"");
    if path.is_null() {
        return status::IO;
    }
    let path = unsafe { CStr::from_ptr(path) }.to_bytes();
    match Image::load_gsf(path) {
        Ok(img) => {
            unsafe { *out = Box::into_raw(Box::new(img)) };
            status::OK
        }
        Err(e) => {
            write_error(error, error_size, &e.message);
            e.status
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_image_from_rom(
    rom: *const u8,
    size: usize,
    out: *mut *mut Image,
) -> c_int {
    if out.is_null() {
        return status::IO;
    }
    unsafe { *out = ptr::null_mut() };
    if rom.is_null() || size == 0 || size > oscilla_gba::image::ROM_MAX as usize {
        return status::FORMAT;
    }
    match Image::from_rom(unsafe { slice::from_raw_parts(rom, size) }) {
        Ok(img) => {
            unsafe { *out = Box::into_raw(Box::new(img)) };
            status::OK
        }
        Err(code) => code,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_image_free(image: *mut Image) {
    if !image.is_null() {
        drop(unsafe { Box::from_raw(image) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_image_tag(image: *const Image, key: *const c_char) -> *const c_char {
    if image.is_null() || key.is_null() {
        return ptr::null();
    }
    let (img, key) = unsafe { (&*image, CStr::from_ptr(key).to_bytes()) };
    img.tag(key).map_or(ptr::null(), CStr::as_ptr)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_image_length_ms(image: *const Image) -> i32 {
    if image.is_null() {
        return -1;
    }
    unsafe { &*image }.length_ms()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_image_fade_ms(image: *const Image) -> i32 {
    if image.is_null() {
        return -1;
    }
    unsafe { &*image }.fade_ms()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_image_entry(image: *const Image) -> u32 {
    unsafe { &*image }.entry()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_image_rom_size(image: *const Image) -> usize {
    unsafe { &*image }.rom().len()
}

// ---- Machines ---------------------------------------------------------

/// The cycle that a running observer callback should see. A callback may call
/// `cgba_cycles` while the machine is mid-render, so the machine's own counter
/// is not read then: the value lives here, outside the machine, and is set
/// before each callback.
struct Callback {
    active: AtomicBool,
    cycle: AtomicU64,
}

/// True while an observer callback runs. Inside one, `cgba_cycles` is the only
/// call that reads the machine. Every other call returns at once with a
/// neutral value, because a second access would alias the machine that the
/// running render holds.
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
    sound_write: Option<unsafe extern "C" fn(*mut c_void, u32, u32, c_int)>,
    fifo_sample: Option<unsafe extern "C" fn(*mut c_void, c_int, i8)>,
}

struct Forward {
    observer: CObserver,
    callback: Arc<Callback>,
}

impl Observer for Forward {
    fn sound_write(&mut self, cycle: u64, address: u32, value: u32, width: i32) {
        if let Some(f) = self.observer.sound_write {
            self.callback.cycle.store(cycle, Ordering::Relaxed);
            self.callback.active.store(true, Ordering::Relaxed);
            unsafe { f(self.observer.context, address, value, width) };
            self.callback.active.store(false, Ordering::Relaxed);
        }
    }

    fn fifo_sample(&mut self, cycle: u64, channel: i32, sample: i8) {
        if let Some(f) = self.observer.fifo_sample {
            self.callback.cycle.store(cycle, Ordering::Relaxed);
            self.callback.active.store(true, Ordering::Relaxed);
            unsafe { f(self.observer.context, channel, sample) };
            self.callback.active.store(false, Ordering::Relaxed);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_new(image: *const Image, output_rate: c_int) -> *mut CMachine {
    if image.is_null() {
        return ptr::null_mut();
    }
    match Machine::new(unsafe { &*image }, output_rate) {
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
pub unsafe extern "C" fn cgba_free(machine: *mut CMachine) {
    if unsafe { in_callback(machine) } {
        return;
    }
    if !machine.is_null() {
        drop(unsafe { Box::from_raw(machine) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_reset(machine: *mut CMachine) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.reset();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_render_s16(machine: *mut CMachine, out: *mut i16, frames: usize) {
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
pub unsafe extern "C" fn cgba_render_f32(machine: *mut CMachine, out: *mut f32, frames: usize) {
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
pub unsafe extern "C" fn cgba_cycles(machine: *const CMachine) -> u64 {
    let callback = unsafe { &*ptr::addr_of!((*machine).callback) };
    if callback.active.load(Ordering::Relaxed) {
        return callback.cycle.load(Ordering::Relaxed);
    }
    unsafe { &*ptr::addr_of!((*machine).machine) }.cycles()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_set_observer(machine: *mut CMachine, observer: *const CObserver) {
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
pub unsafe extern "C" fn cgba_run_cycles(machine: *mut CMachine, cycles: u64) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.run_cycles(cycles);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_register(machine: *const CMachine, index: c_int) -> u32 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.register(index)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_next_pc(machine: *const CMachine) -> u32 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.next_pc()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_next_vblank(machine: *const CMachine) -> u64 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.next_vblank()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_peek(machine: *const CMachine, address: u32, width: c_int) -> u32 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.peek(address, width)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_warnings(machine: *const CMachine) -> *const c_char {
    if unsafe { in_callback(machine) } {
        return c"".as_ptr();
    }
    unsafe { &(*machine).machine }
        .warnings_nul()
        .as_ptr()
        .cast()
}

// ---- Snapshots, channels and the end of a song -------------------------

pub struct CSnapshot(oscilla_gba::Snapshot);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_snapshot_take(machine: *const CMachine) -> *mut CSnapshot {
    if unsafe { in_callback(machine) } {
        return ptr::null_mut();
    }
    Box::into_raw(Box::new(CSnapshot(
        unsafe { &(*machine).machine }.snapshot(),
    )))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_snapshot_restore(
    machine: *mut CMachine,
    snapshot: *const CSnapshot,
) -> c_int {
    if unsafe { in_callback(machine) } {
        return 3;
    }
    match unsafe { &mut (*machine).machine }.restore(&unsafe { &*snapshot }.0) {
        Ok(()) => 0,
        Err(oscilla_gba::RestoreError::Rate) => 1,
        Err(oscilla_gba::RestoreError::Image) => 2,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_snapshot_size(snapshot: *const CSnapshot) -> usize {
    unsafe { &*snapshot }.0.size()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_snapshot_free(snapshot: *mut CSnapshot) {
    if !snapshot.is_null() {
        drop(unsafe { Box::from_raw(snapshot) });
    }
}

#[repr(C)]
pub struct CChannelLevels {
    psg: [i32; 4],
    fifo: [i32; 2],
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_channel_levels_get(
    machine: *const CMachine,
    out: *mut CChannelLevels,
) {
    if unsafe { in_callback(machine) } {
        return;
    }
    let l = unsafe { &(*machine).machine }.channel_levels();
    unsafe {
        *out = CChannelLevels {
            psg: l.psg,
            fifo: l.fifo,
        }
    };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_set_channel_mask(machine: *mut CMachine, mask: u32) {
    if unsafe { in_callback(machine) } {
        return;
    }
    unsafe { &mut (*machine).machine }.set_channel_mask(mask as u8);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_channel_mask(machine: *const CMachine) -> u32 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    u32::from(unsafe { &(*machine).machine }.channel_mask())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_end_state_get(machine: *const CMachine) -> c_int {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    match unsafe { &(*machine).machine }.end_state() {
        oscilla_gba::EndState::Running => 0,
        oscilla_gba::EndState::HaltedWithoutWake => 1,
        oscilla_gba::EndState::Parked => 2,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cgba_quiet_cycles(machine: *const CMachine) -> u64 {
    if unsafe { in_callback(machine) } {
        return 0;
    }
    unsafe { &(*machine).machine }.quiet_cycles()
}
