# OpenManic implementation handover

## Resume point

- Branch: `codex/openmanic-mvp-implementation-continuation`
- Start from the branch tip, not an old task worktree.
- The implementation ledger at `docs/gui/implementation/task-ledger.md` is the ownership and integration record.
- The canonical next-work authority is `docs/gui/spec/implementation-plan.md`.

## User-directed execution cadence

Prioritize implementation throughput over per-task ceremony.

Follow the shared [agent execution and context strategy](agent-execution-strategy.md).

- Batch adjacent plan rows that share a crate/layer and have no unresolved dependency; keep the plan's ordering and product requirements intact.
- Use at most three implementation agents at once, normally one each for independent domain/application-storage, platform, and UI streams.
- Keep a small stable set of worktrees for a batch. Do not create a new worktree, ledger commit, verifier, or full review for every plan row.
- Run only focused compile/tests needed to unblock an author while work is in progress. Run one consolidated `cargo xtask quality` and one targeted read-only review at the end of a phase/gate.
- Windows-only/manual evidence belongs to the applicable phase gate. Do not turn it into repetitive task-level checks.
- Update the ledger at the start/end of a batch or phase, with the current integration SHA and any real limitation. Preserve its ownership record, but keep entries concise.

## Current state

Phase 0 and Phase 1 are accepted. Phase 2 component tasks OM-200 through OM-296 are integrated, including the runtime, storage, Windows adapters, timeline projection/interaction/rendering, Today controller, and summary presentation models.

OM-299 now has the primary vertical-slice composition in `crates/openmanic/src/composition.rs`: it keeps the bootstrap/data-root lock and instance owner alive for the full process, runs the exclusive SQLite writer and tracking service on the named writer worker, supplies immutable Today snapshots through a latest mailbox, routes tray/activation actions through bounded ingress, and drives ordered explicit Quit. The root has a direct, pinned `eframe` dependency so its renderer features match the selected UI renderer. Close-to-tray retains the process resources and tracking worker; the Today view exposes pause/resume controls with correlated pending/confirmed/rejected acknowledgement.

Use `cargo +stable-x86_64-pc-windows-msvc` for all Windows checks in this checkout. The configured GNU toolchain cannot build bundled SQLite because `gcc.exe` is absent; this is an environment limitation, not a product failure. The MSVC toolchain and `cl.exe` are installed and `cargo +stable-x86_64-pc-windows-msvc xtask quality` has passed after the current source repairs. Set `CARGO_TARGET_DIR=target-msvc` for that command and remove the generated directory afterwards.

The current primary-owned changes also repair Windows-only quality defects in `windows_single_instance.rs` and `windows_tray.rs`, and add `WindowsControlWindow::run_with_tray_actions`. That control-loop method forwards retained tray actions only after their native callback returns, so the composition may route Open/Pause/Resume/Quit without blocking a callback. Keep this bounded behavior; do not call storage or application services directly from a Win32 callback.

The resolver/catalog blocker is now wired: a changed live HWND is resolved on the normal control loop, mapped deterministically from a stable AUMID or normalized executable path, and placed on the writer lane with an application upsert before its `TrackingEvidence::Foreground` command. Unresolved/process-denied identity remains an explicit `ApplicationIdentity` degradation, and overflow still emits loss before a fresh foreground sample. The foreground catalog preserves its earliest and latest observed bounds across upserts.

Phase 2's implementation gate is accepted on the completed integration and `CARGO_TARGET_DIR=target-msvc cargo +stable-x86_64-pc-windows-msvc xtask quality`, which covers formatting, workspace checks, strict Clippy, tests, rustdoc, and documentation checks. The independent read-only review and the full Windows lifecycle checklist are deferred to the UI/UX-complete stabilization gate; the latter remains explicitly unobserved.

## Phase 3 handover

Phase 3 is complete at `58a016f` (`feat(focus): [OM-321] notify through tray`). The completed slice includes privacy-gated stabilized Windows title spans; category, icon, and exclusion flows; the durable focus lifecycle with visible and native tray completion delivery; and the authoritative schedule service, projections, Timeline editor, scoped recurrence editing/deletion, and explicit overlap reconciliation.

Verification at this tip passed with the MSVC toolchain:

- `cargo +stable-x86_64-pc-windows-msvc test --workspace`
- `cargo +stable-x86_64-pc-windows-msvc clippy --workspace --all-targets -- -D warnings`

The real-machine Windows lifecycle checklist remains deferred to the UI/UX-complete stabilization gate as the implementation plan permits. Title collection defaults to disabled until a persisted setting explicitly enables it; user-facing persisted notification and sound preferences are owned by OM-641, not Phase 3.

## Next phase

Proceed with Phase 4 in the canonical order: OM-400 through OM-402 for Overview, and OM-410 through OM-412 for Calendar. Start from this branch tip and preserve the ledger's one-writer/disjoint-path rule.

## Workspace hygiene

Do not stage generated `.agents/` worktrees. Any unrelated user change should be preserved unless explicitly requested otherwise.
