# OpenManic MVP Terra implementation handover

- Accepted implementation baseline: `a3d5d9ffebca59bb5eba9d8718a78ba668a8aa61`
- Baseline branch: `codex/openmanic-mvp-implementation`
- Prepared: 2026-07-19
- Intended implementer: one Terra coding session acting as the sole writer
- Review authority after handback: the user and primary Codex reviewer

## 1. Purpose

This document is the self-contained implementation handover for continuing the OpenManic Windows MVP outside the current Codex task. Read it completely before editing.

The immediate objective is to implement the complete remaining plan, OM-040 through OM-740, in dependency order. Do not stop at OM-050 or at a phase gate solely because independent verification is pending.

The user intends to implement first and run full review, integration, platform, performance, recovery, and release checks after the complete implementation is handed back. Mark every completed task `implemented, unverified` until that review. During implementation, use narrowly targeted compiler or test feedback only when it is needed to keep work moving; recurring comprehensive checks are deliberately deferred.

## 2. Canonical sources and precedence

Read these files before starting:

1. [Product requirements](../openmanic-gui-product-requirements.md) for user-visible behavior.
2. [Implementation plan](../spec/implementation-plan.md) for task order, ownership, evidence, and gates.
3. [Architecture](../spec/architecture.md) for process and frontend/backend boundaries.
4. [Project structure](../spec/project-structure.md) for crates, modules, features, and dependency directions.
5. [Code quality standards](../spec/code-quality-standards.md) for formatting, linting, errors, tests, documentation, dependencies, and readability.
6. [Performance and reliability](../spec/performance-and-reliability.md) for fixtures, measurements, responsiveness, queues, and recovery.
7. [UI implementation](../spec/ui-implementation.md) for egui frame-path and interaction rules.
8. [Platform adapters](../spec/platform-adapters.md) for the Windows-first adapter boundary.
9. [Delivery and setup](../spec/delivery-and-setup.md) for portable Windows delivery.
10. [Implementation ledger](task-ledger.md) for accepted task evidence and active ownership.
11. Repository-root `AGENTS.md` for worktree, Git, ownership, and handoff rules.

If sources conflict, use the precedence above and stop for the user rather than inventing behavior. A task brief may narrow writable paths but may not weaken product or architectural requirements.

## 3. Exact repository state

The accepted implementation state before adding this handover document was:

```text
repository: F:\claude\projects\OpenManic
branch:     codex/openmanic-mvp-implementation
accepted ancestor: a3d5d9ffebca59bb5eba9d8718a78ba668a8aa61
```

No implementation task is active in the ledger. No secondary worktree remains attached.

Before editing, run:

```powershell
git rev-parse --show-toplevel
git branch --show-current
git rev-parse HEAD
git status --short
git worktree list
git merge-base --is-ancestor a3d5d9ffebca59bb5eba9d8718a78ba668a8aa61 HEAD
```

The final command must succeed. `HEAD` will be later than the accepted ancestor because it includes this handover document and may include user-prepared continuation-branch metadata. Record every later commit before editing. If the accepted ancestor is missing, the worktree is dirty, or later commits contain unexplained implementation changes, stop and report the observed state. Do not reset, clean, delete, or overwrite changes that were not created by your task.

### 3.1 Continuation branch policy

Do not commit new implementation directly onto `codex/openmanic-mvp-implementation`. Have the user prepare one continuation branch or isolated worktree containing this handover and descending from the accepted implementation ancestor above, then work serially there.

Because there will be one Terra writer, prefer one continuation branch with small ordered commits. Extra per-milestone branches are unnecessary. If the user explicitly enables parallel writers, each writer must have a separate worktree and completely disjoint writable paths.

Never run two writers that can touch the same manifest, lockfile, source file, fixture directory, decision record, or generated evidence file.

## 4. Completed and accepted foundation

### 4.1 OM-010 workspace foundation

Accepted implementation includes:

- Rust `1.97.1` pinned through `rust-toolchain.toml`.
- Rust 2024 workspace with six production crates.
- Rustfmt, strict workspace Clippy/rustdoc lints, `deny.toml`, `.editorconfig`, `.gitattributes`, and feature guards.
- Windows/WGPU default feature selection and separate Windows/Glow compilation.
- Exactly one renderer and one platform family per artifact.

Accepted integration evidence is recorded in the ledger. Do not restructure the workspace or change feature policy incidentally.

### 4.2 OM-020 Rust quality runner

`tools/xtask` implements:

```text
cargo xtask quality
cargo xtask docs-check
cargo xtask dependency-check
cargo xtask release-check
```

