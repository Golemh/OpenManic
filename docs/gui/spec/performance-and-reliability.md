# OpenManic MVP performance and reliability specification

## 1. Purpose

This document turns “lightweight, smooth, and safe” into measurable budgets and failure behavior. The targets apply to release builds on named reference hardware and are verified with reproducible fixtures.

The UI must feel immediate even when background work is not immediate. Startup may reveal the shell before historical aggregation, and refresh may preserve prior data while a new snapshot is built.

## 2. Responsiveness budgets

### 2.1 Approved targets

| Measure | MVP target |
| --- | --- |
| Action acknowledgement | Visible by the next rendered frame under normal conditions |
| Routine p95 full-frame time | No greater than 16.7 ms on the reference 60 Hz path |
| OpenManic p95 UI CPU work | Provisional target no greater than 4 ms during routine interaction |
| 120 Hz full-frame time | 8.3 ms stretch target, measured but not a release failure unless explicitly promoted |
| Tray restore to interactive retained UI | p95 no greater than 100 ms |
| Warm process launch to usable shell | p95 no greater than 1 second |
| Cold launch to usable shell | p95 no greater than 2 seconds |
| Long-operation feedback | Visible progress/activity for work expected to exceed 250 ms |
| Timeline data scale | At least 10,000 raw intervals in the selected test day/range before render aggregation |

The 4 ms UI CPU budget is intentionally stronger than the full-frame target and remains provisional until the representative eframe spike measures renderer/driver overhead. The 16.7 ms product requirement remains binding.

### 2.2 Definitions

- **Action acknowledgement**: the first frame that visibly shows selection, pressed/pending state, changed mode, local draft, or an error accepting the command.
- **Usable shell**: main window painted with navigation, tracking status, primary controls, and a clear loading/prior-data state. Historical widgets need not be fully aggregated.
- **Warm launch**: executable and common libraries are in OS cache; process was not already running.
- **Cold launch**: first measured launch after the benchmark protocol clears or controls relevant cache according to its documented method.
- **Tray restore**: the process and UI model already exist; measurement begins at accepted tray/activation command and ends when the restored viewport responds to input.
- **Full-frame time**: end-to-end frame duration reported by the instrumented native fixture, including OpenManic UI work and selected renderer submission.
- **UI CPU work**: time spent in OpenManic’s event drain, reducers, layout, hit testing, and paint preparation, excluding sleeping/waiting.

## 3. Reference hardware and protocol

Before performance work is accepted, publish a fixture manifest containing:

- Exact Windows 11 build.
- CPU model, physical/logical core count, and power mode.
- Installed RAM.
- GPU and driver.
- Display refresh rate and scaling.
- Storage type and free space.
- Rust version, Cargo profile, git revision, renderer feature, and dependency lockfile hash.
- Antivirus/security configuration relevant to launch measurement.
- Synthetic generator seed and dataset metadata.
- Warm/cold measurement procedure.
- Sample count and p50/p95 calculation method.

Results without this manifest are diagnostic observations, not release evidence.

## 4. Benchmark datasets

The deterministic Rust fixture generator creates:

1. A normal workday with realistic application/category durations.
2. A dense day/range containing at least 10,000 raw activity intervals.
3. Three independently segmented category/state/application bands.
4. Rapid A -> B -> A foreground changes and many same-app window switches.
5. Dense adjacent schedule brackets, overnight schedules, recurrence exceptions, and DST boundary dates.
6. Simultaneous activity, focus, and schedule overlays.
7. Large application/category lists.
8. Rapid browser title observations at 10, 50, and 100 changes per second.
9. Concurrent tracking writes, import batches, and Overview projection.
10. A deliberately slowed UI to test queue behavior and coalescing.

The fixture metadata records raw interval counts before projection/downsampling.

## 5. Timeline performance design

### 5.1 Projection

The background projection produces:

- Sorted, gap-explicit Category, Activity-state, and Application bands.
- Visible schedule occurrences and focus overlays.
- Stable segment IDs and descriptive metadata.
- Raw interval indexes for exact hit testing.
- Optional paint-level aggregation for subpixel density.

