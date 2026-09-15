//! The bridge against a *real*, third-party VST 2 plugin — the check the
//! fixture cannot make, because a fixture is written against the same
//! interface the bridge is. This opens whatever plugin the environment points
//! at, scans it, and — for an instrument — plays a note and asserts sound
//! actually came out.
//!
//! Ignored by default: it needs a plugin on disk, which CI does not have. Run
//! it by hand against one you have installed:
//!
//! ```text
//! FONTELLE_VST2_PLUGIN=/usr/lib/vst/amsynth_vst.so \
//!   cargo test -p fontelle-vst2 --test real -- --ignored --nocapture
//! ```
//!
//! On Windows point it at a `.dll`, on macOS at the binary inside a `.vst`
//! bundle. The same bridge code loads all three; this test is the proof for
//! whichever platform it is run on.

use std::ffi::{CStr, CString};

use fontelle_vst2::bridge_abi::{FontelleBridge, PluginInfo};
use fontelle_vst2::fontelle_bridge_entry;

fn bridge() -> &'static FontelleBridge {
    // SAFETY: the entry point returns a pointer to a 'static table.
    unsafe { &*fontelle_bridge_entry() }
}

fn plugin_path() -> Option<String> {
    std::env::var("FONTELLE_VST2_PLUGIN")
        .ok()
        .filter(|p| !p.is_empty())
}

#[test]
#[ignore = "needs a real VST 2 plugin; set FONTELLE_VST2_PLUGIN"]
fn it_loads_scans_and_sounds_a_real_plugin() {
    let Some(path) = plugin_path() else {
        panic!("set FONTELLE_VST2_PLUGIN to a plugin path");
    };
    let bridge = bridge();
    let c_path = CString::new(path.clone()).unwrap();

    // Scan: the plugin names itself, and says whether it is an instrument.
    let mut infos: *mut PluginInfo = std::ptr::null_mut();
    let mut count: u32 = 0;
    // SAFETY: valid out-params.
    let rc = unsafe { (bridge.scan_bundle)(c_path.as_ptr(), &mut infos, &mut count) };
    assert_eq!(rc, 0, "the scan of {path} failed");
    assert!(count >= 1, "a VST 2 bundle holds at least one plugin");
    // SAFETY: the scan filled `count` valid entries.
    let (name, vendor, version, is_instrument) = unsafe {
        let info = &*infos;
        (
            CStr::from_ptr(info.name).to_string_lossy().into_owned(),
            CStr::from_ptr(info.vendor).to_string_lossy().into_owned(),
            CStr::from_ptr(info.version).to_string_lossy().into_owned(),
            info.is_instrument != 0,
        )
    };
    // SAFETY: frees what the scan allocated.
    unsafe { (bridge.free_infos)(infos, count) };
    eprintln!("scanned: {name:?} by {vendor:?} v{version} instrument={is_instrument}");
    assert!(!name.is_empty(), "the plugin has a name");

    // Open and drive it.
    let empty = CString::new("").unwrap();
    // SAFETY: the ABI calls in the order the contract requires.
    unsafe {
        let inst = (bridge.open)(c_path.as_ptr(), empty.as_ptr());
        assert!(!inst.is_null(), "{name} opened");

        let inputs = (bridge.audio_inputs)(inst);
        let outputs = (bridge.audio_outputs)(inst);
        let params = (bridge.param_count)(inst);
        let notes = (bridge.accepts_notes)(inst) != 0;
        eprintln!("  in={inputs} out={outputs} params={params} accepts_notes={notes}");
        assert!(outputs >= 1, "{name} has an output to listen to");

        assert_eq!(
            (bridge.activate)(inst, 48_000.0, 512),
            0,
            "{name} activated"
        );

        let frames = 512usize;
        let out_ch = outputs.max(1) as usize;
        let mut out_bufs: Vec<Vec<f32>> = vec![vec![0.0; frames]; out_ch];
        let in_ch = inputs as usize;
        // An instrument is silent until played; an effect needs signal in.
        let in_bufs: Vec<Vec<f32>> = if is_instrument || inputs == 0 {
            (0..in_ch).map(|_| vec![0.0; frames]).collect()
        } else {
            (0..in_ch)
                .map(|_| (0..frames).map(|i| (i as f32 * 0.1).sin() * 0.5).collect())
                .collect()
        };

        if notes {
            (bridge.note_on)(inst, 0, 60, 0.9);
        }

        // Run a few blocks: an instrument's envelope may take a moment to open.
        let mut peak = 0.0f32;
        for _ in 0..8 {
            let in_ptrs: Vec<*const f32> = in_bufs.iter().map(|b| b.as_ptr()).collect();
            let out_ptrs: Vec<*mut f32> = out_bufs.iter_mut().map(|b| b.as_mut_ptr()).collect();
            (bridge.process)(
                inst,
                if in_ptrs.is_empty() {
                    std::ptr::null()
                } else {
                    in_ptrs.as_ptr()
                },
                inputs,
                out_ptrs.as_ptr(),
                outputs,
                frames as u32,
            );
            for buf in &out_bufs {
                for &s in buf {
                    peak = peak.max(s.abs());
                }
            }
        }
        eprintln!("  output peak over 8 blocks: {peak:.4}");

        if notes {
            assert!(
                peak > 1e-4,
                "{name} is an instrument but a note produced silence (peak {peak})"
            );
        }

        (bridge.deactivate)(inst);
        (bridge.close)(inst);
    }
    eprintln!("OK: {name} loaded, ran, and produced audio through the bridge");
}
