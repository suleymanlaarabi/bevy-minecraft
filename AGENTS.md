# Project rules

- Write code and project content in English.
- Verify code changes with development-profile `cargo check`. Never run tests or ordinary gameplay. Development benchmarks may run only when explicitly authorized by the user, as authorized for the current performance task.
- Never compile, check, run or benchmark a release profile, including `--release`, `--profile release` or equivalent custom profiles. Performance investigations do not override this prohibition.
- Use modern Bevy features, ECS, parallelism and data-oriented design. Performance is paramount.
- This is an unreleased beta: saves and generated caches are disposable. Replace obsolete interfaces directly; never add legacy readers, compatibility shims, migrations, format-version fields or version checks.
- Preserve concurrent user edits. Change only what the requested task requires.

## Find the relevant code

| Area | Start here |
| --- | --- |
| IDs, catalogs, cells, coordinates, sections, terrain encoding | `crates/core/src` |
| Shared voxel access, edits, streaming, lighting, simulation, storage | `crates/engine/src` |
| Meshing, materials, embedded shaders, visibility, uploads | `crates/render/src` |
| Shared slots, motion, articulation, following, cursor | `crates/features/src` |
| Gameplay, player, inventory/UI, Minecraft adapters, sky, audio | `crates/minecraft/src` |
| Terrain generation and native data adapters | `crates/worldgen/src` |
| Application composition | `src/main.rs` |

- Locate files with `rg --files <area>` and symbols with scoped `rg -n`. Read relevant sections, then follow callers and dependencies only as needed for correctness. Stop expanding once the affected behavior and constraints are understood.
- For cross-project exploration, exclude bulk data: `rg -n -g '*.rs' -g '!**/generated/**' <pattern> src crates`. Use `rg -l` when only filenames are needed. Narrow an overflowing search instead of repeatedly reading truncated output.
- Generated registry sources in `crates/worldgen/crates/pumpkin-data/src/generated` are very large. Consult exact symbols there only for native definitions or generation changes; do not read whole generated files. Search shaders separately in `crates/render/src/shaders`.
- Use the crate map above for boundaries and current code for implemented behavior. Read `crates/worldgen/README.md` only for generator context and `ROADMAP.md` only for planning. No documentation sweep or additional context document is required.

## Targeted Cargo verification

- For an internal change, start with `cargo check -p <affected-package>`. Group affected packages with repeated `-p` flags instead of rerunning overlapping checks.
- When a shared interface changes, also check its affected consumer packages. Use the manifests and callers to identify consumers; a library check alone does not verify them.
- For conditional code, check the default configuration and the affected feature configurations, including `dev`, `sqlite` or `content-ron` where relevant. Check the owning package independently when workspace feature unification could hide a missing feature or dependency.
- Reserve `cargo check --workspace` for changes spanning the workspace or shared dependency configuration. Do not repeat a successful check unless a new change or unresolved failure requires it.
- Check crate examples only when affected, using `cargo check -p <package> --example <name>` with the relevant features. Do not use `--all-targets`, which includes test and benchmark targets.
- Documentation, instruction and Codex configuration changes need no Cargo check. Changed Cargo manifests require an appropriate targeted check. Never run tests, ordinary gameplay or release profiles; only explicitly authorized development benchmarks are exceptions to the application execution restriction.

## Targeted performance review

- Performance optimizations must reduce actual runtime work in production code and benefit release execution as well as development builds.
- Never trade development compilation speed for runtime FPS. Do not change compiler backends, Cargo profiles, optimization levels, codegen units, LTO, compiler flags, debug assertions or overflow checks as a performance fix. Preserve the existing fast development build configuration.
- A performance request never authorizes build-configuration changes. This includes per-crate overrides, dependency build settings, toolchains, target CPU/features, environment variables such as `RUSTFLAGS` or `CARGO_PROFILE_*`, and temporary command-line overrides. Change these only when the user explicitly requests that specific build-configuration work; report it separately from production-code optimizations.
- Keep build settings identical for the baseline and optimized measurements. Never switch a crate from Cranelift to LLVM, raise its optimization level, or disable checks to claim a performance gain, even temporarily or only for a benchmark.
- Never inflate performance results by reducing resolution, view distance, visual quality, world content, simulation work or benchmark workload. Compare the same scenario with the same build settings. Diagnostic ablations must be explicitly authorized, clearly labeled and excluded from optimization gain claims.
- Development-only profiling does not establish release bottleneck rankings or release speedups. Distinguish observed hotspots, proposed algorithmic improvements and verified gains; never present a compiler configuration change or a lighter workload as a code optimization.
- If a development hotspot may come from its compiler backend or optimization settings, state that limitation and investigate the production algorithm before changing code. Do not work around it through compilation changes or claim a release benefit without release evidence. The prohibition on release execution still applies: describe release benefits as expected and unmeasured when only development evidence is available.
- Review the changed path and relevant dependencies: execution frequency, CPU work, allocations, footprint, locality and lookup complexity. Fix avoidable costs there; report material adjacent issues without expanding scope.
- Carry existing compact typed IDs, enums and Bevy handles at runtime. Resolve external names at import/initialization. Keep immutable definitions in shared catalogs and only IDs plus necessary state in instances.
- Prefer arrays, slices, indexed vectors and bitsets for dense bounded domains; account for sparsity and total storage. Choose integer widths without truncation or overflow.
- Avoid repeated strings, conversions, allocations, cloning, hashing and scans in hot loops. Use change detection and targeted scheduling; reuse buffers when their retained memory is justified.
- Justify non-obvious maps, reference counting, indirection or caches against lighter representations. Do not add speculative caches or duplicate state. Comment only material non-obvious tradeoffs near the decision.
- Before finishing, review affected data flow once for redundant work and storage. Describe expected improvements as unmeasured unless measured evidence exists.

## Bevy access safety

- Review every affected system's queries and resources for overlapping component reads/writes (`B0001`). `Changed<T>` and `Added<T>` also access `T` and may conflict with another query's `&mut T`.
- Prefer one query with `Mut<T>::is_changed()` / `is_added()`. Otherwise use sequential `ParamSet` access; use `With` / `Without` only for genuinely disjoint entities.
- `cargo check` does not detect runtime query conflicts or establish visual correctness; do not launch the game to bypass the validation restriction.

## Refactor accounting

- For actual production-code refactors, format the affected Rust files before the baseline and count production lines excluding blank/comment-only lines. Measure the same scope afterward, including added code. Refactors must reduce total production code; report missed targets.
- Formatting, naming, moves, removed useful comments/spacing, shorter names, merged statements, missing braces or inlined schemas/conditions are not reductions. Remove duplication, branches or dead plumbing, or use maintained library/framework features.
- Do not apply refactor accounting to ordinary fixes, feature additions or instruction/configuration edits.

## Work and output

- Use one agent by default; delegate only when explicitly requested. Use native tools and compaction rather than compression wrappers or mandatory context files.
- Stay within the request. Do not add unsolicited examples, documentation, files, extra features or offers of further work.
- Keep progress updates brief. Finish with the requested result, verification and material limitations or tradeoffs, without a lengthy recap. Expand only when the user requests an audit, plan or explanation.
