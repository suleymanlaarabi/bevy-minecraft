- Always write code and project content in **English**, including identifiers, comments, documentation, UI text, and error messages.
- Never compile the project in **release** mode.
- Verify changes using only `cargo check`. Do not run the project.
- Do not validate performance optimizations with benchmarks or tests. Code inspection and reasoning are sufficient.
- Make full use of **Bevy** and its modern features.
- Favor **ECS**, **parallelism**, and **data-oriented design** wherever possible.
- Performance is paramount.
- This is an unreleased beta: never retain legacy code, backward compatibility, compatibility shims, or migrations. Replace obsolete interfaces and formats directly, and reject incompatible saves or imported assets explicitly.

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
