# some_global_executor

Cross-platform global executor implementation for the `some_executor` framework.

This crate provides a thread pool-based executor that works seamlessly on both standard platforms and WebAssembly (WASM) targets. It implements the `SomeExecutor` trait from the `some_executor` framework and provides efficient task scheduling with configurable parallelism.

## Architecture Overview

The executor uses platform-specific implementations through the `sys` module:
- **Standard platforms**: Uses OS threads with `crossbeam-channel` for task distribution
- **WebAssembly**: Uses web workers for parallelism in browser environments

The platform abstraction is completely transparent to users - the same API works across all supported platforms.

## Key Features

- **Cross-platform support**: Automatic platform detection and optimal implementation selection
- **Dynamic thread pools**: Create executors with custom thread counts and resize them at runtime
- **Task observation**: Monitor task execution state through the observer pattern
- **Graceful shutdown**: Both synchronous and asynchronous draining of pending tasks
- **Global executor support**: Set executors as global or thread-local defaults
- **Zero-cost abstractions**: Platform-specific code is conditionally compiled

## Usage Patterns

### Basic Task Spawning

The most common use case is creating an executor and spawning tasks:

```rust
use some_global_executor::Executor;
use some_executor::SomeExecutor;
use some_executor::task::{Task, Configuration};

// Create an executor with 4 worker threads
let mut executor = Executor::new("my-executor".to_string(), 4);

// Spawn a simple async task
let task = Task::without_notifications(
    "example-task".to_string(),
    Configuration::default(),
    async {
        // Perform async work here
        println!("Task executing on worker thread!");
        42
    }
);

let observer = executor.spawn(task);
// Task is now running in the background on one of the worker threads

// Wait for all tasks to complete before shutting down
executor.drain();
```

### Observing Task Progress

Tasks can be observed to monitor their execution state:

```rust
use some_global_executor::Executor;
use some_executor::SomeExecutor;
use some_executor::task::{Task, Configuration};
use some_executor::observer::{Observer, Observation};

let mut executor = Executor::new("observer-example".to_string(), 2);

let task = Task::without_notifications(
    "monitored-task".to_string(),
    Configuration::default(),
    async { "result" }
);

let observer = executor.spawn(task);

// Poll the observer to check task state
loop {
    match observer.observe() {
        Observation::Ready(value) => {
            println!("Task completed with: {}", value);
            break;
        }
        Observation::Pending => {
            // Task still running
            std::thread::yield_now();
        }
        _ => break,
    }
}

executor.drain();
```

### Global Executor Pattern

Set an executor as the global default for the application:

```rust
use some_global_executor::Executor;

// Create and configure the global executor
let executor = Executor::new("global".to_string(), num_cpus::get());
executor.set_as_global_executor();

// Now tasks can be spawned using the global executor from anywhere
// in the application without passing executor references
```

### Dynamic Thread Pool Management

Adjust executor capacity based on workload:

```rust
use some_global_executor::Executor;

let mut executor = Executor::new("dynamic".to_string(), 2);

// Scale up for heavy workload
executor.resize(8);

// Scale down during idle periods
executor.resize(2);

executor.drain();
```

## Performance Considerations

- Thread pool sizing: Default to `num_cpus::get()` for CPU-bound work
- For I/O-bound tasks, consider using more threads than CPU cores
- WASM targets have platform-specific limitations on parallelism
- Use `drain_async()` in async contexts to avoid blocking

## Logging

This crate uses the `logwise` framework for structured logging. Internal operations are logged at various levels for debugging and monitoring:

```rust
// Executor creation and operations are automatically logged
let executor = Executor::new("logged-executor".to_string(), 4);
// Logs: "Creating executor with name logged-executor and 4 threads"
```

## Build and Development

### Standard Testing
```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture
```

### WebAssembly Testing
```bash
# Run WASM tests with special flags for atomics support
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER="wasm-bindgen-test-runner" \
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals' \
cargo +nightly test --target wasm32-unknown-unknown -Z build-std=std,panic_abort
```

### Documentation
```bash
# Generate documentation with warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

# Open documentation in browser
cargo doc --open
```

## Dependencies

The project uses several local dependencies that may be patched in `Cargo.toml`:
- `logwise`: Logging framework
- `some_executor`: Core executor traits
- `test_executors`: Testing utilities
- `wasm_safe_mutex`: WASM-safe mutex implementation

## License

[License information would typically go here]