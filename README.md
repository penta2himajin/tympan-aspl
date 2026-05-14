English | [日本語](README.ja.md)

# tympan-aspl

A Rust framework for writing macOS AudioServerPlugins.

`tympan-aspl` provides Rust abstractions over the Core Audio HAL
AudioServerPlugin interface, enabling Rust applications to implement
custom virtual audio devices and audio drivers on macOS without
writing C++ or Objective-C.

## Status

**Early implementation.** The cross-platform foundation has landed: the
realtime primitives (`RealtimeContext`, lock-free SPSC ring, atomic
lifecycle state machine, off-thread log sink), the Core Audio object
model (`OsStatus`, `FourCharCode`, `AudioObjectId`, `PropertyAddress`,
`StreamFormat`), the `Driver` / `Device` / `Stream` API surface, and
`.driver` bundle `Info.plist` generation — all unit-tested and CI-gated.

The macOS FFI bridge (`raw`) and the `coreaudiod` HAL-load test tiers
are next; see
[`docs/decisions/0001-ci-verification-strategy.md`](docs/decisions/0001-ci-verification-strategy.md).
See [`docs/overview.md`](docs/overview.md) for scope and
[`docs/architecture.md`](docs/architecture.md) for the API design.

## Naming

*Tympan* — the tympanal organ of moths, a membrane-based ultrasound sensor
on the abdomen of pyralid and noctuid moths. Evolved to detect the
echolocation calls of bats. The name reflects the library's role: a thin
membrane between the OS audio engine and user-space Rust code.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

## Documentation

| Doc | Content |
|---|---|
| [`docs/overview.md`](docs/overview.md) | Project purpose, scope, comparison to existing implementations |
| [`docs/architecture.md`](docs/architecture.md) | API design and module layout |
| [`docs/plugin-author-guide.md`](docs/plugin-author-guide.md) | Writing a driver: setup, identity, the realtime path, packaging, pitfalls |
| [`docs/testing.md`](docs/testing.md) | Testing and CI strategy across tiers |
| [`docs/references.md`](docs/references.md) | Apple documentation, prior art, related crates |
| [`docs/handoff-protocol.md`](docs/handoff-protocol.md) | Session handoff protocol for long-running work |
