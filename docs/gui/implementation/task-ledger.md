# OpenManic MVP implementation ledger

- Integration branch: `codex/openmanic-mvp-implementation`
- Plan commit: `9002482`
- Integration authority: primary agent only
- Last updated: 2026-07-19

This ledger is the source of truth for delegated implementation ownership and integration decisions. A task may have only one writing agent, and concurrently active tasks must have separate worktrees and disjoint writable paths.

## Active and queued work

| Task | Branch | Worktree | Base SHA | Writable paths | Shared contracts | Status |
| --- | --- | --- | --- | --- | --- | --- |
| OM-040 | `codex/om040-native-ui-fixture` | `F:\\claude\\projects\\OpenManic\\.agents\\om040-native-ui-fixture` | `df085bd32f402a2e3eade28e0ff487e79a57b8d8` | `Cargo.toml`, `Cargo.lock`, `tools/native-ui-fixture/**`, `fixtures/performance/native-ui/**` | Root manifest and lockfile transfer released after integration `ab1b40b6e80767538d14a04cbd70f54a73ecfe39`; scoped repairs integrated through `cfbeaf2650443158a1b6de9c2e3483368b817b7d` | Implemented and Phase 0 code-quality verified |
| OM-050 | `codex/om050-ui-direction-spike` | `F:\\claude\\projects\\OpenManic\\.agents\\om050-ui-direction-spike` | `719497c6209b33a4c146467567b4fc2463a7938e` | `Cargo.lock`, `tools/ui-direction-spike/**`, `fixtures/ui-direction/**` | Lockfile transfer released after integration `c213601f2649eb86e8b4704a89c7367844d517f8`; scoped repairs integrated through `9361d818be8febe4a3723c6f95d55fa015849482` | Implemented and Phase 0 code-quality verified |
| OM-100 | `codex/om100-domain-foundation` | `F:\\claude\\projects\\OpenManic\\.agents\\om100-domain-foundation` | `fe97155f0b9a5dbb7607f7f255f9f71d43d2681b` | `crates/openmanic-domain/**` | Domain crate transfer released after integration `2f3acd89f14b57c656207d9f59cf71bb644d32a7` | Implemented and focused-verified |
| OM-110 | `codex/om110-focus-domain` | `F:\\claude\\projects\\OpenManic\\.agents\\om110-focus-domain` | `deda5083905e4f5d22d84721ca0baf56eceb5b6f` | `crates/openmanic-domain/src/focus.rs` | Module transfer released after integration `94a9a8e` | Implemented and focused-verified |
| OM-120 | `codex/om120-schedule-domain` | `F:\\claude\\projects\\OpenManic\\.agents\\om120-schedule-domain` | `deda5083905e4f5d22d84721ca0baf56eceb5b6f` | `crates/openmanic-domain/src/schedule.rs` | Module transfer released after integration `ec9efe6` | Implemented and focused-verified |
| OM-130 | `codex/om130-document-domain` | `F:\\claude\\projects\\OpenManic\\.agents\\om130-document-domain` | `deda5083905e4f5d22d84721ca0baf56eceb5b6f` | `crates/openmanic-domain/src/documents.rs` | Module transfer released after integration `96e5295` | Implemented and focused-verified |
| OM-140 | `codex/om140-application-contracts` | `F:\\claude\\projects\\OpenManic\\.agents\\om140-application-contracts` | `4188184925a9006a846defb9867ed4eed44cee89` | `crates/openmanic-application/**` | Application crate transfer released after integration `96105c0688d2a0047344ae5e682b1b4b7771f2e7` | Implemented and focused-verified |
| OM-150 | `codex/om150-sqlite-schema` | `F:\\claude\\projects\\OpenManic\\.agents\\om150-sqlite-schema` | _recorded at activation commit_ | `Cargo.toml`, `Cargo.lock`, `crates/openmanic-storage-sqlite/**` | Storage crate, root dependency declaration/lockfile, and initial migration exclusively transferred; no application/domain/UI/platform contract or specification change | Active; serialized before OM-151 |

## Completed and integrated work

