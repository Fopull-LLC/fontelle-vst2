# Contributing to fontelle-vst2

Ordinary Rust contributions are welcome the usual way: an issue, a branch, a PR,
`cargo test` and `cargo clippy --workspace --all-targets -- -D warnings` green.

One part of this repository is special, and this document is the condition on
touching it.

## The clean-room rule (`src/vst2.rs` and the fixture's copy of the interface)

`crates/fontelle-vst2/src/vst2.rs` is a **clean-room** description of the
VST 2.4 plugin interface. VST 2 has not been licensable from Steinberg since
October 2018, and Steinberg's VST 2 headers (`aeffect.h`, `aeffectx.h`) may not
be copied, shared, or used to write compatible code. The `crates/vst2-fixture`
plugin contains a second, independent copy of the same interface for testing.

Every contributor who writes or changes either of those files affirms, in the
PR, all of the following:

1. **You have never read Steinberg's VST 2 SDK headers**, and you did not open,
   consult, or refer to them — or to any copy of them — while writing or
   changing this code.
2. Anything you wrote here you wrote from a **public, non-Steinberg**
   description of the interface: the documented `AEffect` calling convention,
   or a clean-room source that is itself not derived from Steinberg's headers —
   for example Xaymar's BSD-3-Clause `vst2sdk`, VeSTige (LMMS), or FST
   (Ardour). Cite which, in the PR.
3. You are **reproducing an interface for interoperability**, not copying
   creative expression. The `#[repr(C)]` layout is the binary contract a
   compiled plugin already expects; the names (`AEffect`, `processReplacing`,
   `effOpen`) are the interface's own words, used so the two programs can talk.

If you cannot affirm all three for a change, do not make it — open an issue
describing what is missing and let someone who can, do it.

Why this matters: reimplementing an interface so existing programs interoperate
is lawful (EU Software Directive art. 6; *Sega v. Accolade*; *Google v.
Oracle*), and it is how every open-source VST 2 host is built. Copying
Steinberg's files is not, and Steinberg has issued DMCA notices over copies of
its own headers. The whole value of this file is that it was written the first
way, and a single contribution made the second way would poison it.

## The rest of the repository

`src/lib.rs`, `src/host.rs`, `src/bridge_abi.rs`, the tests and the tooling are
ordinary MIT/Apache Rust. `src/bridge_abi.rs` mirrors Fontelle's
`fontelle-bridge-abi` and must stay in step with it — if it drifts, the loader
test stops passing, which is the point of the test.

By contributing you agree that your contributions are licensed under MIT OR
Apache-2.0, matching the project.