### 5.2 UI geometry

The renderer MUST:

- Allocate one egui interaction `Response` for the timeline, not one per interval.
- Convert pointer X to time once.
- Binary-search the relevant band’s raw interval index for hover/selection.
- Binary-search visible range boundaries and paint only visible/near-visible segments.
- Aggregate paint geometry at subpixel density without changing raw totals/identity.
- Use the same time-to-pixel transform for all bands, selections, ticks, focus overlays, and schedule brackets.
- Avoid cloning complete histories per frame.

### 5.3 Cache keys

Data-space projection cache:

```text
data revision
range and time-zone context
filters and grouping
widget configuration revision
projection schema revision
```

Screen-geometry cache:

```text
snapshot revision
allocated bounds
zoom/pan
pixels-per-point/scaling
theme revision
```

Hover changes MUST NOT invalidate data-space projection. Theme changes may invalidate paint geometry without rerunning storage aggregation.

## 6. Repaint policy

- Workers publish state before requesting repaint.
- Live tracking and focus timers use authoritative timestamps, not event-per-frame updates.
- Idle windows request no continuous maximum-rate repaint.
- Animations are interruptible and stop requesting repaint when complete.
- Progress is coalesced to approximately 10-30 visible updates per second unless a specific visual requires more.
- A background refresh keeps valid prior content visible.

## 7. Startup path

The critical startup path contains only:

1. Parse minimal CLI/bootstrap configuration.
2. Resolve single instance and data directory.
3. Install minimal local diagnostics/panic hook.
4. Validate/open SQLite and perform only immediate compatibility checks.
5. Start supervisor, writer, platform, and projection worker.
6. Create eframe and show the shell/tracking status.
7. Request historical/layout/widget projections asynchronously.

Do not decode all icons, aggregate history, parse external themes, run full integrity checks, import files, or expand broad recurrence ranges before showing the shell.

Migrations that are not immediate enter an explicit migration/recovery experience before normal tracking; they do not run during an egui frame.

## 8. Memory and cache budgets

Exact byte ceilings are established by the initial measurement spike, but the following policies are binding:

- Presentation snapshots share substantial immutable data with `Arc` or equivalent.
- No per-frame full-history clone.
- Icon and decoded-image caches are bounded by entry count and estimated bytes.
- Geometry caches are bounded and revision-keyed.
- Projection/occurrence caches are rebuildable and evictable.
- Logs rotate and profiler traces are development-only.
- Channel and mailbox sizes are fixed/configured, never unbounded.
- Imported rows stream through bounded staging/batches.

Release diagnostics record peak resident memory for startup, idle, dense timeline interaction, and concurrent import/tracking scenarios.

## 9. SQLite latency and durability

### 9.1 Initial policy

- WAL mode.
- `synchronous=FULL`.
- One writer connection.
- One initial projection reader.
- Short read transactions.
- Tracking transitions have higher writer priority than bulk-import batches.
- Open interval checkpoints approximately every five seconds.

### 9.2 Measurement

Record:

- Writer queue depth/high-water mark.
- Transaction wait, execution, and commit latency.
- Tracking-transition commit p50/p95.
- Checkpoint duration and WAL size.
- Read transaction duration.
- Import batch size/throughput.
- Busy retry/exhaustion counts.

If `FULL` causes user-visible delay, first adjust batching, transaction size, writer scheduling, and checkpoint timing. Moving to `NORMAL` is allowed only after a documented benchmark and explicit durability exception because recent committed transactions may be lost after OS/power failure.

## 10. Backpressure verification

Every bounded path has a measurable policy:

| Path | Overload behavior |
| --- | --- |
| User commands | Small UI outbox + Busy/Pending; no block or silent drop |
| Critical events | Prioritized drain; service stops accepting more work before silent loss |
| Platform ingress | Atomic evidence-loss marker, `UnknownMissing`, immediate reconciliation |
| Snapshots | Replace older snapshot in same slot |
| Progress | Replace older progress for same job |
| Repaint hints | Coalesce/drop |
| Diagnostics | Drop low-severity entries with dropped-count metric |

