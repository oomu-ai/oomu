# Development setup

OOMU is a macOS Tauri application with a Next.js renderer, a Rust backend, native inference, and generated helper runtimes. Generated dependencies, models, application state, and binaries are intentionally absent from the repository.

## Prerequisites

- macOS 14 or later
- Xcode command-line tools
- Node.js 22.17.0 and npm 10.9.2
- Rust 1.95.0 with the `aarch64-apple-darwin` target
- CMake, LLVM, and Python 3

Install the common native packages with Homebrew:

```sh
xcode-select --install
brew install cmake llvm python
```

## Install and run

Use the committed lockfiles:

```sh
npm ci
npm run tauri:dev
```

To exercise first-launch onboarding without deleting a normal profile:

```sh
npm run tauri:dev:first-run
```

The development command prepares the local inference and native helper dependencies it needs. Connector credential bundles remain local and ignored; see `src-tauri/oauth/README.md`.

## Validate

```sh
npm run check:quality
npm run test:frontend
CARGO_TARGET_DIR=/private/tmp/oomu-cargo-target cargo check --manifest-path src-tauri/Cargo.toml --locked
CARGO_TARGET_DIR=/private/tmp/oomu-cargo-target cargo test --manifest-path src-tauri/Cargo.toml --locked
```

Production signing and notarization are local, protected release operations. They are never performed in GitHub workflows.
