# Phase 9 – Factor Half-Life Analysis

## Objective

After a benchmark run the snapshot file contains detailed lifecycle information
about every operational factor that was evicted. Phase 9 extracts that data and
answers four questions:

| Question | Metric |
|---|---|
| How long did a typical factor survive? | **Half-life** — median lifetime in ticks |
| Were factors discarded almost immediately? | **Ephemeral ratio** — % living ≤ 10 ticks |
| What is the spread of lifetimes? | **25th / 75th percentiles** |
| What does the distribution look like? | **Lifetime histogram PNG** |

---

## Understanding the Input Data

Each snapshot in the JSON array carries an `evicted_factors` array. The final
snapshot contains the complete, deduplicated list of every operational factor
that was created and later evicted during the entire run:

```json
{
  "operation": 10000,
  "evicted_factors": [
    {
      "id": 42,
      "intent": ["Role=Engineer", "Region=West"],
      "extent": [1, 7, 19, 45],
      "created_at_tick": 120,
      "evicted_at_tick": 5538
    },
    ...
  ]
}
```

Key fields used by the script:

| Field | Type | Meaning |
|---|---|---|
| `created_at_tick` | integer or `null` | Tick when the factor was materialised |
| `evicted_at_tick` | integer or `null` | Tick when the factor was swept out |

Lifetime = `evicted_at_tick − created_at_tick`. Factors with either timestamp
`null` are excluded from the statistical analysis; they still count toward the
total evicted-factor list.

---

## Script Walkthrough: `julia/halflife_report.jl`

### Loading data

```julia
using JSON, Plots, Statistics

filename = ARGS[1]
data = JSON.parsefile(filename)

last_snapshot = data[end]
evicted_factors = last_snapshot["evicted_factors"]
```

`JSON.parsefile` returns a plain Julia `Dict` / `Array` tree. The script pulls
the final snapshot because it is cumulative — earlier snapshots only hold the
factors evicted up to that point, so `data[end]` is always the superset.

### Computing lifetimes

```julia
lifetimes = []
for f in evicted_factors
    created = f["created_at_tick"]
    evicted = f["evicted_at_tick"]
    if evicted !== nothing && created !== nothing
        push!(lifetimes, evicted - created)
    end
end
```

The `!== nothing` guard handles factors whose lifecycle timestamps are absent
(e.g. structural factors that were never evicted and accidentally appear in the
array). Only factors with both timestamps contribute to statistics.

### Statistics

```julia
half_life  = median(lifetimes)
mean_life  = mean(lifetimes)
p25        = quantile(lifetimes, 0.25)
p75        = quantile(lifetimes, 0.75)
```

`median`, `mean`, and `quantile` all come from Julia's built-in `Statistics`
standard library — no external package required beyond what is already in
`julia/Project.toml`.

### Ephemeral ratio

```julia
ephemeral_threshold = 10
ephemeral_count = count(lf -> lf <= ephemeral_threshold, lifetimes)
ephemeral_ratio = ephemeral_count / length(lifetimes) * 100
```

A factor that lives ten ticks or fewer was created and discarded before it
could serve even a handful of queries. A non-zero ephemeral ratio suggests
the adaptation threshold (`min_pattern_hits`, currently 3) may be too
aggressive for this workload.

### Terminal output

```julia
println("\n=== Factor Half-Life Report ===")
println("Number of evicted factors: ",          length(evicted_factors))
println("Factors with complete lifecycle: ",     length(lifetimes))
println("Half-life (median lifetime): ",         half_life, " ticks")
println("Mean lifetime: ",                       round(mean_life, digits=1), " ticks")
println("25th percentile: ",                     p25, " ticks")
println("75th percentile: ",                     p75, " ticks")
println("Ephemeral ratio (lived ≤ $ephemeral_threshold ticks): ",
        round(ephemeral_ratio, digits=1), "%")
```

### Histogram

```julia
hist = histogram(lifetimes, bins=20, legend=false,
                 xlabel="Lifetime (ticks)", ylabel="Frequency",
                 title="Factor Lifetime Distribution")
vline!([half_life], color=:red, linestyle=:dash, label="Half-life")

output_png = replace(filename, r"\.json$" => "_lifetimes.png")
savefig(hist, output_png)
println("Lifetime histogram saved to $output_png")
```

The output filename is derived automatically from the input: passing
`adversarial_snapshots.json` produces `adversarial_snapshots_lifetimes.png`.
The red dashed vertical line marks the half-life so the median is immediately
visible against the raw distribution.

---

## Running the Script

```bash
julia --project=analysis analysis/halflife_report.jl data/adversarial_snapshots.json
```

Must be run from the **repo root** so `--project=julia` resolves correctly.
If Julia is not yet installed, run `bash scripts/setup_julia.sh` first.

Expected terminal output:

```
=== Factor Half-Life Report ===
Number of evicted factors: 60
Factors with complete lifecycle: 60
Half-life (median lifetime): 5420 ticks
Mean lifetime: 5310.2 ticks
25th percentile: 4800 ticks
75th percentile: 5900 ticks
Ephemeral ratio (lived ≤ 10 ticks): 0.0%
Lifetime histogram saved to adversarial_snapshots_lifetimes.png
```

---

## Interpreting the Results

### High half-life (Phase A factors)

A median lifetime in the thousands of ticks means operational factors were
created early in Phase A and survived almost until the workload shifted. This
is the expected result: Phase A queries are stable and repetitive, so the
factors that match them are used continuously and never fall below the
inactivity threshold.

### Low half-life after the Phase B shift

If you run the report on a Phase B-only snapshot, median lifetime will be
shorter — new operational factors are still being built as the workload
explores new conjunction patterns. This is the adaptation window described in
the Phase 7 spec.

### Ephemeral ratio near zero

Confirms that the `min_pattern_hits = 3` threshold is well-calibrated. The
system does not materialise speculative factors; it waits until a conjunction
has been requested three times before committing to a factor.

### High ephemeral ratio (warning sign)

If this number climbs above 5–10%, the threshold is too low relative to the
workload's query diversity. Factors are being created and discarded before
serving enough queries to justify their build cost. Consider raising
`min_pattern_hits` in the workload config.

---

## Automation via Rust

`halflife_report.jl` is called automatically at the end of
`cargo run --bin adversarial_test` via the `run_julia_script` bridge in
`src/benchmark.rs`. If Julia is not installed the binary skips the step and
prints:

```
[Julia] not installed — skipping halflife_report.jl. Run scripts/setup_julia.sh to install.
```

To add the half-life report to the regular benchmark binary, append to
`src/main.rs`:

```rust
use jimvd::benchmark::run_julia_script;
run_julia_script("halflife_report.jl", "data/snapshots.json");
```
