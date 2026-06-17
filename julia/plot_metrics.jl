using JSON, Plots, Statistics

# Read snapshot file path from command line
if length(ARGS) < 1
    error("Usage: julia plot_metrics.jl <snapshots.json>")
end
filename = ARGS[1]

# Parse the JSON file
data = JSON.parsefile(filename)

# Extract arrays of metrics, grouped by phase
phases        = [d["phase"]               for d in data]
ops           = [d["operation"]           for d in data]
factor_util   = [d["factor_utilization"]  for d in data]
uaf_vals      = [d["uaf"]                for d in data]
struct_factors = [d["structural_factors"] for d in data]
oper_factors  = [d["operational_factors"] for d in data]

# Find transition point (first Phase B snapshot)
transition_op = nothing
for i in 2:length(phases)
    if phases[i] == "B" && phases[i-1] == "A"
        transition_op = ops[i]
        break
    end
end

# Plot 1: Factor Utilization over time
p1 = plot(ops, factor_util .* 100, label="Factor Utilization (%)",
          xlabel="Operation", ylabel="%", title="Factor Utilization",
          legend=:bottomright)
if transition_op !== nothing
    vline!([transition_op], label="Phase Shift", linestyle=:dash, color=:red)
end

# Plot 2: UAF over time
p2 = plot(ops, uaf_vals, label="UAF", xlabel="Operation",
          ylabel="UAF", title="Update Amplification Factor")
if transition_op !== nothing
    vline!([transition_op], label="Phase Shift", linestyle=:dash, color=:red)
end

# Plot 3: Factor counts
p3 = plot(ops, struct_factors, label="Structural", xlabel="Operation",
          ylabel="Count", title="Factor Counts", color=:blue)
plot!(ops, oper_factors, label="Operational", color=:orange)
if transition_op !== nothing
    vline!([transition_op], label="Phase Shift", linestyle=:dash, color=:red)
end

# Combine into a single figure
fig = plot(p1, p2, p3, layout=(3, 1), size=(800, 900))

# Save — derive output name from input
output_png = replace(filename, r"\.json$" => "_metrics.png")
savefig(fig, output_png)
println("Plot saved to $output_png")

# Summary statistics
println("\n=== Metrics Summary ===")
println("Number of snapshots: ", length(data))
if transition_op !== nothing
    println("Phase shift at operation: ", transition_op)
end
println("Mean Factor Utilization: ",   round(mean(factor_util) * 100, digits=2), "%")
println("Median Factor Utilization: ", round(median(factor_util) * 100, digits=2), "%")
println("Max UAF: ",  round(maximum(uaf_vals), digits=2))
println("Mean UAF: ", round(mean(uaf_vals), digits=2))
