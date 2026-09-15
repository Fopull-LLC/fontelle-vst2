//! A clean-room description of the VST 2.4 plugin interface.
//!
//! # Provenance — read this before touching this file
//!
//! **No Steinberg file was used to write this.** VST 2 has not been licensable
//! since October 2018, and its headers (`aeffect.h`, `aeffectx.h`) may not be
//! copied or shared. What is here is a *clean-room* transliteration of the
//! interface, in the same tradition every open-source VST 2 host uses — VeSTige
//! (LMMS), FST (Ardour), and Xaymar's BSD-3-Clause `vst2sdk`, which was written
//! from the observable behaviour of the ABI by developers who never read
//! Steinberg's headers. This module was written from that public,
//! BSD-3-licensed description and from the documented `AEffect` calling
//! convention; it links no Steinberg code and copies none.
//!
//! The `#[repr(C)]` layout below is not creative expression — it is the binary
//! interface a compiled plugin already expects, the way a socket's wire format
//! is. Reproducing an interface so existing programs can interoperate is the
//! established basis every host here relies on (EU Software Directive art. 6;
//! *Sega v. Accolade*; *Google v. Oracle*). See `CONTRIBUTING.md`, which every
//! contributor to this file agrees to: you have never read Steinberg's VST 2
//! headers, and you did not consult them to write or change this.
//!
//! The names are the ones the interface has always had (`AEffect`,
//! `processReplacing`, `effOpen`), because interoperating means speaking the
//! same words; they are facts about the interface, not text taken from a file.

#![allow(non_snake_case, non_upper_case_globals, dead_code)]

use std::ffi::{c_char, c_void};

/// The magic number an `AEffect` carries in its first field: the bytes
/// `'VstP'` big-endian, which is how a host tells a real plugin struct from
/// random memory.
pub const VST_MAGIC: i32 = 0x5673_7450; // 'V''s''t''P' (kEffectMagic)

/// The ABI revision a 2.4 host reports and a plugin checks: 2400.
pub const VST_VERSION_2_4: i32 = 2400;

/// The entry point every VST 2 shared library exports. Older plugins export
/// `main`; 2.4 plugins export `VSTPluginMain`. Both take the host callback and
/// return the plugin's `AEffect`.
pub const ENTRY_SYMBOLS: [&[u8]; 2] = [b"VSTPluginMain\0", b"main\0"];

pub type HostCallback = unsafe extern "C" fn(
    effect: *mut AEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr: *mut c_void,
    opt: f32,
) -> isize;

pub type EntryFn = unsafe extern "C" fn(callback: HostCallback) -> *mut AEffect;

pub type DispatcherFn = unsafe extern "C" fn(
    effect: *mut AEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr: *mut c_void,
    opt: f32,
) -> isize;

pub type ProcessFn = unsafe extern "C" fn(
    effect: *mut AEffect,
    inputs: *const *const f32,
    outputs: *mut *mut f32,
    frames: i32,
);

pub type SetParameterFn = unsafe extern "C" fn(effect: *mut AEffect, index: i32, value: f32);
pub type GetParameterFn = unsafe extern "C" fn(effect: *mut AEffect, index: i32) -> f32;

/// The struct a plugin's entry point returns — the whole of the interface a
/// host holds. The field order and offsets are the ABI's; nothing here is a
/// choice.
#[repr(C)]
pub struct AEffect {
    /// `VST_MAGIC`.
    pub magic: i32,
    /// Everything that is not audio goes through here.
    pub dispatcher: Option<DispatcherFn>,
    /// The pre-2.4 accumulating process. Deprecated; hosts call
    /// `processReplacing`.
    pub process: Option<ProcessFn>,
    pub setParameter: Option<SetParameterFn>,
    pub getParameter: Option<GetParameterFn>,

    pub numPrograms: i32,
    pub numParams: i32,
    pub numInputs: i32,
    pub numOutputs: i32,

    /// A bitmask — see the `EFF_FLAGS_*` constants.
    pub flags: i32,

    pub resvd1: isize,
    pub resvd2: isize,

    /// The latency the plugin adds, in samples.
    pub initialDelay: i32,

    pub realQualities: i32,
    pub offQualities: i32,
    pub ioRatio: f32,

    /// The plugin's own instance pointer; the host does not touch it.
    pub object: *mut c_void,
    /// Free for the host to hang its own per-instance data on.
    pub user: *mut c_void,

    pub uniqueID: i32,
    pub version: i32,

    /// The one every host actually calls: writes each output channel rather
    /// than adding into it.
    pub processReplacing: Option<ProcessFn>,
    /// The 64-bit variant; rarely present.
    pub processDoubleReplacing: *mut c_void,

    /// Padding to the struct's fixed size. Not read.
    pub future: [u8; 56],
}

// The plugin's `AEffect` lives in the plugin's memory and is single-threaded
// by the ABI's contract (see the module note in `lib.rs`); the bridge holds a
// pointer to it and does the synchronising the ABI asks of a host.
unsafe impl Send for AEffect {}

// ---- `flags` bits ---------------------------------------------------------

