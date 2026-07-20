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

## Continuation deadlock prevention

A persistent goal or automatic continuation is not a substitute for substantive work. An agent
must not end its own turn prematurely and then describe the resulting continuation as an external
runner interruption.

- Do not end a turn after reconnaissance, planning, one failed command, or a status update when
  the next safe implementation action is known.
- A coding turn normally contains one complete work unit: focused inspection, edit, narrow
  deterministic verification, and owned-diff review. Commit only at a coherent batch boundary.
- Never send an empty handoff or one that only says "continuing", "resuming", or "checking".
- Do not attribute a self-ended turn to runner termination without evidence of an actual timeout,
  cancellation, tool failure, or external interruption.
- If a command fails, repair it or use a safe alternative in the same turn. A failed lookup,
  quoting error, or incomplete investigation is not a blocker.
- Before returning chat, answer all three questions: is a safe next edit or verification known;
  is the required authority available; and is the worktree usable? If all answers are yes,
  continue working rather than returning chat.
- Mark a goal blocked only for a repeated, externally caused impasse that prevents all meaningful
  progress. Agent cadence mistakes, incomplete investigation, or a voluntarily ended turn are
  never blockers.
- If three consecutive turns produce no file change, test result, committed evidence, or newly
  discovered external blocker, stop status-only continuation. Reassess and either make a concrete
  implementation move or request the one necessary user decision.
- At a genuine batch handoff, report completed work, exact verification evidence, remaining
  scope, and the next concrete implementation unit.

### Minimum return threshold

Return chat only for a coherent verified batch boundary, a required user decision or approval, a
phase/gate handoff, or a genuine external dependency after safe alternatives have been exhausted.
Do not return merely because a command had a quoting/path error, reconnaissance is complete, or
the next safe edit is already known. Automatic continuation must not compensate for an agent
ending its own turn prematurely.

## Current manual-validation policy

The Windows lifecycle checklist is deferred until the UI/UX is complete. Before release, a human
must verify visible, minimized, hidden-to-tray, resized, and stalled tracking; pause/resume;
restart persistence; live and historical Timeline plus both summary widgets; tray and
single-instance behavior; and coordinated explicit Quit on Windows 11.
