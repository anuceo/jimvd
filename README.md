# JimVD

**A database that thinks in rectangles, adapts to your workload, and only builds what you actually need.**

---

## Why does JimVD exist?

Most databases store your data the same way you'd organize a filing cabinet: rows and columns, every value tucked neatly in its place. That works fine—until you notice how much information you're repeating. The same department name appears in thousands of rows. The same doctor–patient relationship gets copied over and over. And when you finally ask a question like "show me all cardiology patients of Dr. Smith at Hospital A," the database still scans far more than it should.

We wanted to see what would happen if we turned the problem inside out. Instead of storing rows and building indexes to speed up queries, we decided to **store the relationships directly as tiny, overlapping rectangles**—each one capturing a set of objects that share a set of properties. Those rectangles become the physical storage units, not rows. Queries become fast set‑operations on those rectangles, not scans. And because the system learns which combinations you query most, it quietly grows new rectangles to make your next request even faster.

JimVD is that experiment made real.

---

## The core idea, in a nutshell

Imagine a massive spreadsheet. Instead of saving every cell, JimVD notices that **Object 1**, **Object 3**, and **Object 5** all have `Role = Admin`. It pulls that out into a single chunk—a rectangle that says "these three objects share `Role = Admin`". Another rectangle might capture "these five objects are in `Region = US`". To reconstruct a full row, you just overlay the rectangles that belong to that object. To answer a query like `Role = Admin AND Region = US`, you simply intersect two sets of objects—no scanning, no index traversal, just set math.

That's the heart of JimVD: **exact factorized storage** derived from formal concept analysis, but treated as a living, breathing database engine rather than a static mathematical curiosity.

---

## What makes JimVD different?

- **Codomains** – Pre‑defined slices of your data (like "Hospital A") that act as access boundaries. When a query comes in with a codomain, we only touch the rectangles relevant to that slice.
- **Contact Relations** – Named set memberships (like "Cardiology → Dr. Smith") that cut across codomains. They replace repeated column values with simple set lists.
- **Factor‑native execution** – Most filtering, intersection, and union happens directly on those rectangles. Rows are only reconstructed at the very end, when you need to display results.
- **Self‑adapting behavior** – JimVD watches which combinations of properties you query most, then materializes new rectangles to speed them up. Unused rectangles quietly fade away. The database shapes itself to your workload.
- **Incremental everything** – A dependency graph tracks how changes ripple through rectangles. Writes are small deltas; the graph updates only what's needed, keeping maintenance lightweight.
- **Virtual disk & JIT reconstruction** – Databases can be frozen to cold storage and brought back to life only when accessed. A snapshot of rectangles plus a chain of deltas is all that's needed.

---

## Is this a replacement for PostgreSQL? No.

JimVD is built *on top* of PostgreSQL, using it as the reliable metadata catalog—the brain that keeps track of codomains, contact relations, rectangles, and deltas. The actual data (the rectangles themselves) lives in a custom virtual disk format. Think of it as a new engine that sits beside your existing stack, not a replacement.

---

## Getting started (prototype)

Right now JimVD is a Rust prototype you can run in any GitHub Codespace. No local storage needed.

1. **Clone the repo**
   ```bash
   git clone https://github.com/your-org/jimvd
   cd jimvd
   ```

2. **Launch the PostgreSQL metadata catalog (Docker)**
   ```bash
   docker run -d --name jimvd-catalog \
     -e POSTGRES_PASSWORD=admin123 \
     -e POSTGRES_DB=db_catalog \
     -p 5432:5432 postgres:16-alpine
   ```

3. **Run a benchmark**
   ```bash
   cargo run --bin jimvd
   ```
   This generates synthetic IAM data, discovers initial factors, and runs a mixed read/write workload, printing metrics like Factor Utilization and Update Amplification Factor.

4. **Analyze the results with Julia**
   ```bash
   julia tools/plot_metrics.jl snapshots.json
   julia tools/halflife_report.jl snapshots.json
   ```
   *(Install Julia and required packages first: `julia -e 'using Pkg; Pkg.add(["JSON", "Plots"])'`)*

---

## What you'll see in a benchmark

Not just "queries per second". We track things like:

- **Factor Utilization** – the percentage of query work that stayed in rectangle‑space without ever building rows.
- **Update Amplification Factor (UAF)** – how many graph nodes a single object update touches. (We aim to keep it low.)
- **Factor half‑life** – how long a learned rectangle sticks around before it's evicted. (Are your workload patterns stable or transient?)
- **Adaptation speed** – after an abrupt workload change, how quickly does the factor set realign?

These numbers tell you whether the engine is truly behaving as an execution model, not just a clever compression trick.

---

## The philosophy behind the code

We believe databases should adapt to the people using them, not the other way around. JimVD is an attempt to build something that feels almost organic: it notices what you care about, remembers it, and forgets what you don't. The math (formal concept analysis, Boolean matrix factorization) is just the scaffolding; the real goal is to make a database that's a good collaborator, not a rigid machine.

If that sounds a little idealistic, that's because it is. We're still learning what works and what doesn't. You're warmly invited to poke around, run the adversarial tests, break things, and tell us what you find.

---

## Contributing

We're in the early, messy, exciting stage where ideas turn into code. If you're intrigued by factorized storage, incremental computation, or just want to hack on a database engine that doesn't look like anything else, please jump in. Open an issue, start a discussion, or send a pull request. We're especially interested in:

- More benchmark scenarios (especially from real-world categorical datasets)
- Alternative factor‑discovery algorithms
- Visualizations that make the internal graph visible
- Any performance surprise you uncover

---

## License

MIT

---

*JimVD – Just In Metadata Virtual Database – because your data has structure you haven't met yet.*
