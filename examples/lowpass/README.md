# lowpass

The `tympan-aspl` example with genuine per-instance *processing
state*: a stereo virtual loopback device that runs audio through a
one-pole low-pass filter. Where `minimal-loopback` is stateless and
`gain` carries only fixed configuration, this driver carries the
filter's running memory and evolves it every IO cycle.

## What it demonstrates

- **Per-instance processing state** — the filter memory (one sample
  per channel) lives in the struct, is created by `Driver::new`, and
  persists across `process_io` calls.
- **Resetting state in `Driver::start_io`** — a fresh IO session
  must not hear the tail of the previous one, so `start_io` clears
  the filter memory. `start_io` is also where a driver may
  *allocate*; `process_io` may not.
- **Direction-aware processing** — like the `gain` example, the
  filter is applied on the `WriteMix` operation only, so a sample
  crossing the loopback is filtered exactly once.
- A realtime-safe `process_io` body — a multiply-add recurrence, no
  allocation and no locks.

## The filter

A one-pole low-pass run independently per channel:

```text
y[n] = y[n-1] + α·(x[n] − y[n-1])
```

`ALPHA` is the smoothing coefficient — smaller is a lower cutoff.

## Building

The crate builds two ways. As an `rlib`, its `Driver` implementation
is unit-tested on every host:

```bash
cargo test -p lowpass
```

As a `cdylib`, it is the loadable plug-in binary —
`tympan_aspl::plugin_entry!` emits the `TympanAsplDriverFactory`
CFPlugIn entry point `coreaudiod` resolves:

```bash
cargo build --release -p lowpass
# → target/release/liblowpass.dylib
```

## The bundle layout

The loadable `LowPass.driver` bundle is the committed `Info.plist`
plus the built cdylib, in the CFBundle layout `coreaudiod` expects:

```text
LowPass.driver/
└── Contents/
    ├── Info.plist          ← the file in this directory
    └── MacOS/
        └── LowPass         ← the built cdylib
```

`Info.plist` is exactly what `tympan_aspl::bundle::plist::generate`
emits for `LowPass::bundle_config()` — the
`committed_info_plist_matches_the_generator` unit test enforces it.

To assemble and install the bundle by hand:

```bash
BUNDLE=LowPass.driver
mkdir -p "$BUNDLE/Contents/MacOS"
cp examples/lowpass/Info.plist "$BUNDLE/Contents/Info.plist"
cp target/release/liblowpass.dylib "$BUNDLE/Contents/MacOS/LowPass"
codesign --force --sign - "$BUNDLE"

sudo cp -R "$BUNDLE" /Library/Audio/Plug-Ins/HAL/
sudo killall coreaudiod
```

and the device appears in System Settings ▸ Sound.
