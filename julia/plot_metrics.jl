using JSON
using Plots
gr()

isempty(ARGS) && error("Usage: julia --project=julia julia/plot_metrics.jl <snapshots.json>")
snapshots = JSON.parsefile(ARGS[1])
isempty(snapshots) && error("No snapshots in $(ARGS[1])")

ops    = [s["operation"]            for s in snapshots]
util   = [s["factor_utilization"] * 100.0 for s in snapshots]
uaf    = [s["uaf"]                  for s in snapshots]
s_cnt  = [s["structural_factors"]   for s in snapshots]
o_cnt  = [s["operational_factors"]  for s in snapshots]
phases = [s["phase"]                for s in snapshots]

# First snapshot that belongs to Phase B marks the transition.
b_idx        = findfirst(==("B"), phases)
transition_op = b_idx !== nothing ? ops[b_idx] : nothing

function mark_transition!(p)
    transition_op !== nothing &&
        vline!(p, [transition_op], label="Phase B start", color=:crimson, ls=:dash, lw=1.5)
end

p1 = plot(ops, util,
    label="Utilization", lw=2, color=:steelblue,
    ylabel="Utilization (%)", ylims=(0, 105), legend=:bottomright,
    title="Adversarial Workload Shift — Factor Metrics")
mark_transition!(p1)

p2 = plot(ops, uaf,
    label="UAF", lw=2, color=:darkorange,
    ylabel="Update Amplification Factor", legend=:topright)
mark_transition!(p2)

p3 = plot(ops, s_cnt,
    label="Structural",  lw=2, color=:seagreen,
    ylabel="Factor Count", xlabel="Operation", legend=:topleft)
plot!(p3, ops, o_cnt, label="Operational", lw=2, color=:mediumpurple)
mark_transition!(p3)

fig = plot(p1, p2, p3, layout=(3, 1), size=(900, 950))
savefig(fig, "metrics_plot.png")
println("Saved → metrics_plot.png")
