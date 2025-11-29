# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] - 2025-11-28

### Changed
- Updated to logwise 0.4.0 for the latest logging goodness
- Bumped test_executors to 0.4.0 to keep our test infrastructure fresh

### Added
- Shiny new build scripts in `scripts/` directory—now you can check, test, and build with ease for both native and wasm32 targets
- Claude Code agent configuration to help with future development
- Additional rustflags for even better builds

### Improved
- Streamlined CI workflow with a cleaner matrix configuration (v9)—because nobody likes messy automation
- Better gitignore coverage to keep your workspace tidy

## [0.1.0] - 2025-11-21

Initial release of some_global_executor—a cross-platform async task executor that works beautifully on both native platforms and WebAssembly.

### Added
- Cross-platform executor implementing the `SomeExecutor` trait
- Native support using OS threads with crossbeam-channel for task distribution
- WebAssembly support using web workers via wasm_thread
- Static executor for non-Send tasks on both platforms
- Custom waker implementations optimized for each platform
- Thread-local executors that run tasks on the same thread where spawned

[Unreleased]: https://github.com/drewcrawford/some_global_executor/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/drewcrawford/some_global_executor/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/drewcrawford/some_global_executor/releases/tag/v0.1.0