The deliberately slowed-UI fixture proves that queue depths remain bounded and the latest correct snapshot wins.

## 11. Assertions and validation

### 11.1 Use `Result` for expected failures

Expected failures include:

- Invalid user input.
- Schedule overlap or invalid recurrence.
- Permission/access limitations.
- Adapter disconnect or missing foreground identity.
- SQLite busy/unavailable/corrupt response.
- Malformed configuration/layout/theme/CSV.
- Cancellation.
- Read-only/unwritable data location.

They return typed errors with stable codes, user-safe summaries, recovery actions, and optional technical sources.

### 11.2 Use types and constraints first

Prevent invalid state with:

- Newtypes for IDs and UTC microseconds.
- Constructors for positive intervals and validated names/zones.
- Exhaustive enums.
- State-machine transition methods.
- SQLite `NOT NULL`, `CHECK`, `UNIQUE`, FK, partial-index, and `STRICT` constraints.
- Serialized writer checks for cross-row overlap and revision conflicts.

### 11.3 Assertion policy

- `debug_assert!` checks expensive internal consistency in development/tests.
- `assert!` is allowed only for a programmer invariant where continuing could corrupt authoritative state or produce unsafe persistence behavior.
- User data and platform input are never assertion conditions.
- Every release assertion has a test that demonstrates the invariant’s intended boundary.
- Domain/application/UI/storage crates forbid local unsafe code. Platform FFI exceptions are documented and isolated.

The compiler, Clippy, exception, unsafe-block, documentation, and test-hygiene enforcement for these rules is defined in [Code quality and readability](code-quality-standards.md).

## 12. Panic policy

Release builds use unwinding rather than `panic=abort` because controlled thread-root failure reporting is part of the accepted reliability policy.

### 12.1 Global hook

The global panic hook may:

- Write a minimal local crash marker and safe metadata.
- Record thread name and panic location/message where available.
- Signal the supervisor through a nonblocking emergency flag.

It MUST NOT:

- Query or mutate complex application state.
- Take ordinary service locks.
- Assume SQLite is usable.
- Include raw window titles.
- Claim a successful final flush.

### 12.2 Thread-root boundaries

- Projection/bulk worker panic: mark job/service failed, close its read resources, recreate if safe.
- Platform thread panic: close trusted attribution, mark adapter unavailable, attempt controlled restart/probe.
- Writer panic: storage-fatal; no transparent restart.
- UI/top-level panic: attempt only a bounded emergency checkpoint request, flush already-buffered diagnostics, and exit nonzero. Do not resume the UI.

`catch_unwind` is containment at thread/process roots, not ordinary error handling.

## 13. Graceful shutdown and failure honesty

Explicit Quit MUST follow the shutdown lifecycle in [System architecture](architecture.md). If a critical flush fails, the UI offers Retry or Quit Anyway and does not claim success.

Forced termination, power removal, OS kill, or corrupted memory cannot guarantee graceful shutdown. OpenManic limits harm through durable transactions and checkpoints. After restart it labels uncertain attribution as Unknown/Missing rather than pretending no data was affected.

The approved durability promise is therefore:

- Completed transitions and successful transactions are protected by SQLite’s configured durability.
- A long open interval is confirmed at most approximately five seconds behind normal runtime.
- Unexpected termination may lose precise application attribution after the last checkpoint, but recovery preserves that period as explicit unknown history rather than silently omitting or inventing it.

## 14. Error presentation contract

Every surfaced error answers:

1. What failed?
2. Is tracking/user data still safe?
3. What action can the user take?
4. Where are optional technical details?

Error severity:

| Severity | Example | Presentation |
| --- | --- | --- |
| Inline validation | Invalid schedule range | Beside field; dialog stays open |
| Local recoverable | One projection failed | Prior content + local retry |
| Global recoverable | Adapter unavailable | Persistent shell status + remediation |
| Storage-protective | DB read-only/write failed | Block mutations; recovery actions |
| Fatal | Writer panic/incompatible newer schema | Recovery screen, local diagnostics, controlled exit |

