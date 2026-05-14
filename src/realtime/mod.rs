//! Realtime-safe primitives.
//!
//! Everything in this module must be allocation-free and lock-free.
//! In particular, per `CLAUDE.md` § Prohibitions:
//!
//! - No `std::sync::Mutex`, `std::collections::HashMap`, `Vec::push`,
//!   `Box::new`, or other allocator-touching operations may appear
//!   in code reachable from the `IOProc` realtime callback.
//! - Errors are represented as `OSStatus`-style integers, never as
//!   heap-allocated `String`s.
//!
//! The module is intentionally cross-platform: the realtime
//! invariants do not depend on any macOS-specific API, and being
//! able to unit-test them on any host is more valuable than gating
//! them behind `cfg(target_os = "macos")`.
//!
//! ## Public surface
//!
//! - [`RealtimeContext`] — a zero-sized compile-time witness that
//!   the current call stack originates from the audio engine's
//!   realtime thread.
//! - [`Refcount`] — a wait-free atomic reference counter for the
//!   CFPlugIn `IUnknown` ref-counting contract.
//! - [`State`] / [`StateCell`] — the atomic AudioServerPlugin
//!   lifecycle state machine (`Uninitialized → Initialized →
//!   Running`).
//! - [`channel`] / [`Producer`] / [`Consumer`] — a lock-free SPSC
//!   ring buffer.
//! - [`log`] — a bounded realtime log sink with an off-thread
//!   drainer, built on the ring.

mod context;
pub mod log;
mod refcount;
pub mod ring;
mod state;

pub use context::RealtimeContext;
pub use refcount::Refcount;
pub use ring::{channel, Consumer, Producer};
pub use state::{State, StateCell, TransitionError};
