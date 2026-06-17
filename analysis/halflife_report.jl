using JSON, Plots, Statistics

# Read snapshot file path from command line
if length(ARGS) < 1
    error("Usage: julia halflife_report.jl <snapshots.json>")
end
filename = ARGS[1]

# Parse the JSON file
data = JSON.parsefile(filename)

# Collect all evicted factors from all snapshots (they should be unique across phases)
# The final snapshot contains the complete list of evicted factors.
# We'll gather from the very last snapshot.
last_snapshot = data[end]
evicted_factors = last_snapshot["evicted_factors"]

if isempty(evicted_factors)
    println("No evicted factors found in the final snapshot.")
    exit()
end

# Extract lifetimes (ticks from creation to eviction)
lifetimes = []
for f in evicted_factors
    created = f["created_at_tick"]
    evicted = f["evicted_at_tick"]
    if evicted !== nothing && created !== nothing
        push!(lifetimes, evicted - created)
    end
end

if isempty(lifetimes)
    println("No lifetime data available (all evicted factors have null timestamps).")
    exit()
end

# Compute statistics
half_life  = median(lifetimes)
mean_life  = mean(lifetimes)
p25        = quantile(lifetimes, 0.25)
p75        = quantile(lifetimes, 0.75)

# Ephemeral ratio: factors that lived less than 10 ticks
ephemeral_threshold = 10
ephemeral_count = count(lf -> lf <= ephemeral_threshold, lifetimes)
ephemeral_ratio = ephemeral_count / length(lifetimes) * 100

# Print report
println("\n=== Factor Half-Life Report ===")
println("Number of evicted factors: ",          length(evicted_factors))
println("Factors with complete lifecycle: ",     length(lifetimes))
println("Half-life (median lifetime): ",         half_life, " ticks")
println("Mean lifetime: ",                       round(mean_life, digits=1), " ticks")
println("25th percentile: ",                     p25, " ticks")
println("75th percentile: ",                     p75, " ticks")
println("Ephemeral ratio (lived ≤ $ephemeral_threshold ticks): ",
        round(ephemeral_ratio, digits=1), "%")

# Plot histogram
hist = histogram(lifetimes, bins=20, legend=false,
                 xlabel="Lifetime (ticks)", ylabel="Frequency",
                 title="Factor Lifetime Distribution")
# Add median line
vline!([half_life], color=:red, linestyle=:dash, label="Half-life")

mkpath("data")
basename_noext = replace(basename(filename), r"\.json$" => "")
output_png = joinpath("data", basename_noext * "_lifetimes.png")
savefig(hist, output_png)
println("Lifetime histogram saved to $output_png")
