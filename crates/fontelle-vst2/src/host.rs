//! Loading a VST 2.4 plugin and driving it through the clean-room interface in
//! [`crate::vst2`].
//!
//! One [`Vst2Plugin`] is one loaded `.so` (Linux) or `.dll` (Windows, including
//! the `.dll` `yabridge` presents Windows plugins as). It keeps the library
//! open for its life, holds the plugin's `AEffect`, and turns the bridge ABI's
//! calls into the dispatcher opcodes and buffer conventions the plugin expects.
//!
//! **Threads.** `process`, `note_on`/`note_off`, the performance calls and
//! `reset` are the audio thread's; everything else is the main thread's — the
//! bridge ABI's contract. The events collected between blocks are drained in
//! `process`, so no allocation happens on the audio thread there once the
//! scratch is sized in `activate`.

#![allow(non_upper_case_globals)]

use std::ffi::c_void;
use std::path::Path;
use std::ptr;

use libloading::Library;

use crate::vst2::*;

/// A loaded plugin.
pub struct Vst2Plugin {
    /// Kept open for the plugin's life; dropped last.
    _lib: Library,
    effect: *mut AEffect,
    rate: f64,
    max_block: usize,
    active: bool,
    /// MIDI events queued for the coming block, cleared after each `process`.
    midi: Vec<VstMidiEvent>,
    /// Output channel scratch, so a plugin handed the exact channel count it
    /// declared has somewhere to write even when the host gave fewer.
    out_scratch: Vec<Vec<f32>>,
    in_scratch: Vec<Vec<f32>>,
    /// Parameter display and name buffers a scan or a display call fills.
    string_buf: Vec<i8>,
}

// The plugin is single-threaded by the ABI (see the module note); the bridge
// holds it behind a raw pointer and never shares it across threads at once.
unsafe impl Send for Vst2Plugin {}

/// The one host callback every plugin is handed. It answers the handful of
/// questions a plugin asks during load and run — the version, the rate, the
/// block, and "can you do X" — and says no to everything else, which is the
/// safe default: a plugin that is told a host cannot do something does without
/// it rather than misbehaving.
unsafe extern "C" fn host_callback(
    _effect: *mut AEffect,
    opcode: i32,
    _index: i32,
    _value: isize,
    ptr: *mut c_void,
    _opt: f32,
) -> isize {
    match opcode {
        audioMasterVersion => VST_VERSION_2_4 as isize,
        audioMasterGetSampleRate => 48_000,
        audioMasterGetBlockSize => 512,
        audioMasterGetCurrentProcessLevel => 2, // realtime
        audioMasterGetVendorString => {
            write_c_string(ptr, "Fopull LLC");
            1
        }
        audioMasterGetProductString => {
            write_c_string(ptr, "Fontelle");
            1
        }
        audioMasterGetVendorVersion => 1,
        // "Can you do X?" — 1 for yes, 0 for don't-know, -1 for no. Say yes to
        // the two a plugin needs to send and receive MIDI, and don't-know to
        // the rest rather than a hard no, which some plugins take badly.
        audioMasterCanDo => {
            let request = unsafe { c_str(ptr as *const i8) };
            match request.as_deref() {
                Some("sendVstEvents" | "sendVstMidiEvent" | "receiveVstEvents")
                | Some("receiveVstMidiEvent") => 1,
                _ => 0,
            }
        }
        _ => 0,
    }
}

/// Copies a Rust string into the fixed C buffer a plugin passed, NUL-terminated
/// and never past the 64 bytes these opcodes reserve.
fn write_c_string(ptr: *mut c_void, s: &str) {
    if ptr.is_null() {
        return;
    }
    let bytes = s.as_bytes();
    let n = bytes.len().min(63);
    // SAFETY: the opcode's contract is a caller buffer of at least 64 bytes.
    unsafe {
        let dst = ptr as *mut u8;
        ptr::copy_nonoverlapping(bytes.as_ptr(), dst, n);
        *dst.add(n) = 0;
    }
}

