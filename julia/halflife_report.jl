using JSON

isempty(ARGS) && error("Usage: julia --project=julia julia/halflife_report.jl <snapshots.json>")
snapshots = JSON.parsefile(ARGS[1])
isempty(snapshots) && error("No snapshots in $(ARGS[1])")

# The final snapshot has the most complete evicted-factor list.
evicted_all = snapshots[end]["evicted_factors"]

# Only operational factors that were actually evicted (not just listed as active elsewhere).
operational = [f for f in evicted_all
               if !f["is_structural"] && f["evicted_at_tick"] !== nothing]
lifetimes   = [f["evicted_at_tick"] - f["created_at_tick"] for f in operational]

div_line = "╠══════════════════════════════════════╣"

println("\n╔══════════════════════════════════════╗")
println("║        Factor Half-Life Report       ║")
println(div_line)
@printf "║  Input file       : %-17s║\n" basename(ARGS[1])
@printf "║  Total evicted    : %-17d║\n" length(evicted_all)
@printf "║  Operational      : %-17d║\n" length(operational)

if !isempty(lifetimes)
    sort!(lifetimes)
    n      = length(lifetimes)
    mean_l = sum(lifetimes) / n
    med_l  = lifetimes[n ÷ 2 + 1]
    p25    = lifetimes[max(1, n ÷ 4)]
    p75    = lifetimes[min(n, 3 * n ÷ 4)]

    println(div_line)
    @printf "║  Mean lifetime    : %-12.1f ticks║\n" mean_l
    @printf "║  Median lifetime  : %-12.1f ticks║\n" Float64(med_l)
    @printf "║  P25              : %-12d ticks║\n" p25
    @printf "║  P75              : %-12d ticks║\n" p75
    @printf "║  Min              : %-12d ticks║\n" lifetimes[1]
    @printf "║  Max              : %-12d ticks║\n" lifetimes[end]
end

println("╚══════════════════════════════════════╝\n")
