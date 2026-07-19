# Agent execution and context strategy

This file is the single operational policy for keeping GenAI-assisted implementation focused.
Product requirements, architecture, ownership, and acceptance criteria remain authoritative in
their canonical specifications; this file governs how agents consume that information.

## Working model

- Assign one primary owner to a cross-layer vertical slice. Split work only when writable paths,
  contracts, and evidence are genuinely independent.
- Start each phase with a short checklist: required contracts, focused automated checks, one
  frozen quality gate, review, and any human evidence. Record only the unresolved items in the
  handover or ledger.
- Search named symbols and focused diffs first. Read complete modules only when a direct boundary
  cannot otherwise be understood. Do not repeat broad reads merely to recreate context.
- While editing, run the narrowest deterministic check that can disprove the current change.
  Run `cargo xtask quality` once after the batch freezes; rerun only after a repair changes code,
  manifests, or checked documentation.
- Keep handovers concise: current phase, integration SHA, changed boundary, passing gate, and
  explicit deferred risk. Do not recreate worktrees, manifests, or status reports per plan row.
- Manual/platform evidence is separate from automated evidence. A deferred human checklist must
  state its trigger and must never be represented as observed or accepted before it is run.

## Current manual-validation policy

The Windows lifecycle checklist is deferred until the UI/UX is complete. Before release, a human
must verify visible, minimized, hidden-to-tray, resized, and stalled tracking; pause/resume;
restart persistence; live and historical Timeline plus both summary widgets; tray and
single-instance behavior; and coordinated explicit Quit on Windows 11.