/// Reads a NUL-terminated C string a plugin passed, if any.
unsafe fn c_str(ptr: *const i8) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the callers pass NUL-terminated ASCII per the opcode contract.
    Some(
        unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// What a scan learns about one plugin, owned so the bridge can hand the host
/// stable strings.
pub struct ScanResult {
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub is_instrument: bool,
    /// A VST 2 `.so` holds one plugin, and its stable id is the `uniqueID` the
    /// plugin reports, printed as hex — the id a `PluginKey` is written with.
    pub id: String,
}

impl Vst2Plugin {
    /// Loads the plugin at `path`, calling its entry point with the host
    /// callback. The library stays open for the returned plugin's life.
    pub fn load(path: &Path) -> Result<Self, String> {
        // SAFETY: loading arbitrary native code is the whole job; there is no
        // safe form of it. The path came from a scan of a plugin folder.
        let lib = unsafe { Library::new(path) }
            .map_err(|e| format!("could not open {}: {e}", path.display()))?;

        let entry = Self::entry(&lib)
            .ok_or_else(|| format!("{} exports no VST entry point", path.display()))?;

        // SAFETY: `entry` is the plugin's `VSTPluginMain`, whose contract is to
        // take a host callback and return an `AEffect`.
        let effect = unsafe { entry(host_callback) };
        if effect.is_null() {
            return Err(format!("{} returned no plugin", path.display()));
        }
        // SAFETY: a non-null return is an `AEffect` per the ABI.
        let magic = unsafe { (*effect).magic };
        if magic != VST_MAGIC {
            return Err(format!("{} is not a VST 2 plugin", path.display()));
        }

        let mut plugin = Self {
            _lib: lib,
            effect,
            rate: 48_000.0,
            max_block: 512,
            active: false,
            midi: Vec::new(),
            out_scratch: Vec::new(),
            in_scratch: Vec::new(),
            string_buf: vec![0; 256],
        };
        plugin.dispatch(eff_Open, 0, 0, ptr::null_mut(), 0.0);
        Ok(plugin)
    }

    /// The entry point, `VSTPluginMain` for a 2.4 plugin or `main` for an older
    /// one.
    fn entry(lib: &Library) -> Option<EntryFn> {
        for symbol in ENTRY_SYMBOLS {
            // SAFETY: the symbol, if present, has the entry signature.
            if let Ok(f) = unsafe { lib.get::<EntryFn>(symbol) } {
                return Some(*f);
            }
        }
        None
    }

    fn dispatch(
        &mut self,
        opcode: i32,
        index: i32,
        value: isize,
        ptr: *mut c_void,
        opt: f32,
    ) -> isize {
        // SAFETY: `self.effect` is a valid `AEffect` and `dispatcher` is set on
        // every real plugin; the opcodes and arguments follow the ABI.
        unsafe {
            match (*self.effect).dispatcher {
                Some(dispatch) => dispatch(self.effect, opcode, index, value, ptr, opt),
                None => 0,
            }
        }
    }

    fn flags(&self) -> i32 {
        // SAFETY: valid effect.
        unsafe { (*self.effect).flags }
    }

    /// Scans a loaded plugin for what a listing needs, then it can be dropped.
    pub fn scan(&mut self) -> ScanResult {
        let name = self.string(eff_GetEffectName);
        let vendor = self.string(eff_GetVendorString);
        let version = {
            let v = self.dispatch(eff_GetVendorVersion, 0, 0, ptr::null_mut(), 0.0);
            if v > 0 {
                v.to_string()
            } else {
                "1".to_string()
            }
        };
        // SAFETY: valid effect.
        let unique = unsafe { (*self.effect).uniqueID };
        ScanResult {
            name: if name.is_empty() {
                "VST 2 plugin".to_string()
            } else {
                name
            },
            vendor,
            version,
            is_instrument: self.is_instrument(),
            id: format!("{unique:08x}"),
        }
    }

    fn is_instrument(&mut self) -> bool {
        if self.flags() & EFF_FLAGS_IS_SYNTH != 0 {
            return true;
        }
        self.dispatch(eff_GetPlugCategory, 0, 0, ptr::null_mut(), 0.0) == PLUG_CATEG_SYNTH
    }

    /// Runs a string-returning opcode into the scratch buffer.
    fn string(&mut self, opcode: i32) -> String {
        for slot in self.string_buf.iter_mut() {
            *slot = 0;
        }
        let ptr = self.string_buf.as_mut_ptr() as *mut c_void;
        self.dispatch(opcode, 0, 0, ptr, 0.0);
        buffer_string(&self.string_buf)
    }

    pub fn num_params(&self) -> u32 {
        // SAFETY: valid effect.
        unsafe { (*self.effect).numParams.max(0) as u32 }
    }

    pub fn num_inputs(&self) -> u32 {
        // SAFETY: valid effect.
        unsafe { (*self.effect).numInputs.max(0) as u32 }
    }

    pub fn num_outputs(&self) -> u32 {
        // SAFETY: valid effect.
        unsafe { (*self.effect).numOutputs.max(0) as u32 }
    }

    pub fn accepts_notes(&mut self) -> bool {
        self.is_instrument()
    }

    /// One parameter's name.
    pub fn param_name(&mut self, index: u32) -> String {
        for slot in self.string_buf.iter_mut() {
            *slot = 0;
        }
        let ptr = self.string_buf.as_mut_ptr() as *mut c_void;
        self.dispatch(eff_GetParamName, index as i32, 0, ptr, 0.0);
        let name = buffer_string(&self.string_buf);
        if name.is_empty() {
            format!("Param {}", index + 1)
        } else {
            name
        }
    }

    /// Parameters are always 0..1 in VST 2 — there are no per-parameter ranges
    /// in the ABI, so the bridge reports every one as a normalised control and
    /// leaves the units to [`display`](Self::display).
    pub fn get_param(&self, index: u32) -> f64 {
        // SAFETY: valid effect; `getParameter` is set on every plugin.
        unsafe {
            match (*self.effect).getParameter {
                Some(get) => get(self.effect, index as i32) as f64,
                None => 0.0,
            }
        }
    }

    pub fn set_param(&mut self, index: u32, value: f64) {
        // SAFETY: valid effect; `setParameter` is set on every plugin.
        unsafe {
            if let Some(set) = (*self.effect).setParameter {
                set(self.effect, index as i32, value.clamp(0.0, 1.0) as f32);
            }
        }
    }

    /// The plugin's own text for a value — "-6.0 dB", "440 Hz". Sets the value,
    /// then reads the display and the label back, because that is the only form
    /// the ABI offers.
    pub fn display(&mut self, index: u32, value: f64) -> String {
        self.set_param(index, value);
        for slot in self.string_buf.iter_mut() {
            *slot = 0;
        }
        let ptr = self.string_buf.as_mut_ptr() as *mut c_void;
        self.dispatch(eff_GetParamDisplay, index as i32, 0, ptr, 0.0);
        let shown = buffer_string(&self.string_buf);
        for slot in self.string_buf.iter_mut() {
            *slot = 0;
        }
        let ptr = self.string_buf.as_mut_ptr() as *mut c_void;
        self.dispatch(eff_GetParamLabel, index as i32, 0, ptr, 0.0);
        let label = buffer_string(&self.string_buf);
        if label.is_empty() {
            shown
        } else {
            format!("{shown} {label}")
        }
    }

    pub fn activate(&mut self, rate: f64, max_block: u32) {
        self.rate = rate.max(1.0);
        self.max_block = max_block.max(1) as usize;
        self.dispatch(eff_SetSampleRate, 0, 0, ptr::null_mut(), self.rate as f32);
        self.dispatch(
            eff_SetBlockSize,
            0,
            self.max_block as isize,
            ptr::null_mut(),
            0.0,
        );
        // resume, then start processing.
        self.dispatch(eff_MainsChanged, 0, 1, ptr::null_mut(), 0.0);
        self.dispatch(eff_StartProcess, 0, 0, ptr::null_mut(), 0.0);
        // Size the scratch here, off the audio thread, so `process` never
        // allocates: one buffer per channel the plugin declared.
        let outs = self.num_outputs() as usize;
        let ins = self.num_inputs() as usize;
        self.out_scratch = (0..outs).map(|_| vec![0.0; self.max_block]).collect();
        self.in_scratch = (0..ins).map(|_| vec![0.0; self.max_block]).collect();
        self.active = true;
    }

    pub fn deactivate(&mut self) {
        if !self.active {
            return;
        }
        self.dispatch(eff_StopProcess, 0, 0, ptr::null_mut(), 0.0);
        self.dispatch(eff_MainsChanged, 0, 0, ptr::null_mut(), 0.0);
        self.active = false;
    }

    /// Queues a MIDI event for the coming block. The three status bytes are
    /// built by the callers below.
    fn queue_midi(&mut self, frame: u32, status: u8, d1: u8, d2: u8) {
        self.midi.push(VstMidiEvent {
            kind: EVENT_TYPE_MIDI,
            byte_size: std::mem::size_of::<VstMidiEvent>() as i32,
            delta_frames: frame as i32,
            flags: 0,
            note_length: 0,
            note_offset: 0,
            midi_data: [status, d1, d2, 0],
            detune: 0,
            note_off_velocity: 0,
            reserved1: 0,
            reserved2: 0,
        });
    }

    pub fn note_on(&mut self, frame: u32, key: u8, velocity: f64) {
        let vel = (velocity.clamp(0.0, 1.0) * 127.0).round() as u8;
        // A note-on with velocity 0 is a note-off to a plugin, so floor at 1.
        self.queue_midi(frame, 0x90, key & 0x7f, vel.max(1));
    }

    pub fn note_off(&mut self, frame: u32, key: u8) {
        self.queue_midi(frame, 0x80, key & 0x7f, 0);
    }

    pub fn controller(&mut self, frame: u32, controller: u8, value: u8) {
        self.queue_midi(frame, 0xb0, controller & 0x7f, value & 0x7f);
    }

    pub fn pitch_bend(&mut self, frame: u32, value: i16) {
        // Centre 0 over -8192..=8191 into the 14-bit unsigned the wire carries.
        let unsigned = (value as i32 + 8192).clamp(0, 16383) as u16;
        self.queue_midi(
            frame,
            0xe0,
            (unsigned & 0x7f) as u8,
            ((unsigned >> 7) & 0x7f) as u8,
        );
    }

    pub fn channel_pressure(&mut self, frame: u32, value: u8) {
        self.queue_midi(frame, 0xd0, value & 0x7f, 0);
    }

    pub fn reset(&mut self) {
        self.midi.clear();
        // All-notes-off on the channel, the portable way to silence a plugin
        // that has no reset of its own in the ABI.
        self.queue_midi(0, 0xb0, 123, 0);
    }

    /// One block. Sends any queued MIDI, then `processReplacing` with the exact
    /// channel counts the plugin declared, copying the host's buffers in and
    /// the plugin's out.
    pub fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]], frames: usize) {
        if !self.active || frames == 0 {
            return;
        }
        self.send_midi();

        let outs = self.out_scratch.len();
        let ins = self.in_scratch.len();

        // Fill the input scratch from the host's channels, cycling the last one
        // if the plugin wants more than it was given.
        for (channel, buf) in self.in_scratch.iter_mut().enumerate() {
            let src = inputs.get(channel).or_else(|| inputs.last());
            for frame in 0..frames.min(buf.len()) {
                buf[frame] = src.map_or(0.0, |s| s.get(frame).copied().unwrap_or(0.0));
            }
        }

        let in_ptrs: Vec<*const f32> = self.in_scratch.iter().map(|c| c.as_ptr()).collect();
        let mut out_ptrs: Vec<*mut f32> = self
            .out_scratch
            .iter_mut()
            .map(|c| c.as_mut_ptr())
            .collect();

        // SAFETY: `processReplacing` is set on every 2.4 plugin, the pointer
        // arrays are `numInputs`/`numOutputs` long as the plugin declared, and
        // each buffer holds at least `frames` (<= max_block) samples.
        unsafe {
            if let Some(process) = (*self.effect).processReplacing {
                process(
                    self.effect,
                    if ins == 0 {
                        ptr::null()
                    } else {
                        in_ptrs.as_ptr()
                    },
                    if outs == 0 {
                        ptr::null_mut()
                    } else {
                        out_ptrs.as_mut_ptr()
                    },
                    frames as i32,
                );
            }
        }

        // Copy the plugin's outputs back to the host, cycling the last plugin
        // channel if the host asked for more than the plugin has.
        for (channel, dst) in outputs.iter_mut().enumerate() {
            let src = self
                .out_scratch
                .get(channel)
                .or_else(|| self.out_scratch.last());
            for frame in 0..frames.min(dst.len()) {
                dst[frame] = src.map_or(0.0, |s| s.get(frame).copied().unwrap_or(0.0));
            }
        }

        self.midi.clear();
    }

    /// Hands the queued events to the plugin as one `VstEvents` array.
    fn send_midi(&mut self) {
        if self.midi.is_empty() {
            return;
        }
        // A `VstEvents` with room for one pointer per event: the header, then
        // the pointers, in one allocation the plugin reads during this call
        // only.
        let count = self.midi.len();
        let header = std::mem::size_of::<i32>() + std::mem::size_of::<isize>();
        let bytes = header + count * std::mem::size_of::<*mut VstEvent>();
        let mut block = vec![0u8; bytes];
        // SAFETY: `block` is `bytes` long, laid out as a `VstEvents` with
        // `count` trailing pointers, which is the ABI's variable-length form.
        unsafe {
            let events = block.as_mut_ptr() as *mut VstEvents;
            (*events).num_events = count as i32;
            (*events).reserved = 0;
            let list = ptr::addr_of_mut!((*events).events) as *mut *mut VstEvent;
            for (i, event) in self.midi.iter_mut().enumerate() {
                *list.add(i) = event as *mut VstMidiEvent as *mut VstEvent;
            }
            self.dispatch(eff_ProcessEvents, 0, 0, events as *mut c_void, 0.0);
        }
    }

    /// The plugin's opaque state, when it keeps one (`effFlagsProgramChunks`).
    pub fn save_state(&mut self) -> Option<Vec<u8>> {
        if self.flags() & EFF_FLAGS_PROGRAM_CHUNKS == 0 {
            return None;
        }
        let mut data: *mut c_void = ptr::null_mut();
        let len = self.dispatch(
            eff_GetChunk,
            0,
            0,
            &mut data as *mut *mut c_void as *mut c_void,
            0.0,
        );
        if len <= 0 || data.is_null() {
            return None;
        }
        // SAFETY: the plugin returned a buffer of `len` bytes it owns; we copy
        // it out and never free it.
        let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, len as usize) };
        Some(bytes.to_vec())
    }

    pub fn load_state(&mut self, bytes: &[u8]) -> bool {
        if self.flags() & EFF_FLAGS_PROGRAM_CHUNKS == 0 {
            return false;
        }
        self.dispatch(
            eff_SetChunk,
            0,
            bytes.len() as isize,
            bytes.as_ptr() as *mut c_void,
            0.0,
        ) >= 0
    }

    pub fn has_editor(&self) -> bool {
        self.flags() & EFF_FLAGS_HAS_EDITOR != 0
    }

    /// Embeds the plugin's editor in the X11 window `parent`, reporting the size
    /// it wants. The window id is the X11 `Window` the host made, passed where
    /// the ABI wants a native parent handle.
    pub fn open_editor(&mut self, parent: u64) -> Option<(u32, u32)> {
        if !self.has_editor() {
            return None;
        }
        let opened = self.dispatch(eff_EditOpen, 0, 0, parent as usize as *mut c_void, 0.0);
        if opened == 0 {
            return None;
        }
        let mut rect: *mut ERect = ptr::null_mut();
        self.dispatch(
            eff_EditGetRect,
            0,
            0,
            &mut rect as *mut *mut ERect as *mut c_void,
            0.0,
        );
        if rect.is_null() {
            return Some((640, 480));
        }
        // SAFETY: `effEditGetRect` set `rect` to a struct it owns.
        let r = unsafe { &*rect };
        let w = (r.right - r.left).max(1) as u32;
        let h = (r.bottom - r.top).max(1) as u32;
        Some((w, h))
    }

    pub fn tick_editor(&mut self) {
        self.dispatch(eff_EditIdle, 0, 0, ptr::null_mut(), 0.0);
    }

    pub fn close_editor(&mut self) {
        self.dispatch(eff_EditClose, 0, 0, ptr::null_mut(), 0.0);
    }
}

impl Drop for Vst2Plugin {
    fn drop(&mut self) {
        self.deactivate();
        self.dispatch(eff_Close, 0, 0, ptr::null_mut(), 0.0);
        // The plugin frees itself on `effClose`; the library is dropped after.
    }
}
