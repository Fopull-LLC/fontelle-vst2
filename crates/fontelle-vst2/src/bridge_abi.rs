//! The C ABI Fontelle loads a bridge through — **ABI version 3**.
//!
//! This is a standalone copy of Fontelle's `fontelle-bridge-abi` crate, the
//! `#[repr(C)]` contract a bridge fills in. It lives here rather than as a
//! dependency for the reason the whole extension is separate (`docs/vst-plan.md`
//! §3.2): Fontelle never links this repository and this repository never links
//! Fontelle. The ABI crate's own note says a bridge in another language
//! includes "the equivalent header"; this is that header, in Rust.
//!
//! **It must stay in step with `fontelle-bridge-abi`.** The number below is the
//! contract both sides agree on; a mismatch is refused by the host rather than
//! read past its end. If Fontelle's ABI moves to 4, this file and
//! [`ABI_VERSION`] move with it — the loader test (`tests/loads.rs`) is what
//! catches a table that no longer lines up.

use std::ffi::{c_char, c_void};

/// The contract version. Must equal the host's.
pub const ABI_VERSION: u32 = 3;

/// The symbol a bridge exports.
pub const ENTRY_SYMBOL: &str = "fontelle_bridge_entry";

pub type EntryFn = unsafe extern "C" fn() -> *const FontelleBridge;

#[repr(C)]
pub struct PluginInfo {
    pub id: *const c_char,
    pub name: *const c_char,
    pub vendor: *const c_char,
    pub version: *const c_char,
    pub is_instrument: u8,
}

#[repr(C)]
pub struct ParamInfo {
    pub id: u32,
    pub name: *const c_char,
    pub module: *const c_char,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub stepped: u8,
    pub hidden: u8,
    pub readonly: u8,
}

pub type Instance = *mut c_void;

#[repr(C)]
pub struct FontelleBridge {
    pub abi_version: u32,
    pub format: *const c_char,
    pub name: *const c_char,
    pub extension: *const c_char,

    pub search_paths: unsafe extern "C" fn(out: *mut *const c_char, capacity: u32) -> u32,

    pub scan_bundle: unsafe extern "C" fn(
        path: *const c_char,
        out: *mut *mut PluginInfo,
        count: *mut u32,
    ) -> i32,
    pub free_infos: unsafe extern "C" fn(infos: *mut PluginInfo, count: u32),

    pub open: unsafe extern "C" fn(path: *const c_char, id: *const c_char) -> Instance,
    pub close: unsafe extern "C" fn(instance: Instance),

    pub param_count: unsafe extern "C" fn(instance: Instance) -> u32,
    pub param_info:
        unsafe extern "C" fn(instance: Instance, index: u32, out: *mut ParamInfo) -> i32,
    pub audio_inputs: unsafe extern "C" fn(instance: Instance) -> u32,
    pub audio_outputs: unsafe extern "C" fn(instance: Instance) -> u32,
    pub accepts_notes: unsafe extern "C" fn(instance: Instance) -> u8,

    pub set_param: unsafe extern "C" fn(instance: Instance, id: u32, plain: f64),
    pub get_param: unsafe extern "C" fn(instance: Instance, id: u32) -> f64,
    pub display: unsafe extern "C" fn(
        instance: Instance,
        id: u32,
        value: f64,
        buffer: *mut c_char,
        capacity: u32,
    ) -> i32,

    pub activate: unsafe extern "C" fn(instance: Instance, sample_rate: f64, max_block: u32) -> i32,
    pub deactivate: unsafe extern "C" fn(instance: Instance),

    pub note_on: unsafe extern "C" fn(instance: Instance, frame: u32, key: u8, velocity: f64),
    pub note_off: unsafe extern "C" fn(instance: Instance, frame: u32, key: u8),
    pub reset: unsafe extern "C" fn(instance: Instance),
    pub process: unsafe extern "C" fn(
        instance: Instance,
        inputs: *const *const f32,
        input_channels: u32,
        outputs: *const *mut f32,
        output_channels: u32,
        frames: u32,
    ),

    pub save_state:
        unsafe extern "C" fn(instance: Instance, out: *mut *mut u8, len: *mut u32) -> i32,
    pub free_state: unsafe extern "C" fn(bytes: *mut u8, len: u32),
    pub load_state: unsafe extern "C" fn(instance: Instance, bytes: *const u8, len: u32) -> i32,

    // ABI 2: the plugin's own editor.
    pub has_editor: unsafe extern "C" fn(instance: Instance) -> u8,
    pub open_editor: unsafe extern "C" fn(
        instance: Instance,
        parent: u64,
        width: *mut u32,
        height: *mut u32,
    ) -> i32,
    pub close_editor: unsafe extern "C" fn(instance: Instance),
    pub tick_editor: unsafe extern "C" fn(instance: Instance),
    pub resize_editor: unsafe extern "C" fn(instance: Instance, width: u32, height: u32) -> i32,

    // ABI 3: the rest of a performance.
    pub controller: unsafe extern "C" fn(instance: Instance, frame: u32, controller: u8, value: u8),
    pub pitch_bend: unsafe extern "C" fn(instance: Instance, frame: u32, value: i16),
    pub channel_pressure: unsafe extern "C" fn(instance: Instance, frame: u32, value: u8),
}

// SAFETY: the table is function pointers and 'static strings, immutable for
// the life of the library that returned it.
unsafe impl Sync for FontelleBridge {}
unsafe impl Send for FontelleBridge {}
