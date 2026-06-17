# Phase 8 – Visualisation & Basic Statistical Report

After any benchmark run—regular or adversarial—you have a JSON snapshot file.
Phase 8 turns that data into:

- A multi-panel PNG plot showing Factor Utilisation, UAF, and factor counts over time.
- A lifetime-distribution histogram for evicted operational factors.
- Printed statistical summaries in the terminal.
- If the file contains an adversarial phase shift, a vertical dashed line marking the exact operation where the workload switched.

---

## Understanding the Input JSON

The snapshot file (e.g. `data/adversarial_snapshots.json`) is an array of objects:

```json
[
  {
    "operation": 500,
    "phase": "A",
    "factor_utilization": 0.87,
    "uaf": 2.1,
    "structural_factors": 12,
    "operational_factors": 3,
    "active_factors": [...],
    "evicted_factors": [...]
  },
  ...
]
```

Key fields:

| Field | Type | Used for |
|---|---|---|
| `operation` | integer | x-axis |
| `phase` | `"A"` or `"B"` | detecting the workload shift |
| `factor_utilization` | float 0–1 | main efficiency metric |
| `uaf` | float | update amplification |
| `structural_factors` | integer | stable factor count |
| `operational_factors` | integer | adaptive factor count |
| `evicted_factors` | array | lifetime analysis |

---

## One-Time Environment Setup

```bash
bash scripts/setup_julia.sh
```

This installs Julia (via juliaup if needed), adds JSON, Plots, and Statistics to
the `analysis/` project environment, and precompiles them. It creates
`analysis/Manifest.toml`, which you should commit so collaborators get exact
package versions:

```bash
git add analysis/Manifest.toml
git commit -m "Lock Julia dependency versions"
```

---

## Running the Scripts

Both scripts are run from the **repo root** using the `--project=analysis` flag,
which activates the isolated environment in the `analysis/` directory.

### Plot metrics

```bash
julia --project=analysis analysis/plot_metrics.jl data/adversarial_snapshots.json
```

Produces `data/adversarial_snapshots_metrics.png` — a three-panel figure:

1. **Factor Utilisation (%)** — how much work stayed in factor space vs falling
   back to row scans. Should be near 100% throughout.
2. **Update Amplification Factor** — graph nodes touched per object write.
   Watch this during the Phase B transition; it drops as the old factors are
   evicted and new, lighter ones are built.
3. **Factor Counts** — structural (solid baseline) and operational (adaptive)
   factors. The operational line collapses and rebuilds at the shift point.

If the file contains both phases, a red dashed vertical line marks where the
workload switched.

Terminal output:

```
Plot saved to data/adversarial_snapshots_metrics.png

=== Metrics Summary ===
Number of snapshots: 18
Phase shift at operation: 5500
Mean Factor Utilization: 100.0%
Median Factor Utilization: 100.0%
Max UAF: 4.26
Mean UAF: 3.82
```

### Half-life report

```bash
julia --project=analysis analysis/halflife_report.jl data/adversarial_snapshots.json
```

Produces `data/adversarial_snapshots_lifetimes.png` — a histogram of how long each
evicted operational factor survived before being swept out. Also prints:

```
=== Factor Half-Life Report ===
Number of evicted factors: 60
Factors with complete lifecycle: 60
Half-life (median lifetime): 5420 ticks
Mean lifetime: 5310.2 ticks
25th percentile: 4800 ticks
75th percentile: 5900 ticks
Ephemeral ratio (lived ≤ 10 ticks): 0.0%
```

A high median lifetime means the workload was stable — factors that matched
Phase A queries survived until the shift evicted them. A low ephemeral ratio
confirms no factors were created and immediately discarded.

---

## How the Phase Shift Is Detected

```julia
transition_op = nothing
for i in 2:length(phases)
    if phases[i] == "B" && phases[i-1] == "A"
        transition_op = ops[i]
        break
    end
end
```

The script scans for the first snapshot where the phase label changes from `"A"`
to `"B"`. If no shift is found (single-phase run), `transition_op` stays
`nothing` and no vertical line is drawn. Both scripts handle single-phase data
correctly.

---

## Automation via Rust

Both scripts are called automatically at the end of `cargo run --bin adversarial_test`
through the `run_julia_script()` bridge in `src/benchmark.rs`. If Julia is not
installed, the binary skips the analysis and prints a hint:

```
[Julia] not installed — skipping plot_metrics.jl. Run scripts/setup.sh to install.
```

To add Julia analysis to the regular benchmark binary, add these two lines at the
end of `src/main.rs`:

```rust
use jimvd::benchmark::run_julia_script;
run_julia_script("plot_metrics.jl", "data/snapshots.json");
```

---

## What the Numbers Actually Mean

**Factor Utilisation at 100%** is the headline result. Every query resolved
through set intersection on factors — zero row reconstruction. This holds even
immediately after the Phase B shift, because queries against attributes that
have no factors yet simply return empty sets (still factor-space work, just
with empty extents).

**UAF dropping during Phase B** is the more interesting signal. Phase A
operational factors cover dense extents (many objects with the same Role +
Region). When Phase B writes add Clearance/Project/Office attributes, the
new factors start small and sparse, so each write touches fewer nodes per
object. UAF falls because the graph is temporarily *under-factorised*
relative to the new workload — exactly the adaptation window we want to
measure.

**Ephemeral ratio near zero** confirms the adaptation threshold of three
repeated conjunctions is well-calibrated. Factors that are created survive
long enough to be useful before the workload shifts away.
