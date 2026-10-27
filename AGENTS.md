- Always write code and project content in **English**, including identifiers, comments, UI text.
- Never compile the project in **release** mode.
- Verify changes using only `cargo check`. Do not run the project.
- Do not validate performance optimizations with benchmarks or tests. Code inspection and reasoning are sufficient.
- Make full use of **Bevy** and its modern features.
- Favor **ECS**, **parallelism**, and **data-oriented design** wherever possible.
- Performance is paramount.
- This is an unreleased beta. Saved data and generated caches are disposable and are deleted for each update. Never retain legacy code, backward compatibility, compatibility shims, migrations, format-version fields, or version checks for saves or imported assets. Replace obsolete interfaces and formats directly. Do not write code to preserve or manage old data.

## Mandatory performance design and review

- Before implementing or modifying a feature, inspect its execution frequency, CPU work, allocation behavior, memory footprint, data locality, and lookup complexity. Choose the least costly representation and algorithm that satisfy the actual requirements. Do not default to convenient dynamic structures without evaluating their cost.
- Represent internal identities with compact, typed numeric IDs and closed sets with enums. Reuse existing registry IDs and Bevy handles instead of introducing parallel identity systems. Resolve external names once at import or initialization boundaries; carry IDs through runtime processing.
- For dense, bounded ID domains, prefer arrays, slices, directly indexed vectors, and bitsets over `HashMap` or `HashSet`. Choose integer widths from the valid domain without truncation or overflow. Account for domain sparsity and total storage before allocating a direct-index table.
- Do not introduce or retain avoidable string identities, name lookups, string construction, repeated allocations, cloning, or hashing in frequent gameplay, per-entity, per-voxel, meshing, or rendering loops. Precompute shared lookup tables and resolve invariant data outside these paths. Reuse buffers when this reduces allocation without retaining excessive memory.
- Store immutable definitions and profiles once in shared catalogs. Keep only IDs and necessary instance state in entities, cells, and items. Favor contiguous storage and avoid duplicated derived data or per-instance heap allocations when shared indexed data suffices.
- Use Bevy queries, change detection, and targeted work scheduling to avoid redundant scans and recomputation. Use parallel iteration and task execution when workload size and data dependencies justify their scheduling and synchronization costs. Review component access conflicts before changing systems.
- Every retained or newly introduced costly structure must have a concrete justification when a lighter representation is plausible. This includes numeric-keyed hash maps, reference counting, and indirection, not only strings. User-visible text, import data, and genuinely sparse keys can justify such choices; document the reason near non-obvious decisions and summarize material tradeoffs in the change report. Replacing `String` with `&str`, or changing a hash function, does not replace this analysis.
- Fix avoidable costs in the code being changed. Report adjacent performance problems outside the task's scope instead of silently expanding the task. Do not add speculative caches, duplicate state, or custom infrastructure without identifying the work they eliminate and their memory and maintenance costs.
- Before completing a change, review the affected data flow for repeated conversions, allocations, hashing, scans, and duplicated storage. Report which costs were removed and which tradeoffs remain. Distinguish expected improvements from measured results; never claim an unmeasured speedup or memory reduction as demonstrated. Follow the existing validation restrictions: code inspection and reasoning for performance, `cargo check` for code verification, no benchmarks, tests, release builds, or game launches.

## Bevy systems and query conflicts

- Before adding or modifying a system, review the accesses of **all** its `Query` parameters and resources. Queries that may match the same entities must not simultaneously read and mutate the same component (`B0001`).
- `Changed<T>` and `Added<T>` filters also access `T`: such a query can conflict with another query containing `&mut T`, even when `T` is absent from the first query's data.
- Prefer a single query with `Mut<T>::is_changed()` / `is_added()` for change detection. When multiple queries are necessary, use a `ParamSet` with sequential access; use `With` / `Without` only when the entity sets are genuinely disjoint.
- `cargo check` does not detect ECS access conflicts. Explicitly review the parameters of every affected system without running the game to bypass the verification rules above.

## Refactoring metrics

Formatting and naming changes are not code reduction.

Refactors must reduce total production code; report missed targets.

Format before refactoring; record production lines excluding blank/comment-only lines. Measure results the same way.

Never reduce lines by removing useful comments/spacing, shortening names, merging statements/declarations, removing braces, or inlining schemas/conditions. Eliminate duplication, branches, dead plumbing, or replace custom code with maintained framework/library features.
