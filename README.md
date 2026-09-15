# fontelle-vst2

The **VST 2 extension** for the [Fontelle](https://github.com/Fopull-LLC/DAW-Fontelle)
DAW: a bridge that lets Fontelle load VST 2.4 plugins.

It is a separate download from a separate repository on purpose. Fontelle
itself hosts VST 3 (MIT-licensed), CLAP and LV2 in its own tree; VST 2 lives
here, out of that tree, so Fontelle's own licence audit is about Fontelle and
so that this support can be added or withdrawn as a download rather than as a
change to the product. See `docs/vst-plan.md` §3 in the DAW repository.

## What it is

A shared library that implements Fontelle's bridge ABI (version 3) and loads
VST 2.4 plugins through a **clean-room** description of the interface — no
Steinberg SDK, no Steinberg headers, nothing under Steinberg's licence. On
Linux it loads `.so` plugins, including the ones
[`yabridge`](https://github.com/robbert-vdh/yabridge) presents Windows plugins
as.

Fontelle finds the built library in its bridges folder
(`$XDG_DATA_HOME/fontelle/bridges/`) and offers to install it from its
Extensions page.

## What it does

Scan, open, parameters, audio, notes and a performance (wheel, bend,
aftertouch), state (for plugins that keep a chunk), and the plugin's own editor
embedded in a window Fontelle owns. Two independent transliterations of the
ABI — this bridge and the test fixture — load and run each other in
`crates/fontelle-vst2/tests/loads.rs`, which is what says the bridge speaks the
interface correctly without a third-party plugin on hand.

Known limits, each a bounded addition rather than a rethink: shell plugins
(several plugins behind one file) are read as one; per-note pitch is sent as a
channel bend, which is what VST 2 carries; Windows `.dll` loading rides on the
same code path but has not been exercised on Windows yet.

## Building

```sh
cargo build --release
cargo test
```

The release archive drops `libfontelle_vst2.so` into the bridges folder; that
is the whole of the install.

## Licence

MIT OR Apache-2.0, at your option — the same as Fontelle. See `LICENSE-MIT` and
`LICENSE-APACHE`.

VST is a registered trademark of Steinberg Media Technologies GmbH. This
project describes a file *format* it loads; it uses no VST logo and puts "VST"
in no product name.

## Contributing

**Read `CONTRIBUTING.md` before touching `src/vst2.rs`.** The interface
description there is clean-room, and keeping it that way is a condition of
every contribution to it.
