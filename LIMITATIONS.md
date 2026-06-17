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
