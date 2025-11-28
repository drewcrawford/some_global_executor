# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
# Full check (format, build, clippy, tests, docs for both native and wasm32)
./scripts/check_all

# Individual checks
./scripts/check          # cargo check for native + wasm32
./scripts/clippy         # clippy for native + wasm32
./scripts/tests          # tests for native + wasm32
./scripts/docs           # docs for native + wasm32
./scripts/fmt            # cargo fmt

# Native-only
./scripts/native/check
./scripts/native/clippy
./scripts/native/tests
./scripts/native/docs

# WASM32-only
./scripts/wasm32/check
./scripts/wasm32/clippy
./scripts/wasm32/tests
./scripts/wasm32/docs

# Run single test (native)
cargo test test_name

# Run single test (wasm32) - requires nightly and special flags
```

## Architecture

This crate provides a cross-platform async task executor implementing the `SomeExecutor` trait from the `some_executor` framework.

### Platform Abstraction (`src/sys.rs`)

The `sys` module provides platform-specific implementations:
- **`stdlib.rs`**: Native platforms - uses OS threads with `crossbeam-channel` for task distribution
- **`wasm.rs`**: WebAssembly - uses web workers via `wasm_thread` crate

Both implementations share the same public API through conditional compilation (`#[cfg(target_arch = "wasm32")]`).

### Key Components

- **`Executor`** (`lib.rs`): Main public type wrapping platform-specific implementation. Manages thread pool, task spawning, and draining.
- **`DrainNotify`** (`lib.rs`): Tracks running task count with `AtomicUsize` and provides `AtomicWaker` for drain completion notification.
- **`StaticExecutor`** (`sys/stdlib.rs`, `sys/wasm/static_executor.rs`): Thread-local executor for non-Send tasks, runs on same thread where spawned.
- **`SpawnedTask`**: Wraps `DynSpawnedTask` with platform-specific waker implementation.

### Threading Model

Native: Threadpool with `crossbeam-channel` for work distribution. Each thread has its own `StaticExecutor` for non-Send tasks and processes from a shared task queue using `select_biased!`.

WASM: Web workers spawned via `wasm_thread`. Custom async channel (`sys/wasm/channel.rs`) for task distribution since WASM can't block. Each worker runs a `StaticExecutor` that provides async context for the worker loop.

### Waker Implementation

- Native (`waker.rs`): Custom `WakeInternal` with atomic flag for inline wake detection
- WASM (`sys/wasm.rs`): Raw waker vtable implementation that sends `TaskMessage::ResumeTask` through channel

## Dependencies

Uses `drewcrawford` crates: `logwise`, `some_executor`, `test_executors`, `wasm_safe_mutex`

Requires Rust edition 2024 (rust-version 1.88.0).
