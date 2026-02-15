# Metropolis Scripts

This directory contains utility scripts for the Metropolis project.

## dependency_order.py

Analyzes Cargo.toml files to generate a dependency graph of Drew Crawford's crates in build order (reverse dependency order).

### Usage

```bash
# Basic usage (scans ~/Code by default)
python3 scripts/dependency_order.py

# Specify a different root directory
python3 scripts/dependency_order.py --root-dir /path/to/crates

# Filter by a different author
python3 scripts/dependency_order.py --author "Your Name"

# Output as JSON
python3 scripts/dependency_order.py --format json

# Get help
python3 scripts/dependency_order.py --help
```

### What It Does

1. **Scans** the specified directory for all `Cargo.toml` files
2. **Filters** crates by author (default: "Drew Crawford")
3. **Builds** a dependency graph showing which crates depend on which
4. **Sorts** crates into tiers using topological sort:
   - Tier 1: No dependencies on other Drew Crawford crates
   - Tier 2: Depends only on Tier 1
   - Tier 3: Depends on Tier 1-2, etc.
5. **Detects** circular dependencies and lists them separately

### Output

The script produces a markdown-formatted list showing:
- Crates grouped by tier (build order)
- Dependencies for each crate
- Circular dependency groups (if any)
- Total count of crates and tiers

### Example Output

```
# Drew Crawford's Crates in Build Order (Reverse Dependency Order)

## Tier 1
(No dependencies on other Drew Crawford crates)

1. **vectormatrix** (no dependencies)
1. **tgar** (no dependencies)

## Tier 2
(Depends on Tier 1-1 only)

2. **some_executor**
   - Depends on: continue

## Cyclic Dependencies (5 crates)
These crates have circular dependencies and cannot be ordered:

- **continue**
  - cyclic: logwise
- **logwise**
  - cyclic: wasm_safe_mutex
...
```

### Use Cases

- **Publishing to crates.io**: Publish crates in tier order to ensure dependencies are available
- **Understanding architecture**: See the dependency structure of your crates
- **Detecting cycles**: Find circular dependencies that might need refactoring
- **Build planning**: Know which crates can be built in parallel (same tier)

### Notes

- The script only analyzes `[dependencies]` sections, not `[dev-dependencies]` or `[build-dependencies]`
- Circular dependencies are detected and shown separately since they cannot be ordered
- Warnings and diagnostic messages are sent to stderr, while the report goes to stdout
