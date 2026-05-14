# ADR 0001: CI verification strategy and scope boundary

- Status: Accepted
- Date: 2026-05-14

## Context

`tympan-aspl` is a macOS-only framework: the `raw` module links
`CoreAudio` / `CoreFoundation` and the `.driver` bundle is loaded by
`coreaudiod`. `CLAUDE.md` lists five explicit prohibitions, the
strongest being an allocation-free, lock-free `IOProc` path. The
question is which of these can be verified mechanically on
GitHub-hosted runners, and which must fall back to local or
self-hosted testing.

`docs/testing.md` already surveyed the runner landscape and the
System Integrity Protection (SIP) constraints. Findings relevant to
CI scoping:

- **GitHub-hosted `macos-15` / `macos-14` runners** ship Xcode and
  the Command Line Tools out of the box: `clang`, `lipo`,
  `codesign`, `plutil`, `nm`, `otool`, `system_profiler`,
  `launchctl`. Public repositories receive unlimited free minutes on
  standard runners.
- **SIP does not prevent loading unsigned HAL plug-ins.** It blocks
  debugger attachment to `coreaudiod`. Tier 3 HAL-load verification
  therefore works on stock runners; deep debugging does not.
- **The cross-platform layers compile and test on any host.** The
  `realtime`, `error`, `format`, `property`, and `bundle` modules
  contain no `cfg(target_os = "macos")` code, so their invariants
  (lock-free ring buffer, atomic state machine, Info.plist
  generation) are unit-testable on the macOS runner without a HAL
  round-trip.
- **Audio I/O verification needs hardware** the runners do not
  expose, so it stays manual.

Industry baseline in adjacent Rust audio projects (`coreaudio-rs`,
`cpal`): CI typically stops at `cargo build` and `cargo test` with no
plug-in lifecycle exercising. Adopting bundle/ABI verification and a
`coreaudiod` HAL-load harness puts this project above that baseline,
on par with the sibling `tympan-apo` and `tympan-ladspa` crates.

## Decision

CI verification is organised in four tiers. Each tier defines what
runs on which trigger and what is intentionally out of scope. The
operational details (commands, runner labels, fixtures) live in
[`docs/testing.md`](../testing.md); the tier boundaries below are
authoritative. CI is built up incrementally as implementation lands —
this ADR describes the target state, and the `.github/workflows/`
directory tracks how much of it is wired up so far.

### Tier 1 — `static` (every PR push, target < 7 min)

- `cargo build --release --all-targets`
- `cargo test --all-targets` for the cross-platform unit tests
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo doc --no-deps --document-private-items` with
  `RUSTDOCFLAGS=-D warnings`
- A `git grep` for `static mut` in `src/` enforcing the
  no-global-state invariant
- `cargo deny` for supply-chain hygiene

This tier blocks merge. **Wired up: yes** (`tier1.yml`).

### Tier 2 — `bundle` (every PR push, target < 10 min)

Tier 1 plus:

- `plutil -lint` over committed `Info.plist` fixtures and over
  Info.plists emitted by the `bundle` module
- `nm -gU` symbol-visibility check once the example builds a
  `.driver` bundle — only the CFPlugIn factory symbol is exported
- `lipo -info` architecture-coverage check
- `codesign -v` ad-hoc signature verification
- Compile-time `static_assertions` for bridged struct sizes against
  the `coreaudio-sys` C definitions

This tier blocks merge. **Wired up: partially** (`tier2.yml`,
`plutil -lint` over the example Info.plist; the symbol / `lipo` /
`codesign` steps land with the first buildable `.driver`).

### Tier 3 — `hal-load` (merge to `main` and nightly, target < 25 min)

Tier 2 plus:

- `sudo cp -R` the built `.driver` into `/Library/Audio/Plug-Ins/HAL/`
- `sudo launchctl kill` `coreaudiod` and wait for relaunch
- `system_profiler SPAudioDataType` confirms the example device is
  enumerated
- An in-process lifecycle harness driving
  `Initialize → StartIO → IOProc ×N → StopIO` under an
  `assert_no_alloc` global-allocator guard

This tier does not block PR merge; failure on `main` opens an issue.
**Wired up: no** (lands with the first buildable `.driver`).

### Tier 4 — out of CI scope

Not tested on GitHub-hosted runners; documented in `docs/testing.md`
(§ Tier 4) as a pre-release manual checklist: audio output to
physical hardware, microphone capture, long-running stability,
notarization-gated behaviour, third-party application interaction,
and System Settings UI verification.

## Consequences

Positive:

- The cross-platform invariants (realtime safety, state machine
  correctness, Info.plist well-formedness) are mechanically enforced
  on every PR from the very first commit, before any FFI lands.
- The tier numbering maps cleanly onto the sibling tympan crates'
  conventions, so contributors moving between crates see consistent
  CI semantics.
- Tier 3's `coreaudiod` HAL-load harness catches the most common
  regression modes (Info.plist drift, CFPlugIn factory wiring, IO
  lifecycle ordering) on `main` and nightly without any macOS
  infrastructure beyond a hosted runner.

Negative:

- Audio-data-path bugs (format negotiation, sample-rate conversion,
  glitching under load) are not caught until manual verification.
- Notarization-related issues surface only at release-time signing.
- Until the first `.driver` example builds, Tier 2 is limited to
  Info.plist linting and Tier 3 is not wired up at all.

## Trigger for revisiting

Re-evaluate when any of the following holds:

- A self-hosted Mac runner with a virtual audio device becomes part
  of the project's CI budget — at that point Tier 4 audio-I/O
  verification is promoted into a workflow.
- A bug ships that would have been caught by `coreaudiod`-level
  loading but was not caught by the Tier 3 harness.
- Apple ships a first-party AudioServerPlugin test host; if that
  lands, evaluate whether it replaces or supplements the in-process
  lifecycle harness.

## References

- Apple: *Creating an Audio Server Driver Plug-in*
  - <https://developer.apple.com/documentation/coreaudio/creating-an-audio-server-driver-plug-in>
- GitHub-hosted macOS runner image inventory
  - <https://github.com/actions/runner-images/tree/main/images/macos>
- `assert_no_alloc` crate — <https://docs.rs/assert_no_alloc>
- Sibling tympan CI strategies:
  - `tympan-apo` ADR 0001 (Windows, in-process COM activation)
  - `tympan-ladspa` ADR 0005 (LADSPA, `applyplugin`-based Tier 2)
- `docs/testing.md` — tiered verification strategy and runner notes
