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
- Build: `cargo build` (root crate) or `cargo build --workspace` (includes the DuckDB runner).

### Workspace layout & the DuckDB runner (non-obvious)
- The repo is a Cargo **workspace**: the root `jimvd` crate plus
  `benchmark-suite/duckdb_runner`. Binaries: `jimvd`, `adversarial_test`,
  `scaling_wall`, `gen_dataset` (root crate) and `duckdb_runner` (member crate).
- `duckdb_runner` depends on `duckdb` with the `bundled` feature, so the **first**
  `cargo build --workspace` (or `--release --workspace`) compiles the DuckDB C++
  amalgamation — expect ~5–7 minutes and high CPU. Subsequent builds are cached.
- Committed `.cargo/config.toml` is required for that build: it forces `CXX=g++`
  (the image's default `c++` is clang without configured libstdc++ headers) and
  adds g++'s lib dir to the linker search path so `lld` can resolve `-lstdc++`.
  The base image already ships `build-essential` + `libstdc++-13-dev`, so no
  system packages need installing. If g++'s major version changes, update the
  `-L /usr/lib/gcc/x86_64-linux-gnu/<ver>` path in `.cargo/config.toml`.
- Multi-engine flow: `cargo run --bin gen_dataset -- --out-dir data` writes
  `employees.csv` + `oplog.jsonl` + `meta.json`; then
  `cargo run -p duckdb_runner -- --data-dir data` replays the same op log.
  Generated benchmark artifacts (`*_snapshots.json`, `*_report.json`, `data*/`)
  are gitignored. See `LIMITATIONS.md` for scope/caveats.
