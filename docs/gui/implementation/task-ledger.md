# OpenManic MVP implementation ledger

- Integration branch: `codex/openmanic-mvp-implementation-continuation`
- Plan commit: `9002482`
- Integration authority: primary agent only
- Last updated: 2026-07-20

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
| OM-150 | `codex/om150-sqlite-schema-v2` | `F:\\claude\\projects\\OpenManic\\.agents\\om150-sqlite-schema-v2` | `b4eb766330b3fa97190e6c2ddcbdc926249454e2` | `crates/openmanic-storage-sqlite/**` | Root `rusqlite` dependency/lockfile was integrated separately; `0001` and the storage crate were serialized. The primary also approved direct use of already-locked `thiserror = 1.0.69`; no new package or transitive footprint | Integrated and focused-verified; OM-151 owns online pre-migration backup, restore, and post-migration integrity checks |
| OM-151 | `codex/om151-migration-safety` | `F:\\claude\\projects\\OpenManic\\.agents\\om151-migration-safety` | `f7d3a9b6b00c665fc0091cd91f8f71da94437ac7` | `crates/openmanic-storage-sqlite/src/{backup.rs,connection.rs,errors.rs,lib.rs,migration.rs}` | No schema or migration source change. The primary enabled the existing pinned `rusqlite` backup API feature before delegation | Integrated and Phase 1 verified; every later post-`0001` migration must use this crate-private guard |
| OM-200 | `codex/om200-runtime-primitives` | `F:\\claude\\projects\\OpenManic\\.agents\\om200-runtime-primitives` | `450fe2799452cd63ff7d250da56a9eda85961303` | `crates/openmanic-application/src/{lib.rs,runtime/{mod.rs,lanes.rs,mailbox.rs,cancellation.rs,health.rs,supervisor.rs,shutdown.rs}}` | The primary added the pinned bounded-channel dependency before delegation; this task exclusively owned its application runtime facade | Integrated at `d9baa03`; Phase 2 verification deferred to the one phase gate |
| OM-270 | `codex/om270-ui-shell` | `F:\\claude\\projects\\OpenManic\\.agents\\om270-ui-shell` | `450fe2799452cd63ff7d250da56a9eda85961303` | `crates/openmanic-ui-egui/src/{lib.rs,app.rs,controller.rs,model.rs,reducer.rs,repaint.rs,shell.rs}` | No storage/platform dependency or application-contract change is transferred | Integrated at `bb2efff`; Phase 2 verification deferred to the one phase gate |
| OM-210 | `codex/om210-tracking-service` | `F:\\claude\\projects\\OpenManic\\.agents\\om210-tracking-service` | `d9baa03` | `crates/openmanic-application/src/{lib.rs,commands.rs,events.rs,ports.rs,tracking.rs}` | This was the serialized primary allocation for typed tracking command/event/persistence-port extensions; no runtime, storage, domain, UI, or platform path overlapped | Integrated at `8838fab`; Phase 2 verification deferred to the one phase gate |
| OM-295 | `codex/om295-bootstrap-data-root` | `F:\\claude\\projects\\OpenManic\\.agents\\om295-bootstrap-data-root` | `bb2efff` | `crates/openmanic/{Cargo.toml,src/{main.rs,lib.rs,bootstrap.rs,cli.rs,data_root.rs,diagnostics.rs}}` | No root manifest, application contract, storage, or platform path was transferred | Integrated at `58a73f3`; Phase 2 verification deferred to the one phase gate |
| OM-220 | `codex/om220-storage-repositories` | `F:\\claude\\projects\\OpenManic\\.agents\\om220-storage-repositories` | `58a73f3` | `crates/openmanic-storage-sqlite/src/{connection.rs,errors.rs,lib.rs,repository.rs,writer.rs}` | The primary explicitly approved the storage crate's direct domain dependency and its expected lockfile edge; no migration, application, root manifest, UI, or platform path was transferred | Integrated at `be641cc`; Phase 2 verification deferred to the one phase gate |
| OM-230 | `codex/om230-platform-normalization` | `F:\\claude\\projects\\OpenManic\\.agents\\om230-platform-normalization` | `58a73f3` | `crates/openmanic-platform/src/{lib.rs,adapter.rs,capabilities.rs,fake.rs}` | Consumes the accepted typed tracking evidence; the primary supplied the application facade re-export at `7e65b15`; no Windows FFI path was transferred | Integrated at `5658d11`; Phase 2 verification deferred to the one phase gate |
| OM-280 | `codex/om280-timeline-projection` | `F:\\claude\\projects\\OpenManic\\.agents\\om280-timeline-projection` | `be641cc` | `crates/openmanic-application/src/{lib.rs,projection.rs,timeline_projection.rs}` | Serialized application-projection allocation after OM-220; the projection requires a future bounded storage adapter to provide stable raw identities rather than fabricating them | Integrated at `02b80c5`; Phase 2 verification deferred to the one phase gate |
| OM-281 | `codex/om281-timeline-geometry` | `F:\\claude\\projects\\OpenManic\\.agents\\om281-timeline-geometry` | `02b80c5` | `crates/openmanic-ui-egui/src/{lib.rs,timeline/{mod.rs,geometry.rs,hit_test.rs,ticks.rs}}` | Serialized UI timeline-geometry allocation after OM-280; generic exact brackets deliberately await a schedule-occurrence identity contract | Integrated at `47c3144`; Phase 2 verification deferred to the one phase gate |
| OM-282 | `codex/om282-timeline-kernel` | `F:\\claude\\projects\\OpenManic\\.agents\\om282-timeline-kernel` | `47c3144` | `crates/openmanic-ui-egui/src/timeline/{mod.rs,paint.rs,interaction.rs}` | Serialized extension of OM-281's timeline module; raw identity remains resolved by OM-281 hit tests and schedule IDs remain future projection data | Integrated at `08ff277`; Phase 2 verification deferred to the one phase gate |
| OM-290 | `codex/om290-today-controller` | `F:\\claude\\projects\\OpenManic\\.agents\\om290-today-controller` | `1df90e4` | `crates/openmanic-ui-egui/src/{lib.rs,controller.rs,model.rs,reducer.rs,today.rs}` | Serialized Today-state/controller allocation after OM-270 and OM-280; uses the existing day-offset model until civil-time conversion is application-owned | Integrated at `b18f881`; Phase 2 verification deferred to the one phase gate |
| OM-291 | `codex/om291-timeline-renderer` | `F:\\claude\\projects\\OpenManic\\.agents\\om291-timeline-renderer` | `aa4a1cc` | `crates/openmanic-ui-egui/src/timeline/{mod.rs,renderer.rs,detail.rs}` | Recreated from the compacted clean task branch at the latest integrated head; uses stable IDs when snapshot display names/occurrences are unavailable | Integrated at `80b7e7d`; Phase 2 verification deferred to the one phase gate |
| OM-292 | `codex/om292-usage-widget` | `F:\\claude\\projects\\OpenManic\\.agents\\om292-usage-widget` | `aa4a1cc` | `crates/openmanic-ui-egui/src/usage.rs` | Recreated from the compacted clean task branch at the latest integrated head; composition must supply the exact already-formatted range label | Integrated at `7bdf299`; Phase 2 verification deferred to the one phase gate |
| OM-293 | `codex/om293-distribution-widget` | `F:\\claude\\projects\\OpenManic\\.agents\\om293-distribution-widget` | `2e0bd4d` | `crates/openmanic-ui-egui/src/distribution.rs` | The primary predeclared the private module; composition must provide stable, already-filtered contribution inputs | Integrated at `3120f0a`; Phase 2 verification deferred to the one phase gate |
| OM-299 | primary integration branch | `D:\\y-Coding\\Human_Coding\\Codex_Hackathon\\OpenManic` | `f906d4a` | `crates/openmanic/{Cargo.toml,src/{lib.rs,main.rs,composition.rs}}, crates/openmanic-platform/src/{windows_control.rs,windows_identity.rs,windows_single_instance.rs,windows_tray.rs}, crates/openmanic-storage-sqlite/src/writer.rs` | Primary-owned end-to-end vertical composition and G2 quality repairs. Resolved Windows identity now maps to a deterministic local catalog ID, upserts on the writer before foreground evidence, and preserves observed bounds; MSVC `cargo xtask quality` passes. | Phase 2 implementation accepted; independent review and human Windows lifecycle validation deferred to the UI/UX-complete stabilization gate |
| OM-310 | primary integration branch | `D:\\y-Coding\\Human_Coding\\Codex_Hackathon\\OpenManic` | `f906d4a` | `crates/openmanic-application/src/{catalog.rs,lib.rs}`, `crates/openmanic-storage-sqlite/src/{errors.rs,writer.rs}` | Primary-owned Phase 3 catalog service: explicit create/rename/delete/bulk-assignment commands, correlated mutation outcomes, a persistence port, atomic SQLite writer mutations, and immutable revision-correlated name/category/Uncategorized query snapshots. The existing projection reads catalog associations at the shared data revision. | Implemented and focused-verified; UI command dispatch and destructive confirmation are consumed by dependent OM-311 |
| OM-311 | primary integration branch | `D:\\y-Coding\\Human_Coding\\Codex_Hackathon\\OpenManic` | `4158bda` | `crates/openmanic/src/composition.rs` | Primary-owned Categories route now consumes one revision-correlated catalog snapshot and routes create, rename, confirmed delete, individual and selected-application assignment/removal, and individual/bulk exclusion policy mutations through the bounded catalog lane. It retains selection through category creation and provides search plus category/Uncategorized/excluded filters. | In progress. OM-312 icon result wiring and Timeline-detail entry-point routing remain; shared shell supplies the loading/error/recovery presentation. |
| OM-312 | primary integration branch | `D:\\y-Coding\\Human_Coding\\Codex_Hackathon\\OpenManic` | `e877e01` | `crates/openmanic-application/src/{application_metadata.rs,lib.rs}` | Primary-owned Phase 3 metadata-cache foundation: rebuildable decoded RGBA icon cache, fixed entry/byte limits, deterministic fallback, LRU eviction, and privacy-safe counters. | In progress at `358b0fa`; Windows extraction and background-result wiring remain before OM-311 consumes icon results. |
| OM-313 | primary integration branch | `D:\\y-Coding\\Human_Coding\\Codex_Hackathon\\OpenManic` | `9603dff` | `crates/openmanic/src/composition.rs`, `crates/openmanic-application/src/catalog.rs`, `crates/openmanic-storage-sqlite/src/{repository.rs,writer.rs}` | Primary-owned Phase 3 exclusion-policy slice: atomic catalog policy mutation, foreground conversion to minimum Excluded evidence, and revision-correlated policy visibility in catalog snapshots. | In progress at `e877e01`; direct Categories UI action and immediate active-interval reconciliation remain before OM-311. |
| OM-320 | primary integration branch | `D:\\y-Coding\\Human_Coding\\Codex_Hackathon\\OpenManic` | `3fd9c36` | `crates/openmanic-domain/src/focus.rs`, `crates/openmanic-application/src/{focus.rs,lib.rs}`, `crates/openmanic-storage-sqlite/src/writer.rs` | Primary-owned focus lifecycle boundary: validated restore, immutable snapshots, explicit draft/start/pause/resume/complete/cancel commands, typed persistence/notification ports, atomic SQLite persistence, optimistic entity revisions, and restart reconciliation. No schema, migration, platform, runtime, or UI path changed. | Implemented and focused-verified; Focus UI/tray controls and platform notification adapter remain OM-321-owned |
| OM-321 | primary integration branch | `D:\\y-Coding\\Human_Coding\\Codex_Hackathon\\OpenManic` | `18d0451` | `crates/openmanic/src/composition.rs` | Primary-owned focus UI/runtime slice: a shared immutable focus snapshot is reconciled on the writer independently of egui repaint; the Today controls render only valid lifecycle actions and an absolute-deadline or frozen paused countdown. | In progress. Native completion notification/sound settings, tray completion delivery, and timeline overlay remain. |
| OM-330 | primary integration branch | `D:\\y-Coding\\Human_Coding\\Codex_Hackathon\\OpenManic` | `9d5454a` | `crates/openmanic-domain/src/schedule.rs`, `crates/openmanic-application/src/{schedule.rs,lib.rs}`, `crates/openmanic-storage-sqlite/src/{repository.rs,writer.rs}`, `crates/openmanic/src/composition.rs` | Primary-owned schedule authority now routes explicit only-this, this-and-future, and every-occurrence edit/delete commands through authoritative snapshots and revision-checked persistence. | In progress. Structured conflicting-occurrence results and full mutation-result reconciliation remain. |
| OM-331 | primary integration branch | `D:\\y-Coding\\Human_Coding\\Codex_Hackathon\\OpenManic` | `c6dc95f` | `crates/openmanic-application/src/{schedule_projection.rs,schedule_time.rs,timeline_projection.rs,lib.rs}`, `crates/openmanic-storage-sqlite/src/{repository.rs,writer.rs,lib.rs}`, `crates/openmanic-ui-egui/src/timeline/{paint.rs,renderer.rs}`, `crates/openmanic/src/composition.rs` | Primary-owned schedule occurrence projection: correlated reads now retain schedules, bounded IANA-aware expansion creates immutable occurrence IDs/ranges/adjustment markers, and Timeline snapshots render exact shared-transform brackets without changing activity facts. | Implemented and focused-verified. OM-332 can consume these occurrence IDs for the shared editor; OM-330's structured conflict and scope reconciliation remain independently open. |
| OM-332 | primary integration branch | `D:\\y-Coding\\Human_Coding\\Codex_Hackathon\\OpenManic` | `ec40557` | `crates/openmanic/src/composition.rs` | Primary-owned shared Timeline schedule UI creates schedules from exact drag ranges and lists visible correlated occurrences for confirmed deletion; recurring deletion exposes only-this, this-and-future, and every-occurrence scopes. | In progress. Existing-schedule edit controls and shared dialog parity remain. |
| OM-240 | `codex/om240-windows-control-loop` | `F:\\claude\\projects\\OpenManic\\.agents\\om240-windows-control-loop` | `e2a4ca6` | `crates/openmanic-platform/src/{lib.rs,windows_control.rs,windows_raw.rs}` | The primary prepared pinned Windows bindings and the lockfile at `e2a4ca6`; live HWND attribution honestly remains degraded until OM-250 | Integrated at `8e3e49d`; Phase 2 verification deferred to the one phase gate |
| OM-250 | `codex/om250-windows-identity` | `F:\\claude\\projects\\OpenManic\\.agents\\om250-windows-identity` | `f9457f9` | `crates/openmanic-platform/src/windows_identity.rs` | The primary enabled the Appx/Globalization namespaces before author checks; control-loop composition remains OM-299-owned | Integrated at `8e9dd90`; Phase 2 verification deferred to the one phase gate |
| OM-260 | `codex/om260-windows-lifecycle` | `F:\\claude\\projects\\OpenManic\\.agents\\om260-windows-lifecycle` | `f9457f9` | `crates/openmanic-platform/src/windows_lifecycle.rs` | The primary enabled the Performance/WindowsProgramming namespaces before final author checks; control-loop composition remains OM-299-owned | Integrated at `1d04a7a`; Phase 2 verification deferred to the one phase gate |
| OM-296 | `codex/om296-windows-tray-instance` | `F:\\claude\\projects\\OpenManic\\.agents\\om296-windows-tray-instance` | `1df90e4` | `crates/openmanic-platform/src/{lib.rs,windows_control.rs,windows_tray.rs,windows_single_instance.rs}` | The primary enabled tray/pipe/security/IO/FileSystem namespaces; data-root lock remains the accepted OM-295 bootstrap boundary and final composition is OM-299-owned | Integrated at `2e0bd4d`; Phase 2 verification deferred to the one phase gate |
| OM-400 | `codex/om400-overview-projection` | `F:\\claude\\projects\\OpenManic\\.agents\\om400-overview-projection` | `f980c5ffb64dc7c90c028b3612b60871d60628c0` | `crates/openmanic-application/src/overview.rs` | The primary owns `lib.rs` and all public re-exports. Pure cancellable model now covers normalized range/filter/selection contexts, revision cache keys, deterministic aggregates, and stale/cancelled result retention; saved-view persistence remains OM-401. The author worktree ACL denied target/index writes, so the primary transferred, formatted, tested, and committed the single owned file. | Integrated at `1b065d5`; focused tests and strict application Clippy pass |
| OM-410 | `codex/om410-calendar-projection` | `F:\\claude\\projects\\OpenManic\\.agents\\om410-calendar-projection` | `f980c5ffb64dc7c90c028b3612b60871d60628c0` | `crates/openmanic-application/src/calendar_projection.rs` | The primary owns `lib.rs`, correlated storage read records, and composition wiring. The pure projection preserves activity/focus/schedule block identity, clipping, overlap, and Timeline navigation without time-zone calculation or a second schedule model. | Integrated at `6accca0`, facade at `0f73ea4`; focused tests and strict application Clippy pass |
| OM-401 | primary integration branch | `F:\\claude\\projects\\OpenManic` | `0f73ea4` | `crates/openmanic-domain/src/documents.rs`, `crates/openmanic-application/src/saved_view.rs`, `crates/openmanic-storage-sqlite/src/{repository.rs,writer.rs}` | Versioned full saved-view documents persist through the sole service and SQLite adapter, retaining deterministic ordering, revision checks, and invalid-document isolation. | Integrated at `59641d3` and `23243ac`; storage round-trip/revision test and G4 quality pass |
| OM-402 | primary integration branch | `F:\\claude\\projects\\OpenManic` | `23243ac` | `crates/openmanic-ui-egui/src/overview.rs` | Pure Overview presentation consumes immutable snapshots, exact totals/ranges, and typed saved-view effects without storage access. | Integrated at `c04104c`; reducer/presentation tests and G4 quality pass |
| OM-411 | primary integration branch | `F:\\claude\\projects\\OpenManic` | `0f73ea4` | `crates/openmanic-ui-egui/src/calendar.rs`, `crates/openmanic/src/composition.rs` | Calendar renders immutable activity/focus/schedule blocks with date controls, dense-period selection, exact details, and activity navigation through the shared Timeline context. | Integrated at `8d30834` and durable composition seam `e3c562d`; focused UI tests and G4 quality pass |
| OM-412 | primary integration branch | `F:\\claude\\projects\\OpenManic` | `e3c562d` | `crates/openmanic/src/composition.rs` | Calendar reuses Timeline's schedule snapshots, drafts, edit/delete scopes, service commands, and occurrence identities; it owns no second editor or persistence path. | Integrated at `5b2b1e0`; shared Calendar-to-Timeline occurrence-contract test and G4 quality pass |
| OM-500 | primary integration branch | `F:\\claude\\projects\\OpenManic` | `fe43ce8` | `crates/openmanic-ui-egui/src/{today.rs,lib.rs}` | Phase 5 batch started: replace the fixed Today bootstrap with a versioned first-party widget registry before layout persistence/editing. | In progress; broad formatting/linting deferred to G5 |

