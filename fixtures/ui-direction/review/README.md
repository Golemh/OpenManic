# OM-050 UI-direction review record

## Purpose and scope

`tools/ui-direction-spike` is a native, low-fidelity review tool. It is deliberately separate from
the production UI crate and uses no storage, platform, application, or persisted-theme contract.
Its local `MockSnapshot` maps deterministic OM-030 fixture scenarios into display-only data, and
its local `SpikeAction` reducer simulates visible pending, confirmed, and rejected outcomes.

Run one renderer at a time:

```powershell
cargo run -p ui-direction-spike
cargo run -p ui-direction-spike --no-default-features --features renderer-glow
```

The review tool does not write fixtures, take automated screenshots, access user data, or create
product evidence. Native captures, if reviewers choose to take them, are human-review material
only and must never be presented as performance, DPI, accessibility, or acceptance evidence.

## Provisional navigation interpretation

The product requirements make Today, Overview, Categories, and Calendar primary destinations and
allow Settings as a fifth. The handover lists Timeline among five screen flows while also requiring
category coverage. This spike keeps five primary destinations—Today, Overview, Categories,
Calendar, and Settings—and presents Timeline as Today's central interactive flow. This is a local,
replaceable review interpretation; it does not settle production routing.

## Variations to review

| Variation | Options | Provisional recommendation | Why it remains replaceable |
| --- | --- | --- | --- |
| Distribution presentation | Labeled stacked bar; ring with text legend | Labeled stacked bar | The compact bar exposes allocation, exact text values, and non-color identification in both Today and Overview. |
| Navigation placement | Left/top compact navigation in the spike | No production decision | The spike proves route retention and information density only. |
| Dashboard reordering | Explicit labeled controls | No production decision | Direct drag-and-drop remains a visual/interaction review choice. |
| Scale treatment | 720, 1024, and 1440 logical-width previews | Test actual Windows scaling later | A logical preview is not a substitute for 125–200% DPI rendering evidence. |

## Review capture inventory

No screenshots are fabricated or committed by this task. During review, capture each item manually
with the renderer, operating-system scale, and width written alongside it.

| ID | Route/scene | Required observation |
| --- | --- | --- |
| R1 | Today / 720 px / Ready | Timeline stays primary; compact widgets retain their primary values and action. |
| R2 | Today / 1024 px / layout editing | Labeled reorder/resize controls, Save, Cancel/Revert, and Reset are distinct from timeline interaction. |
| R3 | Today / 1440 px / schedule mode | Three bands, selection vs hover, focus overlay, schedule brackets, and visible draft feedback. |
| R4 | Today / Loading, Refreshing, Empty, Partial, Error, Recovered | Each state is plain-language; refresh retains prior content and errors expose retry/details. |
| R5 | Overview / both distribution variants | Exact labels and totals remain visible without relying on color. |
| R6 | Categories / multiselect assignment | Search, uncategorized filter, bulk assignment, and create-during-assignment flow retain selection. |
| R7 | Calendar / schedule mode | Activity, focus, and schedule layers remain distinguishable with date navigation and Timeline handoff. |
| R8 | Settings / default plus advanced | Privacy-sensitive choices are understandable before technical details are expanded. |

## Open decisions for primary/user review

1. Approve the labeled stacked bar or select the ring/another compact distribution presentation.
2. Choose final navigation placement, component density, typography, colors, spacing, radii, and
   shadow treatment through a later semantic-theme implementation.
3. Choose whether direct drag-and-drop supplements the spike's labeled layout controls.
4. Confirm the exact schedule-editor visual flow, snapping increment, and Timeline/Calendar details
   after domain and application schedule contracts exist.
5. Validate the selected direction on real Windows 125%, 150%, 175%, and 200% scaling; this spike
   intentionally provides no such acceptance claim.

## Deferred verification

Only narrow package compilation and deterministic reducer tests belong to this spike task. Full
workspace quality, renderer/measurement evidence, the Phase 0 G0 gate, and primary acceptance
trace review remain deferred until OM-040, OM-050, and OM-060 are complete.
