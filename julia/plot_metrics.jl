using JSON, Plots, Statistics

if length(ARGS) < 1
    error("Usage: julia plot_metrics.jl <snapshots.json>")
end
filename = ARGS[1]

data = JSON.parsefile(filename)

phases         = [d["phase"]               for d in data]
ops            = [d["operation"]           for d in data]
factor_util    = [d["factor_utilization"]  for d in data]
uaf_vals       = [d["uaf"]                for d in data]
struct_factors = [d["structural_factors"]  for d in data]
oper_factors   = [d["operational_factors"] for d in data]

# Find every phase-transition point (op where phase label changes).
transition_ops   = Int[]
transition_labels = String[]
for i in 2:length(phases)
    if phases[i] != phases[i-1]
        push!(transition_ops, ops[i])
        push!(transition_labels, phases[i])
    end
end

function add_transitions!(p)
    colors = [:red, :green, :purple, :orange]
    for (j, (op, lbl)) in enumerate(zip(transition_ops, transition_labels))
        c = colors[mod1(j, length(colors))]
        vline!(p, [op], label="→ $lbl", linestyle=:dash, color=c)
    end
end

p1 = plot(ops, factor_util .* 100,
          label="Factor Utilization (%)",
          xlabel="Operation", ylabel="%",
          title="Factor Utilization across Phases",
          legend=:bottomright, linewidth=2)
add_transitions!(p1)
hline!(p1, [90.0], label="90% threshold", linestyle=:dot, color=:black)

p2 = plot(ops, uaf_vals,
          label="UAF",
          xlabel="Operation", ylabel="UAF",
          title="Update Amplification Factor",
          legend=:topright, linewidth=2)
add_transitions!(p2)

p3 = plot(ops, struct_factors,
          label="Structural", color=:blue,
          xlabel="Operation", ylabel="Count",
          title="Factor Counts", linewidth=2)
plot!(p3, ops, oper_factors, label="Operational", color=:orange, linewidth=2)
add_transitions!(p3)

fig = plot(p1, p2, p3, layout=(3, 1), size=(900, 1050))

mkpath("data")
basename_noext = replace(basename(filename), r"\.json$" => "")
output_png = joinpath("data", basename_noext * "_metrics.png")
savefig(fig, output_png)
println("Plot saved to $output_png")

println("\n=== Metrics Summary ===")
println("Snapshots: ", length(data))
println("Phases:    ", join(unique(phases), " → "))
if !isempty(transition_ops)
    println("Transitions at ops: ", join(transition_ops, ", "))
end
println("Mean Factor Utilization:   ", round(mean(factor_util) * 100, digits=2), "%")
println("Median Factor Utilization: ", round(median(factor_util) * 100, digits=2), "%")
println("Max UAF:  ", round(maximum(uaf_vals), digits=2))
println("Mean UAF: ", round(mean(uaf_vals), digits=2))

# Per-phase breakdown
println("\n=== Per-Phase Breakdown ===")
for phase_name in unique(phases)
    idx = findall(p -> p == phase_name, phases)
    utils = factor_util[idx]
    println("  $phase_name: util_mean=$(round(mean(utils)*100, digits=1))%  util_final=$(round(utils[end]*100, digits=1))%  snapshots=$(length(idx))")
end
