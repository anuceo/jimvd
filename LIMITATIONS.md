# JimVD — Limitations & Scope

This document records the known limitations of the JimVD prototype and the scope
under which its results are valid. It is deliberately blunt: JimVD is a research
prototype, not a production database.

## Positive scope

> This prototype demonstrates that factor‑space execution is feasible for
> categorical, correlated datasets under moderate scale, and that adaptive
> materialisation responds correctly to workload shifts.

Everything below qualifies that statement.

## Data assumptions

- **Real‑world data.** All published results use **synthetic categorical data**.
  Real‑world value distributions, correlations, and dirty data are **untested**.
- **Uniform by default.** Unless a config sets per‑attribute `weights`,
  `hot_values`, `null_probability`, or `continuous` ranges, generated data is
  **uniform categorical with no NULLs**. The skew / NULL / continuous / hot‑spot
  options exist so the bias of uniform synthetic data can be probed, but the
  headline metrics (Factor Utilization ≈ 100%, etc.) are produced with **uniform
  categorical data** and should be read with that caveat.

## Attribute / query limitations

- **High‑cardinality attributes.** Attributes with many distinct values degrade
  the greedy cover into mostly **singleton factors** (one object, one atom).
  This case is **untested** at scale and defeats the compression premise.
- **Multi‑value attributes.** An object may hold only one value per attribute.
  Set‑valued / repeated attributes are **not supported**.
- **Temporal / range queries.** A `continuous` integer attribute type exists and
  equality queries against it correctly **fall back to row scans** (they are not
  factorised), but true range predicates (`<`, `BETWEEN`) and temporal queries
  are **not implemented and not benchmarked**. The continuous type was added
  recently and has only smoke‑level coverage.

## Concurrency

- The engine is a **single‑threaded prototype**. There is no transaction
  isolation, locking, or concurrent access; metrics are gathered from a single
  serial op stream.

## Scale, memory & storage

- **Covering step is the scaling wall.** `GreedyCover::build_factors()` is
  `O(atoms × uncovered_pairs)` per rectangle. For the bundled **low‑cardinality**
  IAM workload (≈11 distinct atoms) the covering cost is roughly linear and
  remains modest up to 100,000 objects:

  | Objects | Cover build | Factors | Process RSS | Storage est. |
  |--------:|------------:|--------:|------------:|-------------:|
  | 1,000   | ~0.009 s    | 11      | ~8 MB       | ~0.06 MB     |
  | 10,000  | ~0.10 s     | 11      | ~21 MB      | ~0.56 MB     |
  | 50,000  | ~0.54 s     | 11      | ~69 MB      | ~2.8 MB      |
  | 100,000 | ~1.14 s     | 11      | ~144 MB     | ~5.5 MB      |

  *(Measured via `cargo run --bin scaling_wall -- --max-scale 100000`,
  seed 0. Numbers vary run‑to‑run.)*

  The covering step becomes the dominant cost — the "scaling wall" — once
  **cardinality** rises: many distinct atoms make each `largest_rectangle`
  iteration scan a large uncovered set, pushing the cost toward quadratic. With
  high‑cardinality or continuous attributes this dominates well before 100K
  objects.
- **Memory / storage scaling is untested beyond 100K objects.** RSS grows
  roughly linearly with object count in the measured range; behaviour past 100K
  (and with realistic cardinality) is unknown.
- The prototype is designed for **small‑to‑medium categorical datasets**.

## Multi‑engine comparison

- A minimal **DuckDB** runner (`benchmark-suite/duckdb_runner`) replays the same
  CSV + operation log produced by `jimvd gen_dataset` and reports P50/P99 latency,
  total time, and row counts. It executes equivalent SQL (`SELECT` for `Eq`,
  conjunctive `JOIN` for `And`, `IN` for `Or`, plus `INSERT`/`UPDATE`/`DELETE`).
- **Postgres comparison is still a placeholder.** `src/catalog.rs` contains the
  Postgres metadata‑catalog scaffolding, but there is **no functional Postgres
  benchmark runner**; only DuckDB is wired up for cross‑engine numbers.
- The two engines measure **different axes** (JimVD reports factor utilization /
  update amplification; DuckDB reports latency). Treat the comparison as
  directional, not a like‑for‑like throughput benchmark.

## Reproducibility

- All runs are driven by a single `rng_seed` (workload config field, default 0).
  The seed is printed at startup and embedded in every snapshot's metadata so
  runs are reproducible. The covering‑step time and factor count are likewise
  recorded in snapshot metadata.