Background refresh never steals focus or replaces valid prior content with an empty spinner.

## 15. Local diagnostics

Use structured `tracing` spans for:

- Frame and UI event-drain duration.
- Screen/widget render CPU time.
- Command submission, confirmation, rejection latency.
- Queue depth/high-water marks.
- Platform adapter transitions and evidence loss.
- SQLite read/write/checkpoint/migration duration.
- Projection request, cancellation, cache hit/miss, and publish latency.
- Import/export batch progress.
- Shutdown phase duration/failure.

Rules:

- Log files live under the selected OpenManic data root.
- Logging is local and rotated.
- Normal logs exclude raw titles.
- Detailed sensitive diagnostics require a separate warned opt-in.
- Off-thread log writing is bounded and reports dropped entries.
- Profiler UI/traces are behind `dev-tools` and absent from normal artifacts.

## 16. Verification layers

### 16.1 Unit/property tests

- Domain interval/state invariants.
- Tracker reducer cause precedence.
- Focus state/deadline/restart behavior.
- Date range and local-day boundaries.
- Recurrence, DST gap/fold, zone-change segments, overnight intervals.
- Schedule overlap and edit scopes.
- Layout/view/theme validation and migration.
- Selection/filter reducers.
- Stale-result rejection.
- Timeline transforms, binary-search hit testing, and bracket alignment.
- Title stabilizer/deduplication under high churn.

### 16.2 Storage integration tests

- Foreign-key/STRICT constraints and all migrations.
- Writer revision atomicity.
- Concurrent reader/writer snapshot consistency.
- WAL growth with intentionally slow reader.
- Busy handling.
- Open checkpoint recovery.
- Pre-migration online backup/restore.
- Import staging, cancellation, and idempotent self-reimport.
- Data move verification.

### 16.3 Application integration tests

- Platform event -> reducer -> persistence -> projection -> UI event.
- Hidden/stalled UI while tracking continues.
- Optimistic command confirmation/rejection rollback.
- Timeline/Calendar schedule parity.
- Category assignment refreshing historical projections.
- Focus behavior across hide/simulated resume/restart.
- Queue overload and coalescing.
- Coordinated shutdown and flush failure.

### 16.4 GUI verification

- Product-document task flows.
- All loading/refreshing/empty/partial/error/recovered states.
- Continuous three-band timeline without accidental gaps/labels.
- Hover, selection, pan, pointer-anchored zoom, range selection, schedule-create mode.
- Compact/expanded widget behavior and layout editing.
- 720/1024/1440 logical widths.
- 125/150/175/200 percent scaling.
- Theme consistency across egui and custom paint.
- Ordinary egui keyboard behavior.

### 16.5 Platform and performance tests

Use the platform matrices in [Platform adapters](platform-adapters.md) and the benchmark datasets in Section 4. These tests run on real platform environments where mocks cannot establish behavior.

## 17. Release evidence

The technical verification report records:

- Passing test suite versions.
- Exact benchmark manifest.
- p50/p95 frame, UI CPU, startup, restore, projection, and commit results.
- Peak memory and artifact size.
- Selected renderer and comparison result.
- Known platform degradation/non-guarantees.
- Any approved performance or durability exception.

This is verification evidence, not a new milestone, risk register, or product traceability matrix.

## 18. Primary references

- [Rust error handling](https://doc.rust-lang.org/stable/book/ch09-00-error-handling.html)
- [`debug_assert!`](https://doc.rust-lang.org/std/macro.debug_assert.html)
- [`std::panic::set_hook`](https://doc.rust-lang.org/std/panic/fn.set_hook.html)
- [`catch_unwind`](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html)
- [SQLite WAL](https://www.sqlite.org/wal.html)
- [SQLite synchronous modes](https://www.sqlite.org/pragma.html#pragma_synchronous)
- [Criterion](https://bheisler.github.io/criterion.rs/book/)
- [`tracing`](https://docs.rs/tracing/latest/tracing/)
