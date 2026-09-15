//! `fontelle-vst2`: the VST 2 extension for Fontelle.
//!
//! A shared library Fontelle finds in its bridges folder at run time and loads
//! through [`bridge_abi::FontelleBridge`] (ABI 3). It hosts VST 2.4 plugins —
//! `.so` on Linux, including the ones `yabridge` presents Windows plugins as —
//! and links **no Steinberg code**: the interface it drives them through is a
//! clean-room description ([`vst2`]), and the whole point of it being a separate
//! download from a separate repository is that Fontelle's own licence audit is
//! about Fontelle (`docs/vst-plan.md` §3).
//!
//! # Trademark
//!
//! VST is a registered trademark of Steinberg Media Technologies GmbH. This
//! extension describes a file *format* it loads; it carries no VST logo and
//! puts "VST" in no product name.
//!
//! # How it is reached
//!
//! Fontelle calls [`ENTRY_SYMBOL`](bridge_abi::ENTRY_SYMBOL),
//! `fontelle_bridge_entry`, which returns the table below. Everything the host
//! needs is in it; see [`bridge_abi`] for the contract and [`host`] for what
//! each entry actually does to a plugin.

pub mod bridge_abi;
mod host;
mod vst2;

use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::sync::OnceLock;

use bridge_abi::{ABI_VERSION, FontelleBridge, Instance, ParamInfo, PluginInfo};
use host::Vst2Plugin;

const FORMAT: &CStr = c"vst2";
const NAME: &CStr = c"VST 2 plugins";
const EXTENSION: &CStr = c"so";

/// The standard folders VST 2 plugins are installed in on Linux, held for the
/// bridge's life so the pointers the host reads stay valid.
fn search_path_strings() -> &'static [CString] {
    static PATHS: OnceLock<Vec<CString>> = OnceLock::new();
    PATHS.get_or_init(|| {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(&home).join(".vst"));
        }
        dirs.push(PathBuf::from("/usr/lib/vst"));
        dirs.push(PathBuf::from("/usr/lib/lxvst"));
        dirs.push(PathBuf::from("/usr/local/lib/vst"));
        if let Some(extra) = std::env::var_os("VST_PATH") {
            for part in std::env::split_paths(&extra) {
                dirs.push(part);
            }
        }
        dirs.into_iter()
            .filter_map(|p| CString::new(p.to_string_lossy().into_owned()).ok())
            .collect()
    })
}

unsafe extern "C" fn search_paths(out: *mut *const c_char, capacity: u32) -> u32 {
    let paths = search_path_strings();
    let n = paths.len().min(capacity as usize);
    if !out.is_null() {
        for (i, path) in paths.iter().take(n).enumerate() {
            // SAFETY: `out` has room for `capacity` pointers; `i < n <= capacity`.
            unsafe { *out.add(i) = path.as_ptr() };
        }
    }
    paths.len() as u32
}

/// Allocates a C string for a `PluginInfo` field, freed in [`free_infos`].
fn into_c(s: &str) -> *const c_char {
    CString::new(s).unwrap_or_default().into_raw() as *const c_char
}

/// Frees one, matching [`into_c`].
unsafe fn free_c(ptr: *const c_char) {
    if !ptr.is_null() {
        // SAFETY: every non-null field was made by `into_c` / `CString::into_raw`.
        drop(unsafe { CString::from_raw(ptr as *mut c_char) });
    }
}

unsafe extern "C" fn scan_bundle(
    path: *const c_char,
    out: *mut *mut PluginInfo,
    count: *mut u32,
) -> i32 {
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    // A VST 2 `.so` is one plugin (shell plugins that pack several behind one
    // file are an ABI-1 extension this does not read yet). Load it, learn what
    // it is, and let it go.
    let mut plugin = match Vst2Plugin::load(std::path::Path::new(&path)) {
        Ok(plugin) => plugin,
        Err(_) => return -1,
    };
    let scan = plugin.scan();
    drop(plugin);

    let info = PluginInfo {
        id: into_c(&scan.id),
        name: into_c(&scan.name),
        vendor: into_c(&scan.vendor),
        version: into_c(&scan.version),
        is_instrument: u8::from(scan.is_instrument),
    };
    let mut boxed = vec![info].into_boxed_slice();
    unsafe {
        *count = boxed.len() as u32;
        *out = boxed.as_mut_ptr();
    }
    std::mem::forget(boxed);
    0
}

unsafe extern "C" fn free_infos(infos: *mut PluginInfo, count: u32) {
    if infos.is_null() {
        return;
    }
    // SAFETY: `infos`/`count` are exactly what `scan_bundle` handed out.
    let slice = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(infos, count as usize)) };
    for info in slice.iter() {
        unsafe {
            free_c(info.id);
            free_c(info.name);
            free_c(info.vendor);
            free_c(info.version);
        }
    }
    drop(slice);
}