Important behavior:

- `quality` runs formatting, locked workspace checks, strict Clippy, tests, rustdoc warnings-as-errors, and documentation checks.
- `docs-check` validates all Markdown under `docs/gui` without network access.
- `dependency-check` requires the external executable `cargo-deny 0.20.2`; xtask never installs it.
- `release-check` includes the separate Windows WGPU/Glow matrix, selected WGPU release build, artifact-size report, and explicit manual smoke prerequisites.

The absence of `cargo-deny` is currently expected. Do not weaken the pin or add it as a Rust dependency.

### 4.3 OM-030 deterministic fixtures

`tools/fixture-generator` and `fixtures/performance` implement:

- dependency-free SplitMix64 deterministic randomness;
- independent manual UTC-microsecond and monotonic-tick clocks;
- generic scripted-input and recording-sink test helpers;
- the ten frozen performance scenarios;
- at least 10,000 raw intervals before aggregation;
- overlapping, independently segmented timeline bands;
- rapid A -> B -> A and same-application window changes;
- UTC-consistent adjacent, overnight, America/New_York DST, and recurrence-exception schedule evidence;
- simultaneous activity/focus/schedule layers;
- 1,000-entry application and category lists;
- browser title rates at 10, 50, and 100 changes per second with bounded retained titles;
- interleaved tracking/import/Overview jobs;
- a behavioral one-slot latest-snapshot coalescing fixture;
- streaming JSONL, deterministic metadata, FNV-1a checksums, atomic publication, identical-rerun acceptance, and differing-file overwrite refusal.

The default seed is `2_026_030`. `fixtures/performance/expected-metadata.json` is the committed regression oracle. These are synthetic inputs, not measured performance results.

## 5. Current verification state

At the accepted OM-030 integration head, the following passed:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked
cargo xtask docs-check
cargo xtask quality
```

The fixture generator has 25 unit tests across its library and binary. A controlled default-seed `all` generation produced ten JSONL files plus `metadata.json`, and the metadata matched the committed oracle exactly.

The full integration quality gate passed before the ledger-only handover commit. The handover document itself must pass `cargo xtask docs-check` before it is committed.

## 6. Environment

Tested Windows tools:

```text
rustc: 1.97.1 (8bab26f4f 2026-07-14)
cargo: 1.97.1
cargo executable: C:\Users\abr\.cargo\bin\cargo.exe
Visual Studio Build Tools: C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools
Developer environment: C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat
```

Initialize MSVC before builds when the shell does not already contain the developer environment:

```powershell
$dev = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat'
$lines = & $env:COMSPEC /d /s /c "`"$dev`" -arch=x64 -host_arch=x64 >nul && set"
foreach ($line in $lines) {
    if ($line -match '^([^=]+)=(.*)$') {
        Set-Item -Path "Env:$($Matches[1])" -Value $Matches[2]
    }
}
$env:RUSTUP_TOOLCHAIN = '1.97.1-x86_64-pc-windows-msvc'
```

The warning about denied access to `C:\Users\abr\.config\git\ignore` is environmental and non-blocking. Do not change global Git configuration to suppress it.

## 7. Implementation-first cadence

Use these rules to avoid the delays seen in earlier large assignments:

1. Work in milestones with one coherent outcome and normally no more than a few related modules.
2. Send or record a heartbeat immediately after repository preflight.
3. Implement the milestone before considering checks. Do not run a full gate per task or phase.
4. Commit each coherent milestone separately with the task ID, even though its verification remains deferred.
5. Review the actual base-to-head diff before starting the next milestone.
6. Record the task as `implemented, unverified`, list checks as deferred, and continue to the next dependency-ready task.
7. Do not wait silently on a blocked edit or command. Report the exact failure and current clean/dirty state.

Do not routinely run `cargo xtask quality`, workspace-wide formatting, Clippy, tests, documentation tests, dependency or release checks, renderer matrices, or full measurement campaigns during this implementation pass. Those checks belong to the later review led by the user and Codex.

When compiler feedback is necessary to continue safely, prefer the smallest relevant command, for example:

```text
cargo check -p <owned-package> --all-targets --locked
cargo test -p <owned-package> <specific-test> --locked
cargo fmt -p <owned-package>
```

These commands are optional diagnostics, not mandatory task gates. Do not represent them as a full workspace, platform, performance, recovery, or release pass.

## 8. Full implementation order

### 8.1 OM-040 native renderer and measurement fixture

Read the exact OM-040 row in the implementation plan and Sections 2 through 5 of the performance specification.

