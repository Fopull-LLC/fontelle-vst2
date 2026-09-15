//! The loader, end to end: the bridge scans, opens, and runs the fixture
//! plugin — two independent transliterations of the VST 2.4 ABI interoperating.
//!
//! This is what says the bridge speaks the interface correctly without a real
//! third-party plugin on hand. The fixture (`vst2-fixture`) is a gain, so the
//! test can assert the exact number that comes out: input times the parameter.

use std::ffi::{CStr, CString};
use std::path::PathBuf;

use fontelle_vst2::bridge_abi::{FontelleBridge, PluginInfo};
use fontelle_vst2::fontelle_bridge_entry;

/// Where Cargo built the fixture's shared library. The test binary runs from
/// `target/<profile>/deps/`, and the cdylib sits one level up.
fn fixture_so() -> PathBuf {
    let exe = std::env::current_exe().expect("a test binary has a path");
    // .../target/<profile>/deps/<test> -> .../target/<profile>
    let profile_dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/<profile> is two above the test binary");
    // The cdylib can sit at the profile root and/or under deps/. `cargo test`
    // refreshes the deps/ copy but does not always uplift the root one, so a
    // stale root copy can shadow the fresh build — pick whichever is newest.
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for name in [
        "libvst2_fixture.so",
        "vst2_fixture.dll",
        "libvst2_fixture.dylib",
    ] {
        for candidate in [profile_dir.join(name), profile_dir.join("deps").join(name)] {
            if let Ok(modified) = std::fs::metadata(&candidate).and_then(|m| m.modified())
                && newest.as_ref().is_none_or(|(t, _)| modified >= *t)
            {
                newest = Some((modified, candidate));
            }
        }
    }
    if let Some((_, path)) = newest {
        return path;
    }
    panic!(
        "the fixture plugin was not built next to the test — expected \
         libvst2_fixture.so under {}",
        profile_dir.display()
    );
}

fn bridge() -> &'static FontelleBridge {
    // SAFETY: the entry point returns a pointer to a 'static table.
    unsafe { &*fontelle_bridge_entry() }
}

#[test]
fn the_table_is_the_abi_the_host_expects() {
    let bridge = bridge();
    assert_eq!(bridge.abi_version, 3, "the host refuses any other number");
    // SAFETY: 'static NUL-terminated strings.
    let format = unsafe { CStr::from_ptr(bridge.format) };
    assert_eq!(format.to_str().unwrap(), "vst2");
    let ext = unsafe { CStr::from_ptr(bridge.extension) };
    assert_eq!(ext.to_str().unwrap(), "so");
}

#[test]
fn it_scans_the_fixture_and_names_it() {
    let bridge = bridge();
    let path = CString::new(fixture_so().to_string_lossy().into_owned()).unwrap();
    let mut infos: *mut PluginInfo = std::ptr::null_mut();
    let mut count: u32 = 0;
    // SAFETY: the pointers are valid out-params.
    let rc = unsafe { (bridge.scan_bundle)(path.as_ptr(), &mut infos, &mut count) };
    assert_eq!(rc, 0, "the scan succeeds");
    assert_eq!(count, 1, "a VST 2 .so is one plugin");
    // SAFETY: the scan filled `count` valid entries.
    let name = unsafe { CStr::from_ptr((*infos).name) }.to_str().unwrap();
    assert_eq!(name, "Fixture Gain");
    let is_instrument = unsafe { (*infos).is_instrument };
    assert_eq!(is_instrument, 0, "the fixture is an effect, not a synth");
    unsafe { (bridge.free_infos)(infos, count) };
}

#[test]
fn it_opens_the_fixture_and_the_gain_actually_applies() {
    let bridge = bridge();
    let path = CString::new(fixture_so().to_string_lossy().into_owned()).unwrap();
    let empty = CString::new("").unwrap();

    // SAFETY: the ABI calls, in the order the contract requires — open,
    // activate, then audio.
    unsafe {
        let inst = (bridge.open)(path.as_ptr(), empty.as_ptr());
        assert!(!inst.is_null(), "the fixture opens");

        assert_eq!((bridge.audio_inputs)(inst), 2);
        assert_eq!((bridge.audio_outputs)(inst), 2);
        assert_eq!((bridge.param_count)(inst), 1);

        assert_eq!((bridge.activate)(inst, 48_000.0, 64), 0);

        // Half gain.
        (bridge.set_param)(inst, 0, 0.5);
        assert!(((bridge.get_param)(inst, 0) - 0.5).abs() < 1e-6);

        let frames = 64usize;
        let left_in = vec![1.0f32; frames];
        let right_in = vec![1.0f32; frames];
        let mut left_out = vec![0.0f32; frames];
        let mut right_out = vec![0.0f32; frames];

        let inputs = [left_in.as_ptr(), right_in.as_ptr()];
        let outputs = [left_out.as_mut_ptr(), right_out.as_mut_ptr()];
        (bridge.process)(inst, inputs.as_ptr(), 2, outputs.as_ptr(), 2, frames as u32);

        // A steady 1.0 through a 0.5 gain is a steady 0.5.
        assert!(
            left_out.iter().all(|&s| (s - 0.5).abs() < 1e-6),
            "the gain did not apply: {:?}",
            &left_out[..4]
        );
        assert!(right_out.iter().all(|&s| (s - 0.5).abs() < 1e-6));

        (bridge.deactivate)(inst);
        (bridge.close)(inst);
    }
}