unsafe extern "C" fn open(path: *const c_char, _id: *const c_char) -> Instance {
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    match Vst2Plugin::load(std::path::Path::new(&path)) {
        Ok(plugin) => Box::into_raw(Box::new(plugin)).cast(),
        Err(_) => std::ptr::null_mut(),
    }
}

unsafe extern "C" fn close(instance: Instance) {
    if !instance.is_null() {
        // SAFETY: `instance` came from `open`'s `Box::into_raw`.
        drop(unsafe { Box::from_raw(instance.cast::<Vst2Plugin>()) });
    }
}

/// Borrows the plugin behind an [`Instance`] for one call.
unsafe fn plugin<'a>(instance: Instance) -> Option<&'a mut Vst2Plugin> {
    // SAFETY: every `Instance` the host passes came from `open` and is not
    // aliased — the ABI's thread rules put main-thread and audio-thread calls
    // in the caller's hands, not two at once here.
    unsafe { instance.cast::<Vst2Plugin>().as_mut() }
}

unsafe extern "C" fn param_count(instance: Instance) -> u32 {
    unsafe { plugin(instance) }.map_or(0, |p| p.num_params())
}

unsafe extern "C" fn param_info(instance: Instance, index: u32, out: *mut ParamInfo) -> i32 {
    let Some(plugin) = (unsafe { plugin(instance) }) else {
        return -1;
    };
    if index >= plugin.num_params() || out.is_null() {
        return -1;
    }
    // The name is owned by the instance until `close`, so it is leaked into a
    // C string the host reads and never frees per parameter — the ABI says a
    // param name stays valid until the instance closes, and an instance opens
    // its parameters once.
    let name = into_c(&plugin.param_name(index));
    unsafe {
        (*out).id = index;
        (*out).name = name;
        (*out).module = into_c("");
        // VST 2 has no per-parameter range: every parameter is 0..1.
        (*out).min = 0.0;
        (*out).max = 1.0;
        (*out).default = plugin.get_param(index);
        (*out).stepped = 0;
        (*out).hidden = 0;
        (*out).readonly = 0;
    }
    0
}

unsafe extern "C" fn audio_inputs(instance: Instance) -> u32 {
    unsafe { plugin(instance) }.map_or(0, |p| p.num_inputs())
}

unsafe extern "C" fn audio_outputs(instance: Instance) -> u32 {
    unsafe { plugin(instance) }.map_or(0, |p| p.num_outputs())
}

unsafe extern "C" fn accepts_notes(instance: Instance) -> u8 {
    unsafe { plugin(instance) }.map_or(0, |p| u8::from(p.accepts_notes()))
}

unsafe extern "C" fn set_param(instance: Instance, id: u32, plain: f64) {
    if let Some(plugin) = unsafe { plugin(instance) } {
        plugin.set_param(id, plain);
    }
}

unsafe extern "C" fn get_param(instance: Instance, id: u32) -> f64 {
    unsafe { plugin(instance) }.map_or(0.0, |p| p.get_param(id))
}

unsafe extern "C" fn display(
    instance: Instance,
    id: u32,
    value: f64,
    buffer: *mut c_char,
    capacity: u32,
) -> i32 {
    let Some(plugin) = (unsafe { plugin(instance) }) else {
        return -1;
    };
    let text = plugin.display(id, value);
    let bytes = text.as_bytes();
    let n = bytes.len().min(capacity.saturating_sub(1) as usize);
    if buffer.is_null() || capacity == 0 {
        return -1;
    }
    // SAFETY: `buffer` has `capacity` bytes; `n < capacity`.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer as *mut u8, n);
        *buffer.add(n) = 0;
    }
    n as i32
}

unsafe extern "C" fn activate(instance: Instance, sample_rate: f64, max_block: u32) -> i32 {
    match unsafe { plugin(instance) } {
        Some(plugin) => {
            plugin.activate(sample_rate, max_block);
            0
        }
        None => -1,
    }
}

unsafe extern "C" fn deactivate(instance: Instance) {
    if let Some(plugin) = unsafe { plugin(instance) } {
        plugin.deactivate();
    }
}

unsafe extern "C" fn note_on(instance: Instance, frame: u32, key: u8, velocity: f64) {
    if let Some(plugin) = unsafe { plugin(instance) } {
        plugin.note_on(frame, key, velocity);
    }
}

unsafe extern "C" fn note_off(instance: Instance, frame: u32, key: u8) {
    if let Some(plugin) = unsafe { plugin(instance) } {
        plugin.note_off(frame, key);
    }
}

unsafe extern "C" fn reset(instance: Instance) {
    if let Some(plugin) = unsafe { plugin(instance) } {
        plugin.reset();
    }
}

