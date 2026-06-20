# Known Limitations

## 1. Greedy biclique cover scalability

The initial structural factorization uses a greedy biclique cover (`src/cover.rs`).
Each iteration finds the atom (attribute-value pair) with the largest uncovered
object set, extends the intent to all atoms shared by every object in that set,
and removes the covered pairs.  Time complexity is O(|atoms| × |objects|) per
iteration, with up to |atoms| × |objects| iterations in the worst case.

For datasets up to ~10 000 objects with a few dozen distinct attribute values this
is fast enough to run at startup.  Beyond that threshold — particularly for
attributes with high cardinality or for datasets in the millions — the initial
factorization becomes the dominant cost.  A production-grade engine would need an
approximate, sampling-based, or incremental cover instead.

## 2. UPDATE deltas require full-row snapshots

The `handle_update` path in `src/graph.rs` computes the diff between the old and
new property sets to determine which factors must be updated.  This diff is correct
only when the incoming delta carries **all** current attribute values, not just the
changed ones.

The benchmark runner (`BenchmarkRunner::do_write` in `src/benchmark.rs`) satisfies
this requirement: it always loads the existing object, merges the full property set
into the delta, and then sends the result.  A client that sends partial deltas
(only the changed attribute) would cause `handle_update` to treat every omitted
attribute as "removed", corrupting BOI membership and producing incorrect factor
utilization metrics.  True partial-update support would require storing a
before-image log or a separate property overlay layer.

## 3. Cold-start row-scan floor for completely novel attributes

The adaptive materialiser (`adapt_conjunction` in `src/graph.rs`) promotes a
repeated AND-query pattern into an operational factor only when the query is served
**entirely from factor space** (`!used_rows`).  If every sub-filter of the AND
falls through to a row scan — because none of the queried attributes appear in any
existing factor's intent — the conjunction counter is never incremented and no
operational factor is ever created.

Concretely: during Phase B of the extended adversarial test, Clearance / Project /
Office have no structural factors (they were absent from the A-phase seed data).
Every B-phase query row-scans the object store, `adapt_conjunction` is never
called, and factor utilization stays near 0% for the entire phase.

This is architecturally intentional: the engine can only promote patterns it has
already started to serve in factor space.  A separate "bootstrap" path — such as
creating a minimal structural factor from the first N observed objects that match a
novel attribute value — would be needed to eliminate the cold-start floor.

## 4. High-cardinality attributes

When an attribute has many distinct values (e.g., free-text fields, UUIDs, or
continuous numeric identifiers), the greedy biclique cover produces one structural
factor per distinct value.  This degrades to a scan of the BPI index and gives no
compression benefit over a conventional B-tree.  The prototype is intended for
low-to-moderate cardinality categorical attributes (role, region, clearance, etc.).

## 5. Multi-value attributes

The current model stores exactly one string per attribute per object.  Attributes
that are naturally multi-valued (a user's set of tags, a list of granted
permissions) must be pre-expanded into multiple objects or encoded as a single
string before ingestion.  Native set-valued attributes would require an extension to
the BPI and BOI index structures.

## 6. Temporal and range queries

Factor-space execution relies on equality atoms of the form `attr=value`.  Temporal
predicates (`created_at > '2024-01-01'`) or range predicates (`salary BETWEEN
50000 AND 90000`) cannot be directly expressed as atoms.  The prototype falls back
to a full row scan for any continuous attribute.  A production engine would need a
range-partitioned index or a discretisation scheme (bucketing) to serve such queries
from factor space.

## 7. Concurrency

All factor graph operations are single-threaded.  The `Metrics` struct uses
`AtomicU64` for lock-free counters, but `FactorGraph` itself is not `Send + Sync`
and must be accessed from a single thread or behind a mutex.  Scaling to concurrent
readers and writers would require either a reader-writer lock, an epoch-based
reclamation scheme, or a partition-per-shard architecture.

## 8. Memory and storage scaling

The storage estimate (`storage_bytes` in MetricsReport) counts factor extents at
4 bytes/object-ID, intent string sizes, BOI entry overhead, and BPI entry overhead.
This does not include Rust allocator overhead or hash-map load-factor waste.  At
100 000 objects with a moderate factor count, empirical RSS is roughly 3–5× the
raw data size.  Memory-mapped or compressed extents would be needed for datasets
in the tens of millions.

## 9. Real-world data

All benchmark data is synthetically generated with configurable correlation
coefficients.  Real IAM datasets may exhibit skewed distributions (95 % of users
have the "Viewer" role), temporal access patterns, or multi-tenancy isolation
requirements that are not captured by the uniform/biased random model.  The
`weights` and `hot_values` configuration keys provide limited skew control but do
not substitute for validation against production data.

## 10. Bottleneck scale

Empirical profiling shows that `GreedyCover::build_factors()` remains under 100 ms
for datasets up to 10 000 objects with 3–5 categorical attributes.  At 100 000
objects (the current scaling-wall ceiling) it takes 5–30 seconds depending on
attribute cardinality.  This is acceptable for a one-time startup cost but
unacceptable for incremental re-factorization.  The cover timing is recorded in
`cover_time_ms` within each MetricsReport snapshot.

## Positive scope

This prototype demonstrates that factor-space execution is feasible for categorical,
correlated datasets under moderate scale, and that adaptive materialisation responds
correctly to workload shifts.  The factor utilization metric reliably rises from
near 0 % during a cold start to 85–95 % once the workload stabilises, and the
Update Amplification Factor confirms that write propagation overhead remains bounded
even as the factor graph grows.
