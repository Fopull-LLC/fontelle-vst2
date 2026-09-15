//! A tiny VST 2.4 plugin, for testing the bridge's loader.
//!
//! It is a **gain**: two in, two out, one parameter that scales the signal.
//! Written against the same clean-room description of the interface the bridge
//! drives plugins through — an independent transliteration of the `AEffect` ABI,
//! which is exactly the point: if the fixture and the bridge, written from the
//! same public description without either reading Steinberg's headers, load and
//! run each other, the bridge speaks the interface correctly.
//!
//! No Steinberg file was used to write this. See the bridge's `src/vst2.rs` and
//! the repository's `CONTRIBUTING.md`.

#![allow(non_snake_case, non_upper_case_globals, dead_code, private_interfaces)]

use std::ffi::c_void;

const VST_MAGIC: i32 = 0x5665_7350;
const VST_VERSION_2_4: i32 = 2400;

const EFF_FLAGS_CAN_REPLACING: i32 = 1 << 4;

const eff_GetEffectName: i32 = 45;
const eff_GetVendorString: i32 = 47;
const eff_GetProductString: i32 = 48;
const eff_GetParamName: i32 = 8;
const eff_GetParamDisplay: i32 = 7;

type HostCallback = unsafe extern "C" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize;
type DispatcherFn = unsafe extern "C" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize;
type ProcessFn = unsafe extern "C" fn(*mut AEffect, *const *const f32, *mut *mut f32, i32);
type SetParameterFn = unsafe extern "C" fn(*mut AEffect, i32, f32);
type GetParameterFn = unsafe extern "C" fn(*mut AEffect, i32) -> f32;

#[repr(C)]
struct AEffect {
    magic: i32,
    dispatcher: Option<DispatcherFn>,
    process: Option<ProcessFn>,
    setParameter: Option<SetParameterFn>,
    getParameter: Option<GetParameterFn>,
    numPrograms: i32,
    numParams: i32,
    numInputs: i32,
    numOutputs: i32,
    flags: i32,
    resvd1: isize,
    resvd2: isize,
    initialDelay: i32,
    realQualities: i32,
    offQualities: i32,
    ioRatio: f32,
    object: *mut c_void,
    user: *mut c_void,
    uniqueID: i32,
    version: i32,
    processReplacing: Option<ProcessFn>,
    processDoubleReplacing: *mut c_void,
    future: [u8; 56],
}

/// The plugin's own state: the one parameter, and a place for the host callback.
struct Fixture {
    gain: f32,
}

unsafe extern "C" fn dispatcher(
    _effect: *mut AEffect,
    opcode: i32,
    _index: i32,
    _value: isize,
    ptr: *mut c_void,
    _opt: f32,
) -> isize {
    let write = |text: &str| {
        if !ptr.is_null() {
            let bytes = text.as_bytes();
            let n = bytes.len().min(31);
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, n);
                *(ptr as *mut u8).add(n) = 0;
            }
        }
    };
    match opcode {
        eff_GetEffectName => {
            write("Fixture Gain");
            1
        }
        eff_GetVendorString => {
            write("Fopull LLC");
            1
        }
        eff_GetProductString => {
            write("Fontelle VST2 Fixture");
            1
        }
        eff_GetParamName => {
            write("Gain");
            1
        }
        eff_GetParamDisplay => {
            write("unity");
            1
        }
        _ => 0,
    }
}

unsafe extern "C" fn set_parameter(effect: *mut AEffect, _index: i32, value: f32) {
    // SAFETY: `object` is the `Box<Fixture>` we made in the entry point.
    let fixture = unsafe { &mut *((*effect).object as *mut Fixture) };
    fixture.gain = value;
}

unsafe extern "C" fn get_parameter(effect: *mut AEffect, _index: i32) -> f32 {
    let fixture = unsafe { &*((*effect).object as *mut Fixture) };
    fixture.gain
}

unsafe extern "C" fn process_replacing(
    effect: *mut AEffect,
    inputs: *const *const f32,
    outputs: *mut *mut f32,
    frames: i32,
) {
    let fixture = unsafe { &*((*effect).object as *mut Fixture) };
    let gain = fixture.gain;
    let frames = frames.max(0) as usize;
    for channel in 0..2usize {
        // SAFETY: the host passes two input and two output channels, each
        // `frames` long, matching `numInputs`/`numOutputs`.
        unsafe {
            let src = *inputs.add(channel);
            let dst = *outputs.add(channel);
            for i in 0..frames {
                *dst.add(i) = *src.add(i) * gain;
            }
        }
    }
}

/// The VST 2.4 entry point. Builds the `AEffect` and hangs the plugin's state
/// off it.
#[unsafe(no_mangle)]
pub extern "C" fn VSTPluginMain(_callback: HostCallback) -> *mut AEffect {
    let fixture = Box::into_raw(Box::new(Fixture { gain: 1.0 }));
    let effect = Box::new(AEffect {
        magic: VST_MAGIC,
        dispatcher: Some(dispatcher),
        process: Some(process_replacing),
        setParameter: Some(set_parameter),
        getParameter: Some(get_parameter),
        numPrograms: 0,
        numParams: 1,
        numInputs: 2,
        numOutputs: 2,
        flags: EFF_FLAGS_CAN_REPLACING,
        resvd1: 0,
        resvd2: 0,
        initialDelay: 0,
        realQualities: 0,
        offQualities: 0,
        ioRatio: 1.0,
        object: fixture as *mut c_void,
        user: std::ptr::null_mut(),
        uniqueID: i32::from_be_bytes(*b"FvTX"),
        version: 1,
        processReplacing: Some(process_replacing),
        processDoubleReplacing: std::ptr::null_mut(),
        future: [0; 56],
    });
    Box::into_raw(effect)
}