## Completed and integrated work

| Task | Author head | Verifier verdict | Primary decision | Integration SHA | Remaining risk |
| --- | --- | --- | --- | --- | --- |
| OM-010 | `c33ce97085f2b3b44953500bca7dd3f3016f74c1` | PASS; no P0-P3 findings | Accepted after primary checks and Windows newline repair | `2ad86099948a98dbead117f420ec9e04056935c7` | `cargo-deny` execution begins in OM-020; no product behavior exists yet |
| OM-020 | `554352106120cb8cd520ce9b5c38b269df15e3b6` | PASS; no P0-P3 findings; quality, 10 xtask tests, 13-document check, and missing-tool diagnostic reproduced | Accepted after complete diff review, independent verification, both Windows renderer checks, and integration `cargo xtask quality` | `b3845aadd430e6543e34e265bc6b9131d35d98fa` | Real Windows lifecycle and portable-artifact smoke evidence remains a release-gate prerequisite; `cargo-deny 0.20.2` is intentionally installed by CI/release environments, not xtask |
| OM-030 | `492bdcdbd31483dd3b70a98c53a79f5a5be3ea3f` | Initial FAIL on one-slot snapshot semantics; focused repair reverified PASS with no remaining findings | Accepted after complete milestone diffs, 25 fixture tests, exact 11-file generation, full workspace quality, and verifier repair pass | `d3a9d748564b54d31433033f3aaba54975773262` | Fixtures are synthetic evidence inputs, not measured performance results; reference-hardware measurements begin in OM-040 |
| OM-150 | `05fd758b613179ca02e23c631be6678f72797ddd`, repaired by `133714c2bd3598cd429b9914f6e32fb9d1562026` and `56be818b0e35cec181dce3ec6569501c4e948e68` | Initial FAIL: P1 focus-state schema mismatch. Repair PASS with no P0/P1; typed-error dependency repair PASS with no P0-P3 | Accepted after serialized schema repair, two independent verifier passes, primary diff/ownership review, and final offline format, 8-test, and strict Clippy checks | `68ecd784e79b1030abb63cfdca70f2b59d0e17a1` | OM-151 must provide pre-migration online backup, retained recovery/restore, and `quick_check`/`foreign_key_check`; repositories and the serialized writer service are OM-220 |
| OM-151 | `27d0737d29e9b9e48582a49236f4e8164ec4baa8`, repaired by `df28da420644cedb0ac05dd2a073c15c30ab7d5d` | Consolidated G1 review found no P0/P1. Verifier-worktree quality run was ACL-blocked before compilation; the identical primary-checkout gate passed | Accepted after the retained-online-backup, restore writer-configuration, and post-migration integrity repairs, then the one Phase 1 gate | `83b7334035066b3f0d9ef9f58603eedbe2efe244` | User-directed backup discovery/restore UI is later work; OM-220 owns repositories and the serialized writer service |

