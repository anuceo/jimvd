# JimVD

A Rust prototype of a factorized ("rectangle"-based) database execution model. See `README.md` for the conceptual overview.

## Cursor Cloud specific instructions

### Toolchain
- This crate uses `edition = "2024"` (see `Cargo.toml`), which requires Rust **≥ 1.85**. The base image's default `rustup` toolchain may be older (e.g. 1.83) and will fail to compile. The startup/update script runs `rustup default stable` to ensure a compatible toolchain; there is no committed `rust-toolchain.toml`.

### Services / how to run
- This is a single Rust crate with two binaries; there is **no long-running service** to start. Run them directly from the repo root:
  - `cargo run --bin jimvd` — runs the main IAM benchmark (reads `benchmarks/workload_iam.json`).
  - `cargo run --bin adversarial_test` — runs the two-phase adversarial workload (reads `benchmarks/adversarial_config.json`) and writes `adversarial_snapshots.json` (gitignored).
- Both binaries read their config via **relative paths**, so they must be invoked from the repository root.

### PostgreSQL is NOT required (despite the README)
- `README.md` describes a PostgreSQL metadata catalog (and `src/catalog.rs` contains `tokio-postgres` code), but the catalog is **not wired into either binary** at runtime — `Catalog::connect` is never called outside `catalog.rs`. You do **not** need Docker or Postgres to build, test, or run the current prototype.

### Julia analysis tooling is optional
- `adversarial_test` optionally invokes Julia scripts in `julia/` to plot metrics. If Julia is absent the binaries print a "not installed — skipping" notice and exit successfully. Install it only if you need the plots: `bash scripts/setup_julia.sh` (downloads Julia via juliaup).

### Lint / test / build
- Lint: `cargo clippy --all-targets` (currently emits warnings only, no errors).
- Test: `cargo test` (no tests are defined yet; suite is empty but passes).
- Build: `cargo build`.