Outcome:

- Build an isolated minimal native egui/eframe fixture without introducing production application contracts.
- Exercise the accepted OM-030 dense and representative scenarios.
- Provide reproducible WGPU and Glow builds as separate artifacts.
- Instrument shell-ready timing, frame CPU/full-frame timing, dense paint preparation, memory observation hooks, artifact size, and environment metadata.
- Define a repeatable cold/warm measurement procedure and p50/p95 calculation method.
- Record driver/renderer failures explicitly rather than hiding them behind fallback.

Suggested owned surfaces:

```text
tools/native-ui-fixture/**
fixtures/performance/native-ui/**
```

Root `Cargo.toml`, `Cargo.lock`, renderer features, release profiles, and shared fixture formats are exclusive shared files. If OM-040 requires them, activate OM-040 as the sole writer and list each exact file in its writable allowlist. Do not let OM-050 run concurrently while either task can change the lockfile.

OM-040 must not claim measured product performance from mocked, headless, debug, virtualized, or unnamed hardware results. Harness code and diagnostic runs are useful, but release evidence requires the complete hardware/OS/build manifest in the performance specification.

Expected milestone split:

1. Compile-safe native fixture shell and feature selection.
2. Deterministic dense paint/input workload using OM-030 fixtures.
3. Instrumentation and structured measurement output.
4. Documented Windows measurement procedure and diagnostic run.
5. Implementation evidence manifest with all unrun checks marked deferred.

### 8.2 OM-050 low-fidelity UI and interaction direction

Start after OM-040 unless the user has prepared disjoint manifests, lockfile ownership, worktrees, and output paths.

Outcome:

- Build mock-snapshot, low-fidelity egui flows for Today, Timeline, Overview, Calendar, and Settings.
- Cover timeline navigation/selection, categories, schedules, focus, widgets/layout editing, loading/empty/partial/error states, and progressive disclosure.
- Use immutable mock snapshots and typed UI-local actions; do not create storage/platform adapters or production application contracts.
- Show key logical widths and scaling behavior required by the product document.
- Produce reviewable alternatives for the unsettled time-distribution presentation.

Suggested owned surfaces:

```text
tools/ui-direction-spike/**
fixtures/ui-direction/**
```

The time-distribution presentation, visual direction, and any product-level interaction ambiguity still require later user review. Present clear alternatives and a recommendation. If no explicit decision is available, use the documented recommendation as a provisional choice behind a replaceable OpenManic-owned abstraction, record it clearly, and continue unaffected implementation. Do not hard-code an unapproved choice into an external or difficult-to-replace contract.

Expected milestone split:

1. Mock snapshot/action model and navigation shell.
2. Five screen flows and required non-happy states.
3. Timeline/widget/layout interaction spikes at required widths.
4. Distribution-presentation alternatives and review captures.
5. Implementation evidence manifest with all unrun checks marked deferred.

### 8.3 OM-060 and provisional G0 record

OM-060 is primary-owned acceptance-trace work. Terra may prepare a candidate mapping report, but it must not silently assign previously unowned product requirements or change the canonical specifications.

For this continuation pass, Terra may prepare the candidate mapping and continue into Phase 1, but it must not change canonical requirements or claim that G0 was independently reviewed. Record all of the following as provisional implementation inputs:

- whether OM-040 evidence is diagnostic or release evidence;
- the renderer and performance budgets used, including their source and provisional status;
- the UI direction and time-distribution presentation used;
- the candidate OM-060 mappings from product criteria and detailed MUST requirements to tasks and evidence cases;
- that independent G0 verification is deferred.

When an explicit user choice is absent, prefer the documented recommendation and isolate it behind an OpenManic-owned type, adapter, or configuration boundary so it remains replaceable. If there is no safe provisional implementation, record the blocker and continue every unaffected task instead of stopping the entire effort.

### 8.4 Phases 1 through 7

Continue through the complete task graph in the implementation plan:

- Phase 1: OM-100 through OM-151.
- Phase 2: OM-200 through OM-299.
- Phase 3: OM-300 through OM-332.
- Phase 4: OM-400 through OM-412.
- Phase 5: OM-500 through OM-520.
- Phase 6: OM-600 through OM-643.
- Phase 7: OM-700 through OM-740.

For every task:

1. Read its exact plan row, dependencies, canonical specification sections, owned paths, and evidence contract.
2. Implement in dependency order without duplicating public contracts, shared types, migrations, or generated formats owned by another task.
3. Keep one writer active and serialize work that can touch the same file, manifest, lockfile, migration sequence, or shared contract.
4. Commit a coherent change with the task ID.
5. Update the continuation record to `implemented, unverified`, list checks as deferred, and note provisional decisions or blockers.
6. Continue to the next dependency-ready task without waiting at a phase gate.