/// The plugin has an editor.
pub const EFF_FLAGS_HAS_EDITOR: i32 = 1 << 0;
/// The plugin can `processReplacing`. Every 2.4 plugin sets it.
pub const EFF_FLAGS_CAN_REPLACING: i32 = 1 << 4;
/// The plugin stores its state as an opaque chunk (`effGetChunk`), rather than
/// only as its parameter values.
pub const EFF_FLAGS_PROGRAM_CHUNKS: i32 = 1 << 5;
/// The plugin is an instrument: it makes sound from notes rather than
/// processing input.
pub const EFF_FLAGS_IS_SYNTH: i32 = 1 << 8;

// ---- `dispatcher` opcodes (host -> plugin) --------------------------------
//
// Only the ones this bridge uses are named; the numbers are the ABI's.

pub const eff_Open: i32 = 0;
pub const eff_Close: i32 = 1;
pub const eff_SetProgram: i32 = 2;
pub const eff_GetParamLabel: i32 = 6;
pub const eff_GetParamDisplay: i32 = 7;
pub const eff_GetParamName: i32 = 8;
pub const eff_SetSampleRate: i32 = 10;
pub const eff_SetBlockSize: i32 = 11;
pub const eff_MainsChanged: i32 = 12;
pub const eff_EditGetRect: i32 = 13;
pub const eff_EditOpen: i32 = 14;
pub const eff_EditClose: i32 = 15;
pub const eff_EditIdle: i32 = 19;
pub const eff_GetChunk: i32 = 23;
pub const eff_SetChunk: i32 = 24;
pub const eff_ProcessEvents: i32 = 25;
pub const eff_GetEffectName: i32 = 45;
pub const eff_GetVendorString: i32 = 47;
pub const eff_GetProductString: i32 = 48;
pub const eff_GetVendorVersion: i32 = 49;
pub const eff_GetVstVersion: i32 = 58;
pub const eff_GetPlugCategory: i32 = 35;
pub const eff_StartProcess: i32 = 71;
pub const eff_StopProcess: i32 = 72;

// ---- host opcodes (plugin -> host, through the callback) -------------------

pub const audioMasterVersion: i32 = 1;
pub const audioMasterAutomate: i32 = 0;
pub const audioMasterIdle: i32 = 3;
pub const audioMasterGetSampleRate: i32 = 16;
pub const audioMasterGetBlockSize: i32 = 17;
pub const audioMasterGetCurrentProcessLevel: i32 = 23;
pub const audioMasterGetVendorString: i32 = 32;
pub const audioMasterGetProductString: i32 = 33;
pub const audioMasterGetVendorVersion: i32 = 34;
pub const audioMasterCanDo: i32 = 37;

/// The plugin category `effGetPlugCategory` reports; `kPlugCategSynth` (2)
/// is the other way a plugin says it is an instrument.
pub const PLUG_CATEG_SYNTH: isize = 2;

// ---- events (host -> plugin, through `effProcessEvents`) -------------------

pub const EVENT_TYPE_MIDI: i32 = 1;

/// The header both event kinds share. A `VstMidiEvent` is passed where a
/// `VstEvent*` is expected; the first two fields line up.
#[repr(C)]
pub struct VstEvent {
    pub kind: i32,
    pub byte_size: i32,
    pub delta_frames: i32,
    pub flags: i32,
    pub data: [u8; 16],
}

/// One MIDI event in a block. `midiData[0..3]` is the status/data bytes.
#[repr(C)]
pub struct VstMidiEvent {
    pub kind: i32,
    pub byte_size: i32,
    pub delta_frames: i32,
    pub flags: i32,
    pub note_length: i32,
    pub note_offset: i32,
    pub midi_data: [u8; 4],
    pub detune: i8,
    pub note_off_velocity: u8,
    pub reserved1: u8,
    pub reserved2: u8,
}

/// The array of event pointers handed to `effProcessEvents`. Declared with a
/// one-element tail the caller over-allocates, which is how the ABI passes a
/// variable-length list.
#[repr(C)]
pub struct VstEvents {
    pub num_events: i32,
    pub reserved: isize,
    /// Really `num_events` pointers; the struct is allocated with room for
    /// them. One here so the type is sized.
    pub events: [*mut VstEvent; 1],
}

/// The rectangle `effEditGetRect` writes, through a pointer to a pointer.
#[repr(C)]
pub struct ERect {
    pub top: i16,
    pub left: i16,
    pub bottom: i16,
    pub right: i16,
}

/// Reads a NUL-terminated ASCII string a plugin wrote into a fixed buffer,
/// which is how every `eff*String` opcode answers.
pub fn buffer_string(buffer: &[c_char]) -> String {
    let bytes: Vec<u8> = buffer
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::VST_MAGIC;

    /// `kEffectMagic = CCONST('V','s','t','P')` — a real plugin's `AEffect.magic`
    /// is this exact number, the big-endian bytes `VstP`. The fixture can stamp
    /// whatever the bridge checks for, so agreement between them proves nothing;
    /// only this pins the constant to the ABI every third-party plugin uses.
    #[test]
    fn the_magic_is_vstp_the_bytes_every_real_plugin_stamps() {
        assert_eq!(
            VST_MAGIC.to_be_bytes(),
            *b"VstP",
            "the magic must be the ABI's 'VstP', or the bridge rejects every real plugin"
        );
    }
}
