# OOMU developer tools

These binaries live in a separate, non-default Cargo package and are excluded from normal application builds. Run them only from a trusted local checkout:

```bash
cargo run --manifest-path tools/developer-tools/Cargo.toml --features benchmark --bin oomu_bench -- <arguments>
cargo run --manifest-path tools/developer-tools/Cargo.toml --features database-tools --bin sanitize_release_db -- <database-path>
cargo run --manifest-path tools/developer-tools/Cargo.toml --features ark-verify --bin ark_verify -- <artifact-path>
cargo run --manifest-path tools/developer-tools/Cargo.toml --features database-tools --bin debug_db -- <arguments>
cargo run --manifest-path tools/developer-tools/Cargo.toml --features database-tools --bin debug_executions -- <arguments>
cargo run --manifest-path tools/developer-tools/Cargo.toml --bin stage_pre_alpha -- <arguments>
```

The Tauri package has no binary targets for these utilities. The canonical bundle gate also rejects every one of their names if it appears anywhere inside `OOMU.app`.