The plan's original gate criteria remain eventual acceptance requirements. This handover changes when they are verified, not what the product must satisfy.

## 9. Architecture guardrails

The Windows MVP remains:

- Rust and egui/eframe only for application code;
- entirely local and offline;
- SQLite-backed when persistence begins;
- portable, without a runtime installer or database server;
- Windows 11 x86-64 first;
- WGPU selected provisionally, with Glow retained only as a separately built comparison artifact until OM-040 decides;
- frontend/backend separated through typed application contracts and immutable snapshots;
- OS focus detection behind an adapter boundary;
- free of telemetry, network updates, accounts, teams, administrators, and employee-monitoring behavior.

UI code must never perform SQLite, platform capture, recurrence expansion, import, or full-history aggregation work in the egui frame path. Spike code must not be mistaken for production layering.

Do not add Tokio, Rayon, an ORM, a plugin framework, a second serialization stack, or another nontrivial dependency without explicit user approval and a recorded footprint/feature rationale.

## 10. Code and safety requirements

- Follow rustfmt and inherited workspace lints.
- No production `.unwrap()`, `.expect()`, `panic!`, `todo!`, `unimplemented!`, or `dbg!`.
- Use typed errors for expected failures and assertions only for programmer invariants.
- Keep unsafe code forbidden outside narrowly contained future platform FFI.
- Public and non-obvious behavior needs rustdoc explaining ownership, units, invariants, and failure behavior.
- Prefer product vocabulary and specific modules; do not introduce vague `common`, `helpers`, `manager`, or `utils` modules.
- Tests are deterministic, offline, and free from arbitrary sleeps or uncontrolled wall time.
- Generated output goes only to an explicit owned directory, uses stable ordering, and is never committed accidentally unless the task explicitly names it.
- Never fabricate benchmark, platform, accessibility, privacy, recovery, or release evidence.

## 11. Git and commit policy

Use small commits such as:

```text
chore(perf): [OM-040] add native measurement fixture
feat(spike): [OM-050] add mock screen navigation
test(spike): [OM-050] cover loading and error states
```

Rules:

- Stage exact owned paths only; never `git add .` or `git add -A`.
- Do not amend an earlier accepted task commit.
- Do not merge, rebase, push, reset, clean, or delete unrelated files.
- Keep the continuation worktree clean at every milestone handoff.
- Preserve all existing commits and the accepted OM-010/020/030 behavior.

## 12. Required handback package

After implementing the complete remaining plan, return all of the following to the user:

```yaml
baseline_sha: a3d5d9ffebca59bb5eba9d8718a78ba668a8aa61
branch:
head_sha:
commits:
tasks_attempted:
tasks_complete:
changed_files:
dependencies_or_lockfile_changes:
feature_changes:
public_contract_changes:
unsafe_inventory:
user_decisions_needed:
diagnostic_measurements:
release_evidence: none unless the complete required manifest exists
commands:
  - command:
    exit_code:
    result:
tests_added:
checks_deferred:
known_limitations:
remaining_risks:
git_status:
```

Also provide:

1. The complete base-to-head diff summary.
2. Exact generated evidence paths and whether each is committed or temporary.
3. Screenshots or measurement reports only when their environment and method are recorded.
4. Every unresolved product, renderer, dependency, performance-budget, or ownership decision.
5. A statement that task, phase, integration, platform, performance, recovery, and release verification are all still pending.

Do not say the MVP, a gate, or a task is accepted. Terra implements and reports evidence; the user and primary Codex reviewer make the final decision.

## 13. Terra kickoff checklist

Before the first edit:

- [ ] Read this handover and every canonical source in Section 2.
- [ ] Verify repository root, continuation branch, exact baseline ancestry, and clean status.
- [ ] Confirm Terra is the only writer.
- [ ] Choose OM-040 first and freeze exact writable paths.
- [ ] Record whether root manifest/lockfile ownership is transferred exclusively.
- [ ] Map every OM-040 requirement to a milestone and evidence case.
- [ ] Record checks as deferred and use targeted compiler feedback only when needed to continue.
- [ ] Record provisional choices and keep them replaceable rather than stopping unaffected work.

Begin with OM-040 milestone 1, then continue through OM-740 in dependency order. Stop immediately only if the baseline or ownership state differs, there is a data-safety risk, or a missing decision makes all remaining work impossible.