unsafe extern "C" fn process(
    instance: Instance,
    inputs: *const *const f32,
    input_channels: u32,
    outputs: *const *mut f32,
    output_channels: u32,
    frames: u32,
) {
    let Some(plugin) = (unsafe { plugin(instance) }) else {
        return;
    };
    let frames = frames as usize;
    // SAFETY: the host passes `input_channels`/`output_channels` valid pointers,
    // each to `frames` samples, per the ABI.
    let ins: Vec<&[f32]> = (0..input_channels as usize)
        .map(|c| unsafe { std::slice::from_raw_parts(*inputs.add(c), frames) })
        .collect();
    let mut outs: Vec<&mut [f32]> = (0..output_channels as usize)
        .map(|c| unsafe { std::slice::from_raw_parts_mut(*outputs.add(c), frames) })
        .collect();
    plugin.process(&ins, &mut outs, frames);
}

unsafe extern "C" fn save_state(instance: Instance, out: *mut *mut u8, len: *mut u32) -> i32 {
    let Some(plugin) = (unsafe { plugin(instance) }) else {
        return -1;
    };
    match plugin.save_state() {
        Some(bytes) => {
            let mut boxed = bytes.into_boxed_slice();
            unsafe {
                *len = boxed.len() as u32;
                *out = boxed.as_mut_ptr();
            }
            std::mem::forget(boxed);
            0
        }
        None => -1,
    }
}

unsafe extern "C" fn free_state(bytes: *mut u8, len: u32) {
    if !bytes.is_null() {
        // SAFETY: `bytes`/`len` are what `save_state` handed out.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(bytes, len as usize)) });
    }
}

unsafe extern "C" fn load_state(instance: Instance, bytes: *const u8, len: u32) -> i32 {
    let Some(plugin) = (unsafe { plugin(instance) }) else {
        return -1;
    };
    // SAFETY: the host passes `len` valid bytes for the call's duration.
    let slice = unsafe { std::slice::from_raw_parts(bytes, len as usize) };
    i32::from(plugin.load_state(slice)) - 1 // 0 on success, -1 on refusal
}

unsafe extern "C" fn has_editor(instance: Instance) -> u8 {
    unsafe { plugin(instance) }.map_or(0, |p| u8::from(p.has_editor()))
}

unsafe extern "C" fn open_editor(
    instance: Instance,
    parent: u64,
    width: *mut u32,
    height: *mut u32,
) -> i32 {
    let Some(plugin) = (unsafe { plugin(instance) }) else {
        return -1;
    };
    match plugin.open_editor(parent) {
        Some((w, h)) => {
            unsafe {
                if !width.is_null() {
                    *width = w;
                }
                if !height.is_null() {
                    *height = h;
                }
            }
            0
        }
        None => -1,
    }
}

unsafe extern "C" fn close_editor(instance: Instance) {
    if let Some(plugin) = unsafe { plugin(instance) } {
        plugin.close_editor();
    }
}

unsafe extern "C" fn tick_editor(instance: Instance) {
    if let Some(plugin) = unsafe { plugin(instance) } {
        plugin.tick_editor();
    }
}

unsafe extern "C" fn resize_editor(_instance: Instance, _width: u32, _height: u32) -> i32 {
    // A VST 2 editor is the size the plugin says; the ABI has no "become this
    // size" call, so the host is told the window cannot be resized.
    -1
}

unsafe extern "C" fn controller(instance: Instance, frame: u32, controller: u8, value: u8) {
    if let Some(plugin) = unsafe { plugin(instance) } {
        plugin.controller(frame, controller, value);
    }
}

unsafe extern "C" fn pitch_bend(instance: Instance, frame: u32, value: i16) {
    if let Some(plugin) = unsafe { plugin(instance) } {
        plugin.pitch_bend(frame, value);
    }
}

unsafe extern "C" fn channel_pressure(instance: Instance, frame: u32, value: u8) {
    if let Some(plugin) = unsafe { plugin(instance) } {
        plugin.channel_pressure(frame, value);
    }
}

/// The table, filled in once and returned by every call to the entry point.
static BRIDGE: FontelleBridge = FontelleBridge {
    abi_version: ABI_VERSION,
    format: FORMAT.as_ptr(),
    name: NAME.as_ptr(),
    extension: EXTENSION.as_ptr(),
    search_paths,
    scan_bundle,
    free_infos,
    open,
    close,
    param_count,
    param_info,
    audio_inputs,
    audio_outputs,
    accepts_notes,
    set_param,
    get_param,
    display,
    activate,
    deactivate,
    note_on,
    note_off,
    reset,
    process,
    save_state,
    free_state,
    load_state,
    has_editor,
    open_editor,
    close_editor,
    tick_editor,
    resize_editor,
    controller,
    pitch_bend,
    channel_pressure,
};

/// The symbol Fontelle looks up: hands back the table above.
///
/// # Safety
///
/// The returned pointer is to a `'static` and stays valid for the life of the
/// loaded library, which is what the ABI requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fontelle_bridge_entry() -> *const FontelleBridge {
    &BRIDGE
}
