#!/usr/bin/env bash
# Install Julia (if missing) and set up the project's Julia environment.
# Run this once from the repo root before using any analysis/ scripts.
set -euo pipefail

# ── 1. Install Julia if not present ──────────────────────────────────────────
if ! command -v julia &>/dev/null; then
    echo "Julia not found — installing via juliaup..."
    curl -fsSL https://install.julialang.org | sh -s -- --yes
    export PATH="$HOME/.juliaup/bin:$PATH"
fi

echo "Found: $(julia --version)"

# ── 2. Resolve / install dependencies ────────────────────────────────────────
ANALYSIS_DIR="$(cd "$(dirname "$0")/../analysis" && pwd)"
echo "Setting up Julia environment at $ANALYSIS_DIR ..."

julia --project="$ANALYSIS_DIR" -e '
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
echo "Julia environment ready. Commit analysis/Manifest.toml to lock exact versions."
echo ""
echo "Usage (from repo root):"
echo "  julia --project=analysis analysis/plot_metrics.jl    data/adversarial_snapshots.json"
echo "  julia --project=analysis analysis/halflife_report.jl data/adversarial_snapshots.json"
