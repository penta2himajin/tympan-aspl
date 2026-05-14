# gain

A `tympan-aspl` driver one step up from `minimal-loopback`: a stereo
virtual loopback device that applies a fixed linear gain to audio
passing through it. The macOS analogue of `tympan-apo`'s `gain`
example and `tympan-ladspa`'s `gain` example.

## What it demonstrates

- **Per-instance configuration** — the gain is held in a struct
  field, initialised by `Driver::new`, the place a real driver
  would read user settings or set up DSP coefficients.
- **Direction-aware processing** — a loopback device's `process_io`
  is called for *two* IO operations per client: `WriteMix` (audio
  entering the device) and `ReadInput` (audio leaving it). This
  driver branches on `IoBuffer::operation` and scales only on
  `WriteMix`, so the gain is applied exactly once across the
  loopback.
- A realtime-safe `process_io` body — a multiply-add loop, no
  allocation and no locks.

## Building

The crate builds two ways. As an `rlib`, its `Driver` implementation
is unit-tested on every host:

```bash
cargo test -p gain
```

As a `cdylib`, it is the loadable plug-in binary —
`tympan_aspl::plugin_entry!` emits the `TympanAsplDriverFactory`
CFPlugIn entry point `coreaudiod` resolves:

```bash
cargo build --release -p gain
# → target/release/libgain.dylib
```

## The bundle layout

The loadable `Gain.driver` bundle is the committed `Info.plist` plus
the built cdylib, in the CFBundle layout `coreaudiod` expects:

```text
Gain.driver/
└── Contents/
    ├── Info.plist          ← the file in this directory
    └── MacOS/
        └── Gain            ← the built cdylib
```

`Info.plist` is exactly what `tympan_aspl::bundle::plist::generate`
emits for `Gain::bundle_config()` — the
`committed_info_plist_matches_the_generator` unit test enforces it.

To assemble and install the bundle by hand:

```bash
BUNDLE=Gain.driver
mkdir -p "$BUNDLE/Contents/MacOS"
cp examples/gain/Info.plist "$BUNDLE/Contents/Info.plist"
cp target/release/libgain.dylib "$BUNDLE/Contents/MacOS/Gain"
codesign --force --sign - "$BUNDLE"

sudo cp -R "$BUNDLE" /Library/Audio/Plug-Ins/HAL/
sudo killall coreaudiod
```

and the device appears in System Settings ▸ Sound.
