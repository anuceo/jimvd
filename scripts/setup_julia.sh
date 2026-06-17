#!/usr/bin/env bash
# Install Julia (if missing) and set up the project's Julia environment.
# Run this once from the repo root before using any julia/ scripts.
set -euo pipefail

# ── 1. Install Julia if not present ──────────────────────────────────────────
if ! command -v julia &>/dev/null; then
    echo "Julia not found — installing via juliaup..."
    curl -fsSL https://install.julialang.org | sh -s -- --yes
    export PATH="$HOME/.juliaup/bin:$PATH"
fi

echo "Found: $(julia --version)"

# ── 2. Resolve / install dependencies ────────────────────────────────────────
JULIA_DIR="$(cd "$(dirname "$0")/../julia" && pwd)"
echo "Setting up Julia environment at $JULIA_DIR ..."

julia --project="$JULIA_DIR" -e '
    using Pkg
    # Pkg.instantiate creates Manifest.toml on first run, or restores it from
    # an existing one. Using Pkg.add first ensures any packages not yet in the
    # Manifest are fetched even if Project.toml was edited by hand.
    Pkg.add(["JSON", "Plots", "Statistics"])
    Pkg.instantiate()
    println("Precompiling packages (first run may take a minute)...")
    Pkg.precompile()
'

echo ""
echo "Julia environment ready. Commit julia/Manifest.toml to lock exact versions."
echo ""
echo "Usage (from repo root):"
echo "  julia --project=julia julia/plot_metrics.jl    adversarial_snapshots.json"
echo "  julia --project=julia julia/halflife_report.jl adversarial_snapshots.json"
