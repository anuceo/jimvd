using JSON, Statistics, Plots

snapshot_path = length(ARGS) >= 1 ? ARGS[1] : "extended_adversarial_snapshots.json"
output_path   = length(ARGS) >= 2 ? ARGS[2] : "data/factor_lifetimes.png"

snapshots = JSON.parsefile(snapshot_path)

all_evicted = []
for snap in snapshots
    for ef in get(snap, "evicted_factors", [])
        ef["is_structural"] && continue
        created  = ef["created_at_tick"]
        evicted  = ef["evicted_at_tick"]
        evicted === nothing && continue
        push!(all_evicted, Int(evicted) - Int(created))
    end
end

if isempty(all_evicted)
    println("No evicted operational factors found in $(snapshot_path).")
    exit(0)
end

n          = length(all_evicted)
med        = median(all_evicted)
mn         = mean(all_evicted)
p25        = quantile(all_evicted, 0.25)
p75        = quantile(all_evicted, 0.75)
ephemeral  = count(x -> x <= 10, all_evicted)

println("Factor lifetime statistics (operational factors only)")
println("  count      : $n")
println("  half-life  : $(round(med, digits=1)) ticks")
println("  mean       : $(round(mn, digits=1)) ticks")
println("  P25        : $(round(p25, digits=1)) ticks")
println("  P75        : $(round(p75, digits=1)) ticks")
println("  ephemeral (≤10 ticks): $ephemeral ($(round(100*ephemeral/n, digits=1))%)")

mkpath(dirname(output_path))

histogram(all_evicted,
    bins        = 40,
    xlabel      = "Lifetime (ticks)",
    ylabel      = "Count",
    title       = "Operational Factor Lifetime Distribution",
    legend      = false,
    color       = :steelblue,
    linecolor   = :white)
vline!([med], color = :red, linestyle = :dash, linewidth = 2,
       label = "median = $(round(med, digits=1))")

savefig(output_path)
println("Histogram saved to $(output_path)")
