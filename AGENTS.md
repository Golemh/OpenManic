# OpenManic agent instructions

These instructions apply to every delegated implementation, repair, and verification agent.

## Worktree and ownership safety

- Before reading broadly or editing, run `git rev-parse --show-toplevel`, `git branch --show-current`, `git rev-parse HEAD`, and `git status --short`.
- The repository root, branch, and HEAD must exactly match the task brief. If any differs, stop without editing and notify the primary agent.
- Never edit the primary integration checkout from a delegated task. Use only the absolute task worktree in the brief for every file and command.
- Edit only the task's explicit writable-path allowlist. All other paths are read-only, including nearby files that appear easy to fix.
- Two active writing tasks must have disjoint writable paths and separate worktrees. If work overlaps, the tasks are serialized or a primary-owned prerequisite is integrated first.
- Existing changes are not yours to clean, reset, reformat, stage, or delete.

## Git authority

- The primary agent creates task branches and worktrees and performs all integration.
- Delegated agents do not create, switch, merge, rebase, push, force-push, clean, or reset branches.
- Stage only assigned paths with `git add -- <exact paths>`; never use `git add .` or `git add -A`.
- Do not amend another task's commit.

## Implementation standard

- Follow `docs/gui/spec/implementation-plan.md` and the task's named canonical specification sections.
- Preserve the crate dependency directions and frontend/backend boundary in `docs/gui/spec/project-structure.md`.
- Follow all lint, documentation, testing, unsafe, error, and readability rules in `docs/gui/spec/code-quality-standards.md`.
- Do not invent product behavior or a public contract. Escalate missing decisions to the primary agent.
- Do not add a dependency, feature, migration, or shared public type unless the task explicitly transfers that ownership.
- Use deterministic tests without network access, arbitrary sleeps, or uncontrolled wall time.

## Handoff

- Review the complete base-to-head diff and confirm every changed path is owned.
- Run the exact checks named in the task brief and report commands, exit codes, and important cases.
- Return the required evidence manifest with base/head SHAs, decisions, limitations, and a clean status.
- Verifiers begin read-only, report findings with exact locations, and never fix the author's branch.