### Consolidated Phase 4 verification

- G4 passed from the primary checkout on 2026-07-20 using
  `$env:CARGO_TARGET_DIR='target-msvc'; cargo +stable-x86_64-pc-windows-msvc xtask quality`.
  The gate passed formatting, workspace checks, strict Clippy, all workspace tests, rustdoc with
  warnings denied, and `docs-check`.
- Evidence covers normalized/cancellable Overview projections, durable saved-view restoration,
  Calendar source distinction and activity-to-Timeline navigation, and the shared
  Calendar-to-Timeline schedule occurrence command contract.
- Deferred product/time-zone limitations are explicitly retained in
  `phase-4-edge-cases.md`; none are represented as completed behavior.

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

### Consolidated Phase 1 verification

- Verified integration head: `83b7334035066b3f0d9ef9f58603eedbe2efe244`.
- The one consolidated G1 verifier found no P0/P1: domain and application boundaries remain
  adapter-free, `0001` covers the required MVP entities and constraints, and OM-151 has no schema
  change while enforcing validation-before-backup, retained online backup/recovery, restored writer
  configuration, and integrity checks before migration commit.
- `cargo xtask quality` passed from the primary checkout: formatting, workspace check, strict
  Clippy, all workspace tests, rustdoc warnings-as-errors, and documentation checks. The verifier
  worktree's attempt was blocked before compilation by its ACL-protected Cargo build-lock file;
  no code check failed.
