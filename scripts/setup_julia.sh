#!/usr/bin/env bash
# Install Julia (if missing) and resolve the project's Julia dependencies.
set -euo pipefail

if ! command -v julia &>/dev/null; then
    echo "Julia not found — installing via juliaup..."
    curl -fsSL https://install.julialang.org | sh -s -- --yes
    # juliaup adds itself to ~/.profile; source the path for the current shell.
    export PATH="$HOME/.juliaup/bin:$PATH"
fi

echo "Julia $(julia --version)"

JULIA_PROJECT_DIR="$(cd "$(dirname "$0")/../julia" && pwd)"
echo "Resolving Julia environment at $JULIA_PROJECT_DIR..."
julia --project="$JULIA_PROJECT_DIR" -e 'using Pkg; Pkg.instantiate()'
echo "Julia environment ready."
echo
echo "Run analysis:"
echo "  julia --project=julia julia/plot_metrics.jl      adversarial_snapshots.json"
echo "  julia --project=julia julia/halflife_report.jl   adversarial_snapshots.json"
