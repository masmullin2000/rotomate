# List available recipes
default:
    @just --list

# Compile the project
build target="debug" toolchain="musl" version="stable":
    #!/usr/bin/env bash
    # Set environment variable for release build
    CARGO_BUILD_FLAG=""
    if [ "{{ target }}" = "release" ] || [ "{{ target }}" = "rel" ]; then
        CARGO_BUILD_FLAG="--release"
    fi
    if [ "{{ toolchain }}" = "musl" ]; then
        CARGO_BUILD_FLAG="$CARGO_BUILD_FLAG --target x86_64-unknown-linux-musl"
    fi

    cargo +{{ version }} build $CARGO_BUILD_FLAG

# Check the project for errors without building
check:
    cargo check

# Run the project
run *ARGS:
    cargo run -r --target x86_64-unknown-linux-musl {{ARGS}}

# Run tests
test:
    cargo test

# Run clippy with pedantic and nursery lints
clippy toolchain="stable":
    cargo +{{ toolchain }} clippy -- -W clippy::pedantic -W clippy::nursery

# Clean build artifacts
clean:
    cargo clean

# Run with single test config
test-run:
    just run -- run -c config/single.yaml -v --root

# Run with split config files
test-run-split:
    just run -- run -c config/hosts.yaml -c config/runs.yaml -c config/tasks.yaml -v --root

# Run with config that uses imports
test-run-imports:
    just run -- run -c config/main.yaml -v

# Test upload performance (release mode)
test-upload count="100": (build "release")
    #!/usr/bin/env bash
    echo "Creating {{ count }}MB test file..."
    dd if=/dev/zero of=/tmp/testfile bs=1M count={{ count }} 2>/dev/null
    echo "Running upload test (release)..."
    # cargo build -r
    time ./target/x86_64-unknown-linux-musl/release/rot run -c config/upload_test.yaml

# Test upload with debug logging (release mode)
test-upload-debug count="100":
    #!/usr/bin/env bash
    echo "Creating {{ count }}MB test file..."
    dd if=/dev/zero of=/tmp/testfile bs=1M count={{ count }} 2>/dev/null
    echo "Running upload test (release + debug logs)..."
    cargo build -r
    RUST_LOG=debug cargo run --release -- run -c config/upload_test.yaml 2>&1

# Test upload with debug build (slow, for debugging only)
test-upload-debug-build count="100":
    #!/usr/bin/env bash
    echo "Creating {{ count }}MB test file..."
    dd if=/dev/zero of=/tmp/testfile bs=1M count={{ count }} 2>/dev/null
    echo "Running upload test (debug build - SLOW)..."
    RUST_LOG=debug cargo run -- run -c config/upload_test.yaml 2>&1