- G1 is accepted. Phase 2 may begin from the post-ledger integration head; Phase 2 preparation
  remains dependency-aware and does not waive later Windows-specific verification.

## Ownership rules

- The primary creates every branch and worktree from an exact integration SHA.
- One writing agent owns one task and one explicit path allowlist.
- No two active writing tasks may own the same file or directory.
- Root manifests, lockfiles, public contracts, migrations, dependency policy, packaging, and specifications are primary-owned unless a task explicitly transfers them.
- Authors do not merge, rebase, push, change branches, or edit outside their allowlist.
- The normal cadence is batch-level: authors run only focused unblock checks while implementing; one consolidated quality run and targeted read-only review occur at the applicable phase/gate. Escalate to an earlier review only for a concrete failure, conflict, or newly discovered high-risk boundary.
- Follow the shared [agent execution and context strategy](agent-execution-strategy.md) for batching, context use, quality cadence, and deferred human evidence.
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
Acceptance checklist (contracts / automated / review / manual):
Author head and evidence:
Verifier findings/verdict:
Primary decision:
Integration SHA:
Remaining risk or waiver:
```

## OM-500 through OM-520 — Phase 5 customization foundation

- **Objective:** establish persisted dashboard-widget layout, explicit layout editing, deterministic
  responsive reflow, and built-in appearance selection.
- **Requirement/spec references:** implementation plan §17; UI implementation §§4, 5, 9, and 13.
- **Branch/worktree:** `codex/openmanic-mvp-implementation-continuation`, primary integration checkout.
- **Base SHA:** `fe43ce8365d129a2741f6aa449f9f8fd5476df67`.
- **Owned paths:** application layout boundary; SQLite layout/theme persistence; domain layout document;
  egui registry, editor, reflow, theme, shell; composition; Phase 5 documentation.
- **Evidence:** `cargo +stable-x86_64-pc-windows-msvc xtask quality` passed with
  `CARGO_TARGET_DIR=target-msvc` on 2026-07-20. This includes formatting, workspace check,
  Clippy with warnings denied, full workspace tests, rustdoc warnings denied, and docs validation.
- **Remaining risk / waiver:** live operating-system theme observation and narrow-window touch
  affordances are explicitly deferred in `phase-5-edge-cases.md`; no external theme documents are
  accepted in this MVP phase.

## OM-600 through OM-643 — Phase 6 data operations and delivery

- **Objective:** deliver local data interchange and recovery operations, portable Windows delivery
  controls, onboarding/settings, job presentation, and lifecycle completion.
- **Requirement/spec references:** implementation plan §18 and the corresponding data-model,
  architecture, delivery/setup, and UI specifications.
- **Branch/worktree:** `codex/openmanic-mvp-implementation-continuation`, primary integration checkout.
- **Base SHA:** `c4b19bfa3b1c5bed04eb0525e62cacbcb9444e0d`.
- **Status:** implementation is pending the final Phase 6 gate. The Settings screen hosts CSV
  import/export with explicit title disclosure, verified backup/restore, verified data-location
  move with persisted per-user locator, privacy-safe diagnostics export, and named recoverable
  data-operation jobs. CSV imports now honor cancellation before and during merge: the transaction
  savepoint rolls back merged entities while retaining terminal cancelled job/batch metadata.
  Restore and move quiesce the writer before storage work, retain the source on move, and rebuild
  the local runtime after completion.
- **Evidence:** the consolidated MSVC gate passed after the Settings visibility and CSV-cancellation
  changes: `CARGO_TARGET_DIR=target-msvc cargo +stable-x86_64-pc-windows-msvc xtask quality`
  completed formatting, locked workspace checks, strict Clippy, the workspace test suite, rustdoc,
  and docs validation. Focused cancellation evidence confirms no staged entity is merged after
  cancellation.
- **G6 limitation:** the automated quality gate is green, but the real-Windows/manual acceptance
  evidence is incomplete. A live debug build on Windows showed the full scrollable Settings data
  surface; a verified backup completed and appeared as a named completed job; restore and move
  confirmation wording was inspected and cancelled without replacing or relocating data. Actual
  restore/move behavior, Explorer/tray recovery, autostart repair, and portable-artifact
  replacement remain unobserved before declaring G6 accepted.
