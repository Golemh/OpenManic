# OpenManic implementation handover

## Resume point

- Branch: `codex/openmanic-mvp-implementation-continuation`
- Start from the branch tip, not an old task worktree.
- The implementation ledger at `docs/gui/implementation/task-ledger.md` is the ownership and integration record.
- The canonical next-work authority is `docs/gui/spec/implementation-plan.md`.

## User-directed execution cadence

Prioritize implementation throughput over per-task ceremony.

- Batch adjacent plan rows that share a crate/layer and have no unresolved dependency; keep the plan's ordering and product requirements intact.
- Use at most three implementation agents at once, normally one each for independent domain/application-storage, platform, and UI streams.
- Keep a small stable set of worktrees for a batch. Do not create a new worktree, ledger commit, verifier, or full review for every plan row.
- Run only focused compile/tests needed to unblock an author while work is in progress. Run one consolidated `cargo xtask quality` and one targeted read-only review at the end of a phase/gate.
- Windows-only/manual evidence belongs to the applicable phase gate. Do not turn it into repetitive task-level checks.
- Update the ledger at the start/end of a batch or phase, with the current integration SHA and any real limitation. Preserve its ownership record, but keep entries concise.

## Current state

Phase 0 and Phase 1 are accepted. Phase 2 component tasks OM-200 through OM-296 are integrated, including the runtime, storage, Windows adapters, timeline projection/interaction/rendering, Today controller, and summary presentation models.

OM-299 is the remaining Phase 2 task. It is primary-owned and must compose the accepted pieces into the first end-to-end Windows vertical slice. Start by wiring the existing bootstrap/data-root lock, SQLite store, bounded application runtime, Windows control/tray/single-instance adapters, immutable UI snapshots, and coordinated Quit path. Do not replace the accepted bounded, adapter-free, or immutable-snapshot boundaries with shortcuts.

After OM-299, run one consolidated Phase 2 gate. It must include the repository quality command and the plan's Windows vertical-slice evidence; do not claim real Windows lifecycle evidence that was not run.

## After Phase 2

Use the ordered Phase 3+ work packages and gates in `docs/gui/spec/implementation-plan.md` as written. Do not begin broad Phase 3 feature work until the G2 gate passes. Preserve the ledger's one-writer/disjoint-path rule and create new task worktrees from the current integration tip.

## Workspace hygiene

Do not stage generated `.agents/` worktrees. Any unrelated user change should be preserved unless explicitly requested otherwise.