| Task | Author head | Verifier verdict | Primary decision | Integration SHA | Remaining risk |
| --- | --- | --- | --- | --- | --- |
| OM-010 | `c33ce97085f2b3b44953500bca7dd3f3016f74c1` | PASS; no P0-P3 findings | Accepted after primary checks and Windows newline repair | `2ad86099948a98dbead117f420ec9e04056935c7` | `cargo-deny` execution begins in OM-020; no product behavior exists yet |
| OM-020 | `554352106120cb8cd520ce9b5c38b269df15e3b6` | PASS; no P0-P3 findings; quality, 10 xtask tests, 13-document check, and missing-tool diagnostic reproduced | Accepted after complete diff review, independent verification, both Windows renderer checks, and integration `cargo xtask quality` | `b3845aadd430e6543e34e265bc6b9131d35d98fa` | Real Windows lifecycle and portable-artifact smoke evidence remains a release-gate prerequisite; `cargo-deny 0.20.2` is intentionally installed by CI/release environments, not xtask |
| OM-030 | `492bdcdbd31483dd3b70a98c53a79f5a5be3ea3f` | Initial FAIL on one-slot snapshot semantics; focused repair reverified PASS with no remaining findings | Accepted after complete milestone diffs, 25 fixture tests, exact 11-file generation, full workspace quality, and verifier repair pass | `d3a9d748564b54d31433033f3aaba54975773262` | Fixtures are synthetic evidence inputs, not measured performance results; reference-hardware measurements begin in OM-040 |

## Provisional OM-060 / G0 record

- Status: candidate implementation input; independent G0 verification is recorded below. This is not
  an accepted renderer, performance budget, visual direction, or release claim.
- Trace source: `docs/gui/spec/implementation-plan.md` Section 11.1. Its 29 AC rows and nine
  detailed-product-flow rows remain the canonical requirement-to-task mapping; this record assigns
  no new owner and changes no requirement.
- OM-040 evidence: diagnostic harness and procedure only. Named hardware, real renderer/driver
  comparison, memory samples, and accepted p50/p95 data are still required before any performance
  or renderer decision.
- Renderer/budgets: WGPU is the provisional default build candidate. Glow remains a separately
  buildable comparison artifact. The performance specification's 16.7 ms full-frame, 4 ms UI CPU,
  1 s warm shell, 2 s cold shell, 100 ms tray restore, and 10,000-interval targets are provisional
  implementation inputs until a named-hardware manifest records results.
- UI direction: OM-050 keeps five primary destinations with Timeline as Today's central flow. Its
  labeled stacked distribution bar is the provisional recommendation; the ring remains selectable.
  The final navigation treatment, visual tokens, density, schedule-editor details, reordering
  affordance, and real DPI behavior remain open review decisions.
- Contract boundaries: domain state/cause vocabulary, command/event/snapshot contracts, migrations,
  recurrence rules, and theme schema stay owned by their Phase 1+ tasks. Neither spike establishes
  a production public type or persistence format.

### Scoped Phase 0 verification

- Verified integration head: `9361d818be8febe4a3723c6f95d55fa015849482`.
- `cargo xtask quality` passed with the locked offline dependency cache: formatting, workspace
  check, strict Clippy, all workspace tests, rustdoc warnings-as-errors, and documentation checks.
- Both WGPU and Glow feature selections compiled independently for `native-ui-fixture` and
  `ui-direction-spike`.
- An independent read-only verifier reported PASS for repairs `288309e..9361d81`, with no P0/P1
  findings. The prior Powered Off inference and non-retained Settings controls are corrected.
- User-scoped deferrals: named-hardware renderer comparison, memory sampling, accepted p50/p95
  budgets, final renderer selection, native review captures, real DPI observation, and final visual
  direction. They remain documented diagnostic/provisional evidence, not release or product claims.

## Ownership rules

- The primary creates every branch and worktree from an exact integration SHA.
- One writing agent owns one task and one explicit path allowlist.
- No two active writing tasks may own the same file or directory.
- Root manifests, lockfiles, public contracts, migrations, dependency policy, packaging, and specifications are primary-owned unless a task explicitly transfers them.
- Authors do not merge, rebase, push, change branches, or edit outside their allowlist.
- High-risk changes receive an independent read-only verifier before primary integration.
- The primary records author evidence, verifier findings, and the final integration decision here.

## Task record template

```text
Task ID:
Objective:
Requirement/spec references:
Branch/worktree:
Base SHA:
Writable paths:
Read-only dependencies:
Public contracts or migrations touched:
Author head and evidence:
Verifier findings/verdict:
Primary decision:
Integration SHA:
Remaining risk or waiver:
```
