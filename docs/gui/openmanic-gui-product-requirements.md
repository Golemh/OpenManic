# OpenManic GUI Product and Functional Requirements

Document owner: Product

Status: Planning baseline

GUI framework decision: egui/eframe

Primary launch platform: Windows desktop

Last updated: 2026-07-18

## 1. Purpose of this document

This document defines what the OpenManic desktop interface must do, how it must behave, and how the foreground interface must remain separated from background tracking and data processing.

It is written as a product requirements document for engineers and future planning agents. It is intentionally more specific about behavior than appearance. The existing sketches and visual scaffolds are references for information hierarchy and workflows only. Their colors, card treatments, spacing, proportions, navigation, and chart styling are not approved designs and must not be copied wholesale.

The framework choice is settled for planning purposes: OpenManic will use egui through eframe. Future architecture work should start from that decision unless implementation evidence shows that a required behavior cannot be delivered acceptably.

## 2. Requirement language

- **MUST** means required for the release scope named in the surrounding section. An unqualified MUST applies to the MVP defined below.
- **SHOULD** means expected unless a documented trade-off is approved.
- **MAY** means optional or appropriate for a later release.
- **MVP** means the first useful Windows release, not a disposable prototype.

## 3. Product summary

OpenManic is a lightweight desktop time-tracking application that records which application is in focus, organizes the resulting activity into understandable intervals, and helps the user review and intentionally manage their time.

OpenManic is an individual, local-first product. It is intended for a person managing and understanding their own time. It is not a team product, management console, employee-monitoring system, or collaboration platform. No requirement should assume organizations, workspaces, administrators, supervisors, shared dashboards, team reports, or centrally managed policy.

The app combines passive tracking with active focus support:

- Passive foreground-application tracking.
- A daily activity timeline.
- Application and category summaries.
- Date-range statistics.
- Category management.
- Calendar-like review of a day.
- Personal schedule intervals shown over the Timeline and Calendar.
- A Pomodoro/focus timer.
- A configurable dashboard of resizable widgets.

The product should feel continuously alive and trustworthy. Starting a task, changing a filter, navigating dates, importing data, or beginning a focus session must produce immediate visible feedback. The interface must not appear frozen while background work is occurring.

## 4. Product goals

### 4.1 Primary goals

1. Let a user understand where their computer time went without requiring manual time entry.
2. Let a user move from a daily overview to a specific activity interval quickly.
3. Let a user categorize applications with low effort, including bulk assignment.
4. Let a user start and monitor a focus session without leaving the main dashboard.
5. Let a user adapt the dashboard layout without turning the app into a complex report builder.
6. Remain responsive while tracking, aggregating, importing, and rendering substantial history.
7. Keep tracking and data ownership reliable when the main window is hidden, minimized, or being redrawn.
8. Let the user place recurring or one-time personal schedule intervals directly on the Timeline or Calendar and compare intention with recorded activity.

### 4.2 Secondary goals

1. Support additional operating systems through platform adapters without rewriting the GUI.
2. Support new first-party widget types through a stable internal widget contract.
3. Leave a credible path to user-created widget specifications later.
4. Provide themes and appearance customization through semantic design tokens.

### 4.3 Non-goals for the MVP

- Loading arbitrary native Rust plugins at runtime.
- A general-purpose analytics or SQL report builder.
- Team accounts, organizations, shared workspaces, administrative roles, management reporting, employee monitoring, or collaboration workflows.
- Cloud synchronization or online accounts unless separately designed as an optional individual feature later.
- Screenshot recording.
- Universal support for every Linux compositor.
- Pixel-perfect imitation of any reference application.
- Full arbitrary-position canvas layout for dashboard widgets.

### 4.4 Release scope

Phases 0 through 5 in this document are implementation increments toward one MVP release. A feature being scheduled in a later phase does not make it post-MVP unless the table below says so.

| Capability | MVP status |
| --- | --- |
| Today dashboard, live timeline, application totals | Required |
| Pomodoro/focus workflow | Required |
| Categories search, filtering, individual and bulk assignment | Required |
| Overview with Day/Week/Month/Year/Custom ranges | Required |
| Calendar day view with activity and focus overlays | Required |
| Personal schedule interval creation and editing from Timeline and Calendar | Required |
| Explicit Today-dashboard layout edit mode | Required |
| Add, remove, reorder, resize, save, cancel, reset default widgets | Required |
| Saved Overview filters/views | Required |
| Calendar week view | Post-MVP candidate |
| Automatic category rules | Post-MVP candidate |
| User-created declarative or WebAssembly widgets | Post-MVP candidate |
| Theme editor and theme import/export | Post-MVP candidate |
| Arbitrary per-screen dashboard layouts | Post-MVP candidate |

All four primary destinations named in Section 8 are therefore MVP requirements. The Today dashboard is the only user-customizable screen layout in the MVP; Overview, Categories, and Calendar use product-defined responsive layouts.

## 5. Product principles

### 5.1 Immediate acknowledgement

Every user action that may take longer than one frame MUST update visible UI state immediately. For example, pressing Import must show an importing state before file parsing completes, and pressing Start on the Pomodoro must show a Starting/pending state immediately; Running appears only after the authoritative focus service accepts the command.

### 5.2 Quiet by default

The app SHOULD prioritize the current day, current tracking state, and a small number of meaningful summaries. It SHOULD avoid dense permanent toolbars, excessive badges, or debug-style information.

### 5.3 Detail on demand

The default view MUST remain understandable without inspecting every interval. Tooltips, selections, detail panels, and drill-down views SHOULD reveal additional information only when requested.

### 5.4 Explicit modes

Normal dashboard interaction and layout editing MUST be separate modes. Resizing or moving a widget must never be confused with selecting timeline data.

### 5.5 Trust and privacy

The interface MUST make tracking state visible and controllable. It MUST clearly distinguish active tracking, paused tracking, unavailable tracking, excluded applications, idle time, and missing data.

### 5.6 Framework independence below the UI

egui is the presentation technology, not the domain model. Tracking, storage, analytics, category rules, and Pomodoro behavior MUST not depend on egui types.

### 5.7 Progressive disclosure

The default experience MUST be understandable to a non-technical user. Primary actions and explanations MUST use ordinary language rather than implementation terms such as process ID, executable identity, adapter, database transaction, recurrence expansion, or snapshot revision.

Technical details MUST remain available where they are useful for advanced users, troubleshooting, automation, privacy review, or precise configuration. They SHOULD appear through secondary details, expandable sections, advanced settings, tooltips, export formats, or diagnostic views rather than occupying the default workflow.

Advanced options MUST not make the basic workflow harder to discover, and simplified defaults MUST not prevent a technical user from inspecting exact times, executable paths, raw identities, configuration, or diagnostic information when the product supports those details.

### 5.8 Visual communication where useful

OpenManic SHOULD prefer direct visual explanation when spatial, temporal, proportional, or categorical relationships are easier to understand graphically. Timeline intervals, schedule brackets, proportional usage, calendar placement, tracking state, and selected ranges are strong visual candidates.

The app MUST NOT force information into a graph when a list, table, form, text label, or conventional control is clearer. Visual design is a product emphasis, not a requirement that every concept become a chart. Essential graphical information MUST have a textual or structured equivalent for accessibility, precision, and technical inspection.

## 6. Settled decisions and open design space

### 6.1 Settled

- Rust is the implementation language.
- egui/eframe is the GUI framework.
- Windows is the first fully supported platform.
- SQLite is the canonical live data store.
- CSV is an import/export format, not the primary database.
- Activity is stored as time intervals rather than one sample row per second.
- The primary screen is a widget dashboard.
- The timeline is a custom interactive visualization.
- Widget layout uses a constrained responsive grid with serializable placements.
- Foreground rendering and background work are separate.
- The MVP supports first-party widget types compiled with the application.

### 6.2 Intentionally unresolved

- Final color palette, typography, spacing scale, radii, shadows, and density.
- Exact navigation placement: top, side, or a compact hybrid.
- Final name and iconography.
- Whether the default distribution visualization is a ring, stacked bar, or another compact form.
- Exact default placement of the Pomodoro widget.
- Whether categories expose one primary category or multiple categories in the first UI.
- Whether dashboard reordering uses direct drag-and-drop, handles, keyboard actions, or a combination.

These unresolved items MUST be explored with visual design work rather than inferred from the earlier scaffold.

## 7. Users and core jobs

### 7.1 Primary user

The primary user is a non-technical individual who wants private, local insight into their own application usage, personal schedule, and optional focus support. They should not need to understand processes, executables, databases, operating-system APIs, recurrence specifications, or chart terminology to install the app, begin tracking, read the main screen, categorize applications, create a schedule interval, or run a focus timer.

The product must also remain welcoming to technical individuals who want precise data, editable configuration, detailed diagnostics, exports, advanced filtering, or a deeper understanding of how tracking decisions were made. Technical capability is supported through progressive disclosure rather than by designing the default interface as a developer tool.

The user may leave OpenManic running throughout the day and open the window only occasionally.

### 7.2 Core jobs

- See what application is currently being tracked.
- Understand the current day's activity at a glance.
- Inspect a specific time interval.
- See which applications and categories consumed the selected time range.
- Correct or categorize activity with minimal effort.
- Review trends across a week, month, year, or custom period.
- Start, pause, resume, complete, or cancel a focus session.
- Adjust the dashboard to emphasize personally important widgets.
- Pause tracking or exclude sensitive applications.
- Create, inspect, edit, and remove personal schedule intervals without opening a separate planning application.

## 8. Information architecture

The initial product MUST expose four primary destinations:

1. **Today** - current or selected day's dashboard.
2. **Overview** - statistics over configurable date ranges and saved views.
3. **Categories** - application categorization, filters, and assignment rules.
4. **Calendar** - chronological day review, with focus sessions overlaid on activity.

Settings MAY be a fifth destination or a separate window/dialog. It MUST include tracking, privacy, storage, appearance, startup, and platform-permission controls.

Navigation MUST preserve each screen's meaningful local state when switching screens, including the selected date range, filters, scroll position where reasonable, and any unsaved layout-edit warning.

## 9. Global application shell

### 9.1 Persistent information

The shell MUST make the following discoverable from every primary screen:

- Current tracking state.
- Access to pause/resume tracking.
- Primary navigation.
- Access to settings.
- Whether a background job needs attention.
- Whether unsaved layout or configuration changes exist.

The shell SHOULD show the currently tracked application without giving it so much prominence that it distracts from review tasks.

### 9.2 Window and tray behavior

- Closing the main window SHOULD hide it to the system tray when background tracking is enabled.
- The user MUST be able to choose whether Close hides the window or exits the application.
- The tray menu MUST expose Open, Pause/Resume Tracking, Start Focus Session, and Quit.
- Quitting MUST finalize or safely checkpoint the current activity interval and active focus session.
- Hiding, minimizing, resizing, or redrawing the window MUST NOT interrupt tracking.

### 9.3 Global statuses

The shell MUST distinguish:

- Tracking active.
- Tracking paused by the user.
- Tracking blocked by missing OS permission.
- Tracking unavailable due to an adapter error.
- Database read-only or unavailable.
- Import/export in progress.
- Recoverable background error.

Statuses MUST use text or icons in addition to color.

### 9.4 Tracking-state presentation contract

Before timeline implementation, the domain/tracker contract MUST define distinct UI states and interval boundaries for:

- Workstation lock and unlock.
- System sleep, hibernate, resume, and clock discontinuity.
- User-configured idle threshold crossing and return from idle.
- Rapid foreground changes and intervals below the normal display threshold.
- OpenManic itself becoming the foreground application.
- Excluded applications.
- Adapter outage, permission loss, and recovery.
- Local time-zone or daylight-saving changes.

The UI MUST receive explicit causes for Active, Idle/Away, Paused, Excluded, Unavailable, and Unknown/Missing intervals. It MUST NOT infer these meanings from a gap alone. Wall-clock timestamps are used for history and display; monotonic elapsed time SHOULD be used where available to avoid incorrect durations during clock changes.

## 10. Today dashboard requirements

### 10.1 Default content

The default dashboard MUST contain:

1. Activity timeline.
2. Top applications or application-usage list.
3. Time-distribution summary.
4. Pomodoro/focus widget.

The timeline is the primary widget and SHOULD receive the largest default area. The other widgets MUST still remain usable when their width or height is reduced to supported minimums.

### 10.2 Date navigation

- The screen MUST show the selected date clearly.
- Previous day and next day navigation MUST be available.
- A date picker MUST allow direct navigation.
- Returning to Today MUST be a single action when viewing another date.
- The next-day action MUST be disabled or reinterpreted when already on the current day.
- Date navigation MUST update all dashboard widgets consistently.

### 10.3 Shared selection and filtering

Selecting a timeline interval or range MUST produce a shared dashboard selection. Widgets that support the selection MUST recompute their displayed values from that same range.

The UI MUST show that a filter or selection is active, identify its range, and provide a clear action to remove it. A selection must not silently remain active after the user navigates to an incompatible date or range.

### 10.4 Empty, partial, and live days

- A day with no records MUST show an explanatory empty state, not an empty chart frame alone.
- A day that is currently being tracked MUST visually distinguish the open interval from completed intervals.
- Missing time caused by paused tracking or unavailable tracking SHOULD be distinguishable from ordinary inactivity.
- The UI MUST avoid implying that unrecorded time was idle time.

## 11. Activity timeline requirements

### 11.1 Purpose

The timeline is the core explanation and planning surface for a day. It maps recorded activity and personal schedule intervals onto one shared time scale. It lets the user inspect what occurred, select a range, and create or edit intended schedule intervals without turning the graph into a text-heavy calendar.

### 11.2 Continuous three-band structure

The timeline MUST be rendered as one continuous graph with three vertically stacked bands and no visual gaps between adjacent time segments:

1. **Category band** - the dominant and tallest band. It shows the categorized breakdown of time, such as Productive, Entertainment, Communication, or Uncategorized.
2. **Activity-state band** - a thin band immediately below the category band. It shows states currently described as `active`, `inactive`, and `powered_off`. These labels are provisional and MAY be renamed after the state model is finalized.
3. **Application band** - a thin band immediately below the activity-state band. It shows the actual application identity for recorded time, such as Discord or a browser.

All three bands MUST share the same horizontal time coordinate, zoom, pan offset, visible range, segment boundaries, cursor, and selection. There MUST be no whitespace gutter between time segments and no separate per-application lanes. At every visible time position, each band MUST render either its known value or an explicit visual value for Unknown, No Application, Uncategorized, or Unavailable. Missing information MUST not be represented as accidental blank chart space.

The three bands MUST contain no text labels inside their colored segments. Application names, category names, state labels, start/end times, and durations appear through the single hover information box, selection details, or an accessible equivalent. Time-axis ticks and external controls are not considered text inside a band.

The Category band is visually primary. The Activity-state and Application bands remain thin enough to read as supporting strips while still allowing reliable pointer targeting and keyboard/accessibility inspection.

### 11.3 Segment identity and hover information

Each band is segmented independently when its value changes. For example, several application changes may occur inside one continuous Productive category segment.

Hovering any colored segment MUST open one shared information box positioned near the pointer without obscuring the segment more than necessary. Only one timeline hover box is visible at a time. Its content depends on the hovered band:

- Category segment: category name, exact start, exact end, and segment duration.
- Activity-state segment: state name, exact start, exact end, and duration.
- Application segment: application name, icon when available, exact start, exact end, and duration.

The hover box MAY include relevant secondary information such as executable identity or whether a segment is ongoing, but it MUST remain concise. It MUST follow the segment under the pointer rather than showing aggregate all-day duration unless aggregate context is explicitly labeled.

Color provides compact identity but MUST not be the only representation available. Hover, selection details, accessible metadata, and optional legends/settings MUST allow the user to determine each value without relying solely on color.

### 11.4 Navigation and selection interactions

- Clicking a segment MUST select it.
- Clicking empty timeline space SHOULD clear the selection.
- Dragging across time MUST support range selection in the MVP.
- Horizontal panning and continuous zooming MUST be supported.
- Zoom MUST remain anchored to the pointer position when initiated by pointer wheel/gesture and to the current selection or chart center when initiated through controls.
- Zooming out MUST merge or aggregate visually indistinguishable short segments without changing underlying totals or interval records.
- Zooming in MUST reveal the original boundaries and more precise time ticks.
- Panning and zooming MUST move all three bands and schedule overlays together.
- A Reset View action MUST restore the default day range.
- Keyboard users MUST be able to move between meaningful segments or an equivalent accessible list.
- Selection MUST remain visually obvious at supported zoom levels.

Normal range selection and schedule creation MUST be distinguishable. The interface MUST provide an explicit Create Schedule action/mode or an equivalent unambiguous gesture so an ordinary drag does not unexpectedly create a schedule rule.

### 11.5 Schedule interval overlay

Personal schedule intervals are a separate visual layer drawn over the continuous three-band timeline. They express intended time, such as Productive Time, and MUST NOT replace, split, recolor, or mutate recorded category/state/application data.

Each schedule interval MUST appear as a bracket-like enclosure aligned precisely to its start and end times:

- A vertical boundary at the start and end.
- Each vertical boundary extends slightly above and below the timeline bands or the overlay's enclosed region.
- Each boundary has a short inward-facing horizontal cap at its top and bottom, producing the feeling of square brackets enclosing the scheduled range.
- The interior SHOULD remain transparent or minimally treated so all three activity bands stay readable beneath it.
- Selected, hovered, conflicting, and keyboard-focused schedule intervals MUST remain distinguishable without depending only on color.

At normal zoom, the bracket boundary MUST make the exact start and end visually clear. At high density, overlapping schedule intervals MAY be stacked into multiple overlay rows or use another non-destructive conflict treatment, but no interval may silently disappear.

### 11.6 Creating and editing schedule intervals

In Create Schedule mode, the user MUST be able to drag across the timeline to define a start and end. Releasing the drag opens a schedule editor popup anchored to the selected range.

The popup MUST show and allow editing of:

- Schedule label or purpose, such as Productive Time.
- Exact start time.
- Exact end time.
- Duration derived from start and end.
- Optional category association.
- Recurrence choice: **Once**, **Specific weekday**, or **Custom**.

Recurrence behavior is:

- **Once** - one interval on the selected calendar date.
- **Specific weekday** - the same local start/end time on the chosen weekday, with an optional effective start date and end date.
- **Custom** - a recurrence editor capable of selecting multiple weekdays and an effective date range. More advanced recurrence grammar is post-MVP unless separately approved.

The popup MUST provide Save and Cancel. Editing an existing bracket MUST open the same popup with current values and additionally provide Delete. Cancel MUST leave the schedule unchanged. Save MUST validate that end is after start and show inline errors without dismissing the popup.

Start and end times SHOULD snap to a configurable or product-defined increment during dragging while allowing exact typed adjustment in the popup. The graph MUST show the provisional bracket and exact start/end feedback during creation so the user understands where the interval begins and ends before saving.

Schedule rules are personal and local. They MUST NOT include assignees, team ownership, approvals, shared availability, or management fields.

### 11.7 Timeline and Calendar schedule parity

Timeline and Calendar MUST edit the same authoritative schedule intervals and recurrence rules. Creating, editing, or deleting a schedule from either screen MUST be reflected in the other after authoritative acceptance. Neither screen may maintain its own incompatible schedule copy.

### 11.8 Time scale and density

- Time-to-pixel mapping MUST be consistent across segments, grid lines, selections, and tooltips.
- Tick density MUST adapt to available width and zoom.
- Labels MUST not overlap to the point of becoming unreadable.
- Segments too small to display individually MAY be aggregated for rendering, but the underlying records must remain available for inspection at a closer zoom.
- Rendering MUST be limited to visible or near-visible intervals.
- Schedule bracket geometry MUST use the same time-to-pixel transform as all three bands.

### 11.9 Live behavior

- The current interval MUST extend visually while tracking continues.
- Live extension SHOULD update smoothly enough to appear current without forcing continuous maximum-rate repainting.
- When the foreground application changes, the previous interval MUST close and a new interval MUST become visible without requiring manual refresh.
- A delayed storage commit MUST NOT make the UI temporarily erase a valid in-memory interval.

### 11.10 Timeline accessibility

Custom painting MUST be accompanied by accessible metadata or an equivalent structured activity list. At minimum, a selected segment MUST expose its band, value, start, end, and duration to assistive technology. Schedule brackets MUST expose label, start, end, recurrence, category association, selection state, and Edit/Delete actions.

## 12. Application usage widget

The application usage widget MUST show, for the active date range or selection:

- Application icon when available.
- Display name.
- Duration.
- Percentage of included tracked time.
- A proportional visual bar.

It MUST support sorting by duration and name. Selecting an application SHOULD filter or highlight related timeline intervals. The widget MUST clearly state when its values reflect a timeline selection instead of the full day.

At narrow widths, the widget MAY hide secondary percentages or icons, but application name and duration MUST remain visible.

## 13. Time-distribution widget

This widget summarizes allocation across categories, applications, activity states, or another selected grouping.

- The active grouping MUST be visible.
- The total included duration MUST be visible.
- The visualization MUST remain interpretable without relying only on color.
- Selecting a group SHOULD filter or highlight related activity.
- The widget MUST have a compact layout for narrow spans and an expanded layout for wider spans.
- The exact chart form is not settled and MUST be validated during visual design.

## 14. Pomodoro and focus requirements

### 14.1 Required controls

The Pomodoro widget MUST support:

- Configurable focus duration.
- Configurable break duration.
- Start.
- Pause.
- Resume.
- Complete early.
- Cancel.
- Reset before starting.

The MVP MUST allow a user to set an intended start time and end time, or derive one from the other plus duration. These values are planning inputs; a future planned start MUST NOT begin a session without an explicit Start action.

When both start and end are supplied, duration equals end minus start. Editing the duration recomputes the end; editing the end recomputes the duration; editing the start preserves duration and recomputes the end. Invalid, zero-length, or past-only ranges MUST show inline validation and MUST NOT begin. Once explicitly started, reaching the authoritative phase end completes the phase unless the session is paused or cancelled.

### 14.2 Required state display

The widget MUST show:

- Current phase: ready, focus, short break, paused, completed, or cancelled.
- Remaining time.
- Planned start and end times when configured.
- Whether the session has begun.
- Optional linked category or task label.

### 14.3 Background behavior

- The timer MUST be based on monotonic or absolute timestamps, not frame counts.
- Hiding, minimizing, sleeping, or temporarily freezing the UI MUST NOT make the timer drift.
- The GUI MUST derive the displayed remaining time from authoritative timer state.
- Notifications MAY announce phase completion.
- A focus session MUST continue when the main window is hidden unless the user cancels it.
- Timer state MUST be recoverable after an application restart where practical.

Long breaks and automatic multi-session cadence are post-MVP candidates. The MVP MAY let a user manually select another focus or short-break duration after completion.

### 14.4 Relationship to tracked activity

Focus sessions are intentional overlays, not replacements for application tracking. The data model and UI MUST be able to show that a focus session occurred while several applications were used.

## 15. Dashboard layout customization

### 15.1 Normal mode

In normal mode, pointer and keyboard input MUST operate the widgets themselves. Resize and reorder controls MUST not intercept chart selection accidentally.

### 15.2 Layout-edit mode

Layout editing MUST be entered explicitly. While active, the UI MUST:

- Clearly indicate the mode.
- Expose resize and reorder controls.
- Prevent ambiguous chart manipulation where necessary.
- Offer Done/Save and Cancel/Revert actions.
- Prevent a widget from being resized below its declared minimum.
- Show valid placement targets.
- Preserve widget identity and state while moving it.

### 15.3 Layout model

The MVP layout MUST use a constrained responsive grid rather than arbitrary pixel coordinates. A saved placement MUST include:

- Stable widget instance ID.
- Stable widget kind/version.
- Order.
- Width span.
- Height class or row span where supported.
- Widget-specific configuration.
- Optional appearance overrides.

Layouts MUST be serializable, versioned, validated before use, and recoverable to defaults. Invalid or outdated layouts MUST not prevent the app from opening.

### 15.4 Responsive behavior

- Widgets MUST reflow when the window becomes narrower.
- Reflow MUST not overwrite the user's saved desktop layout.
- Each widget MUST define minimum, preferred, and supported compact sizes.
- A compact widget MAY simplify internal content but MUST retain its primary value and action.
- The MVP SHOULD optimize for desktop widths from 720 to 1440 logical pixels and MUST be explicitly tested at 720, 1024, and 1440 logical pixels plus 125%, 150%, 175%, and 200% scaling.

### 15.5 Widget addition and removal

The Today layout editor MUST support adding another instance of supported widget kinds, removing optional widgets, reordering widgets, resizing widgets, and restoring the default layout. Save MUST persist the edited layout. Cancel MUST restore the complete layout that was active when edit mode began. The product MUST prevent removal of every route to an essential action unless that action remains accessible elsewhere.

## 16. Widget extensibility requirements

### 16.1 MVP definition

A custom widget in the MVP means a new first-party widget implemented in Rust and registered with the application. The app MUST support multiple widget kinds without hard-coding every kind into the main dashboard function.

Every registered `WidgetDefinition` MUST declare:

- Stable kind identifier and schema version.
- Display name, description, and picker metadata.
- Supported capabilities and user actions.
- Minimum, preferred, compact, and maximum size behavior.
- Configuration schema, defaults, validation, and migration behavior.
- Required snapshot/data dependencies.
- Actions the renderer is permitted to produce.
- Accessible name/value/action strategy and non-visual fallback.
- Optional appearance-override schema.

If a saved layout references a missing, disabled, or incompatible renderer, the dashboard MUST show a recoverable placeholder that identifies the widget and offers Remove or Reset. A missing renderer MUST NOT prevent the remaining layout or application from opening.

### 16.2 Widget responsibilities

A widget renderer MAY:

- Render presentation-ready snapshot data.
- Maintain transient interaction state.
- Produce typed application actions.
- Request derived data through a controller action.
- Adapt presentation to its allocated rectangle.

A widget renderer MUST NOT:

- Query SQLite directly.
- Poll operating-system tracking APIs.
- Spawn unmanaged worker threads.
- Own authoritative domain state.
- Block the UI thread waiting for I/O or computation.
- Store unversioned configuration that cannot be migrated.

### 16.3 Future extensibility

The widget definition and configuration formats SHOULD remain serializable so a future declarative or WebAssembly widget system is possible. Loading arbitrary Rust dynamic libraries is explicitly out of scope until ABI, security, crash isolation, permissions, and versioning have dedicated designs.

## 17. Overview requirements

### 17.1 Date range

Overview MUST support Day, Week, Month, Year, and Custom ranges. The active range and its exact dates MUST be visible.

Navigation MUST handle calendar boundaries and local time correctly. Changing range granularity SHOULD preserve the user's contextual date where reasonable.

### 17.2 Default overview

The default Overview SHOULD contain one dominant time-allocation visualization and supporting widgets for top applications and categories. It SHOULD avoid repeating every Today widget without purpose.

Selecting a period in the dominant chart MUST update compatible supporting widgets. The UI MUST reveal active selections and allow clearing them.

### 17.3 Saved views

A saved view MUST store a normalized range definition and the effective values for:

- Range type.
- Grouping.
- Filters.
- Sort order.
- Widget configuration.

Each value MAY encode an explicit default or empty state, but loading a view MUST reproduce the same effective range, grouping, filters, sort order, and compatible widget configuration that was saved.

A saved view MUST NOT contain executable code or arbitrary SQL. Views MUST be renameable, duplicable, reorderable, and deletable with confirmation where recovery is not available.

In the MVP, saved Overview views do not own a dashboard layout. They store range, grouping, filters, sort order, and compatible widget configuration within the product-defined Overview layout. Today owns the only user-editable dashboard layout. Per-view and per-screen custom layouts are post-MVP candidates.

## 18. Categories requirements

### 18.1 Application list

The Categories screen MUST provide a searchable list of known applications with:

- Application icon where available.
- Display name.
- Executable identity or path in secondary details.
- Assigned category or categories.
- Recent or total usage where useful.
- Whether the app is excluded from tracking.

### 18.2 Filtering and assignment

The screen MUST support:

- Search by display name and executable identity.
- Filter by category.
- Filter for uncategorized applications.
- Multi-selection.
- Bulk category assignment.
- Removing a category assignment.
- Creating a category during assignment without losing selection.

The interface SHOULD make the common task, assigning several uncategorized apps, possible without opening a dialog for every row.

### 18.3 Category definition

A category MUST have a stable ID and display name. It MAY have a color, icon, description, and productivity classification. Renaming a category MUST preserve assignments.

The domain model SHOULD support many-to-many assignments even if the first interface emphasizes one primary category.

### 18.4 Rules

Automatic category rules MAY be added after basic manual assignment. Candidate match inputs include executable path, process name, window-title pattern, and application identity. Rule priority and conflict behavior MUST be explicit before release.

## 19. Calendar requirements

### 19.1 MVP scope

The MVP Calendar MUST provide a day view with a vertical time axis. A week view MAY follow after overlap and density behavior are validated.

### 19.2 Content

The day view MUST be able to show:

- Tracked application activity.
- Idle or unavailable gaps.
- Focus/Pomodoro sessions as overlays.
- Personal schedule intervals using the same authoritative schedule rules as the Timeline.
- User annotations or corrections when those features are introduced.

### 19.3 Interaction

- Selecting a calendar block MUST reveal its time range and source.
- The user SHOULD be able to navigate to the corresponding timeline selection.
- Overlapping focus and activity data MUST remain distinguishable.
- Dense periods MUST remain navigable without hiding records permanently.
- The user MUST be able to enter Create Schedule mode and drag over the vertical time scale to define a schedule interval.
- Releasing the drag MUST open the same Once/Specific weekday/Custom schedule editor used by the Timeline.
- Existing schedule intervals MUST be selectable, editable, and deletable from Calendar.
- Calendar schedule boundaries MUST use a bracket-like enclosure adapted to the vertical time axis and MUST not obscure the recorded activity or focus data beneath them.
- Exact start/end feedback and snapping behavior MUST match the Timeline.

The calendar must not fabricate narrative summaries from raw application names unless a separate summarization feature is designed and clearly labeled.

## 20. Settings and privacy requirements

Settings MUST include:

- Start tracking automatically.
- Start application at login.
- Close-to-tray behavior.
- Pause/resume tracking.
- Idle detection threshold and policy.
- Excluded applications.
- Whether window titles are collected.
- Data location.
- Import/export.
- Retention controls if supported.
- Theme and density preferences.
- Notifications and focus-session sounds.
- Platform permission status and remediation.

Sensitive settings MUST explain their data implications. Disabling window-title collection MUST take effect prospectively and MUST not silently delete existing data.

## 21. Visual and interaction design requirements

### 21.1 Design direction

The final interface SHOULD feel like a focused consumer productivity app rather than an egui demo or developer tool. Achieving this requires an internal design system and custom component treatments.

The previous visual scaffold is rejected as a final design. It MAY inform screen inventory and interaction coverage only.

The interface SHOULD communicate through graphics where the visual relationship is the information: time along an axis, duration through length, proportions through bars or suitable charts, schedule boundaries through overlays, and current state through compact status treatment. Graphics MUST remain purposeful, restrained, and readable; decorative charts, redundant visualizations, and graphical controls with unclear meaning are not acceptable.

Conventional text, lists, tables, menus, and forms SHOULD be used whenever they make entry, comparison, precision, or explanation easier. A technical-looking control is not automatically more powerful, and a graphical control is not automatically easier for a non-technical user.

### 21.2 Semantic design tokens

The GUI MUST use semantic tokens for:

- Canvas, panel, card, elevated, and overlay surfaces.
- Primary, secondary, disabled, inverse, and link content.
- Primary, hover, pressed, selected, focus, and disabled interactions.
- Success, warning, error, active, away, paused, excluded, and unavailable states.
- Timeline grid, cursor, selection, and overlay marks.
- Chart series and category colors.
- Spacing, padding, radius, stroke, typography, icon size, and motion timing.

Renderers MUST not scatter hard-coded colors and spacing throughout widget code.

### 21.3 Theme resolution

Appearance MUST resolve in this order:

1. Base application theme.
2. Widget-kind defaults.
3. Valid saved widget-instance overrides.
4. Temporary interaction state.

Application and category identity colors are data-level identity choices and MUST remain distinguishable from general theme colors.

### 21.4 Theme modes

The app MUST support dark mode. It SHOULD support light mode and following the operating-system preference before the MVP is considered polished. Both modes MUST meet contrast and state-distinction requirements.

The MVP theme UI is limited to built-in Dark, Light, and Follow System choices, with Light and Follow System treated as release-quality targets rather than a general editor. Theme import/export and a user-facing token editor are post-MVP candidates. The MVP MAY expose a small approved set of widget appearance choices, but arbitrary per-widget token editing is not required. Any appearance override that is persisted MUST use the versioned widget appearance schema rather than raw egui values.

### 21.5 Motion

Motion SHOULD communicate state change, not decorate idle screens. Animations MUST be interruptible, MUST not delay input, and SHOULD respect reduced-motion preferences where the platform exposes them.

### 21.6 Default and advanced presentation

- Default screens MUST use familiar terms such as Application, Time, Category, Schedule, Tracking, and Focus Session.
- Process names, executable paths, rule precedence, database locations, and diagnostic IDs SHOULD be hidden until requested unless they are required to distinguish two items.
- Advanced controls MUST be labeled and grouped; they MUST not be mixed unpredictably into primary actions.
- Exact numeric values and time ranges MUST remain available even when a graphical summary is primary.
- Icons without obvious universal meaning MUST have text labels or accessible names.
- A visualization MUST not be the only way to perform an essential corrective or configuration action when precise keyboard/form entry is more appropriate.

## 22. Foreground and background separation

### 22.1 Definition

For this product:

- **Foreground/UI layer** means the egui event/render loop, screen controllers, transient interaction state, and presentation of immutable or cheaply readable snapshots.
- **Background layer** means OS activity tracking, database I/O, schedule-rule persistence and recurrence expansion, import/export, aggregation, indexing, icon loading/decoding where expensive, category-rule application, and other work that can exceed the frame budget.

Visual foreground/background surfaces are part of theming and are separate from this execution boundary.

Background is a logical execution boundary. It does not require a separate operating-system process; it may be implemented with dedicated threads, an async runtime, or a controlled hybrid. Hiding or closing the window to the tray MUST leave required tracking, persistence, and focus services running in the same application process.

### 22.2 Foreground responsibilities

The foreground layer MUST:

- Drain available events without blocking.
- Update `AppModel` through typed actions/reducers or equivalent controlled transitions.
- Render from presentation-ready snapshots.
- Show optimistic acknowledgement for commands where safe.
- Display job progress, cancellation, completion, and failure.
- Request repaints only as often as needed for input, animation, or live data.
- Keep transient hover, focus, open-menu, and drag state out of the domain model.

The foreground layer MUST NOT:

- Perform blocking channel receives.
- Run database queries during a frame.
- Wait for a mutex held during significant background work.
- Parse large imports.
- Aggregate full history.
- Poll the OS for foreground application state.
- Base timers on repaint frequency.

### 22.3 Background responsibilities

Background services MUST:

- Own long-running and blocking work.
- Publish typed events and immutable/versioned view snapshots.
- Report determinate progress when total work is knowable.
- Report indeterminate activity when total work is not knowable.
- Accept cancellation when cancellation is safe.
- Coalesce superseded high-frequency updates.
- Protect data integrity independently of whether the window is visible.
- Avoid flooding the UI with an event per raw sample.
- Validate and persist schedule-rule mutations and publish the resulting authoritative interval projections used by Timeline and Calendar.

### 22.4 Communication pattern

The expected flow is:

```text
User action -> AppAction -> command dispatcher -> background service
     |                                             |
     +-> immediate pending UI state                +-> DomainEvent/JobEvent
                                                         |
                                                         v
UI drains events -> AppModel/ViewSnapshot revision -> repaint affected views
```

Commands and events MUST have stable IDs where operations can overlap. Late results from superseded queries MUST not replace newer state.

### 22.5 Snapshot requirements

A UI snapshot SHOULD be immutable and reference-counted where data is substantial. It MUST carry a revision or request identity sufficient to determine whether it is newer and relevant.

Snapshots SHOULD contain presentation-ready values such as the aligned Category/Activity-state/Application timeline bands, visible schedule intervals, aggregated totals, formatted identity information, and Pomodoro state. Formatting and bracket geometry that depend on locale or available pixels MAY remain in the UI.

### 22.6 Backpressure and coalescing

- Command and event channels MUST be bounded or otherwise protected from unbounded growth.
- User commands and domain mutations MUST be delivered losslessly and processed in a defined order per affected entity.
- Cancellation MUST be idempotent and scoped to a stable job ID.
- Progress events MAY replace older progress for the same job.
- Live tracking updates SHOULD publish the latest interval state rather than an unbounded sequence of per-tick messages.
- Repaint hints, superseded query snapshots, and intermediate progress MAY be coalesced or dropped when a newer equivalent exists.
- Domain mutations, final job outcomes, committed tracking transitions, and critical persistence failures MUST NOT be dropped.
- If the UI falls behind, the latest correct snapshot is more important than replaying every intermediate rendering state.

### 22.7 Failure isolation

A tracker, import, icon, or analytics failure MUST not crash the UI where recovery is possible. Failures MUST be converted into typed errors with a user-facing summary and diagnostic details available for support.

### 22.8 Shutdown lifecycle

An explicit Quit begins controlled shutdown. The application MUST stop accepting new nonessential jobs, request cancellation of cancellable work, checkpoint the open activity interval and focus state, flush critical configuration/database writes, and then terminate background services. If critical state cannot be flushed, the app MUST report the failure and offer Retry or Quit Anyway rather than silently claiming success.

Force termination by the operating system cannot be guaranteed, so periodic checkpointing MUST limit recoverable loss independently of the normal Quit path.

### 22.9 Concurrent job presentation

- The originating screen or widget MUST show local state for work it initiated.
- The global shell MUST show jobs that continue after navigation or require attention.
- Concurrent jobs MUST be distinguishable by stable ID and user-facing name.
- A background refresh MUST not steal keyboard focus or replace valid prior content with an empty state.
- An actionable failure MUST remain discoverable until acknowledged, retried, or dismissed.

## 23. Responsiveness and performance requirements

### 23.1 Interaction targets

- Pointer and keyboard actions MUST receive visible acknowledgement by the next rendered frame under normal conditions.
- Routine UI updates SHOULD complete within the frame budget: 16.7 ms at 60 Hz and preferably under 8.3 ms on 120 Hz displays.
- No intentional operation on the UI thread SHOULD exceed 4 ms without profiling and justification.
- Progress SHOULD update approximately 10-30 times per second at most unless the visual specifically benefits from higher frequency.

These are product targets, not claims that every background operation completes within one frame.

### 23.2 Timeline targets

- The timeline MUST remain interactive with at least 10,000 intervals in the loaded day/range test dataset.
- Panning, zooming, hover, and selection MUST not perform full-history database work.
- Static geometry SHOULD be cached by data revision, bounds, zoom, and theme revision.
- Only visible or near-visible geometry SHOULD be generated.
- Dense data SHOULD be downsampled or aggregated for rendering without corrupting totals.

### 23.3 Memory and startup

- Startup SHOULD present the shell and tracking status before expensive historical aggregation completes.
- Historical views SHOULD load progressively or show a clear loading state.
- The UI SHOULD avoid cloning full activity histories per frame.
- Large icons and decoded images SHOULD use bounded caches.

### 23.4 Background progress

Every operation expected to exceed roughly 250 ms MUST expose one of:

- Determinate progress with completed/total work.
- Indeterminate progress with a meaningful current phase.
- Immediate background completion notification when foreground progress would be distracting.

Cancellation MUST immediately change the UI to Cancelling when cleanup is asynchronous. Cancelled and failed operations MUST not be shown as successful.

### 23.5 Benchmark protocol

Phase 0 MUST publish a reproducible performance fixture containing benchmark hardware/OS details, build profile, synthetic-data generator seed, interval distribution, category/application counts, and measurement procedure. The 10,000-interval requirement refers to raw activity intervals available to the selected day/range before render aggregation.

The release gate MUST record p50 and p95 frame time plus input-to-visible-feedback latency during pan, zoom, hover, and selection. Exact p95 thresholds will be approved after the first representative spike, but they MUST be no weaker than the 16.7 ms routine 60 Hz frame target without a documented product exception. Tests MUST be run at the exact logical widths and scaling factors named in Section 15.4.

## 24. State ownership

### 24.1 Authoritative state

Persistent/domain services own authoritative state. Authoritative domain state includes:

- Current tracking state and open activity interval.
- Stored activity records.
- Category identities and assignments.
- Personal schedule intervals and recurrence rules.
- Pomodoro/focus session lifecycle.
- Saved layouts and views.
- Background job lifecycle.

### 24.2 UI application state

`AppModel` or its equivalent SHOULD own:

- Current route.
- Selected date/range.
- Shared selection and filters.
- Selected or draft schedule interval and pending schedule command state.
- Active dashboard layout.
- Theme choice.
- Current view-snapshot references.
- Pending command states and surfaced errors.

These are projected, cached, or draft UI representations of authoritative state. `AppModel`, widget state, and `ViewSnapshot` MUST never become the write authority for persisted activity, category assignments, schedule rules, focus lifecycle, saved layouts/views, or final job outcomes.

Every optimistic mutation MUST follow a visible reconciliation lifecycle:

```text
Draft/Pending -> Confirmed
Draft/Pending -> Rejected -> restore or reload authoritative state + show error
```

Commands MUST have correlation IDs so confirmations and rejections update the correct pending operation. A newer local draft MUST not be overwritten by a late response to an older command. Optimistic Pomodoro acknowledgement MAY show Starting immediately, but Running is authoritative only after the focus service accepts the command; rejection restores Ready or the previous authoritative phase and explains the failure.

### 24.3 Transient widget state

Transient state includes hover targets, drag handles, popup visibility, temporary text edits, animation progress, and chart cursor position. This state MAY live in widget/controller state and SHOULD not be persisted unless it represents a user choice.

## 25. Suggested module boundaries

The initial codebase MAY begin as modules in one workspace crate, but dependencies should follow these logical boundaries:

```text
openmanic-app
  startup, lifecycle, tray, dependency wiring

openmanic-domain
  entities, commands, events, validation, policies

openmanic-tracker
  platform-neutral ActivitySource and OS adapters

openmanic-storage
  SQLite repositories, migrations, CSV import/export

openmanic-analytics
  range aggregation, indexing, downsampling, snapshots

openmanic-focus
  Pomodoro state machine and notification intents

openmanic-schedule
  personal schedule intervals, recurrence rules, expansion, validation

openmanic-widgets
  widget definitions, configuration, layout schema, registry contracts

openmanic-theme
  semantic tokens and appearance resolution

openmanic-ui-egui
  shell, screens, controllers, renderers, accessibility, UI tests
```

The UI crate MAY depend on domain-level types and read-only snapshot types. Domain, tracker, storage, analytics, focus, and schedule modules MUST NOT depend on `egui`.

## 26. Accessibility requirements

- All essential actions MUST be keyboard reachable.
- Focus order MUST be predictable and visible.
- Custom widgets MUST publish accessible roles, names, values, and actions where supported by egui/AccessKit.
- Custom-painted charts MUST provide an equivalent textual or structured representation for essential data.
- Color MUST not be the only indicator of tracking state, selection, category, error, or progress.
- Text and controls MUST remain usable at 125%, 150%, 175%, and 200% scaling.
- Tooltips MUST not contain the only available version of essential information.
- Reduced motion and high-contrast needs SHOULD be considered in the theme system.

Accessibility acceptance MUST include manual keyboard review and platform screen-reader smoke testing, not only visual inspection.

## 27. Loading, empty, error, and recovery states

Every data-driven widget MUST define:

- Initial loading.
- Refreshing while prior data remains visible.
- Empty range.
- Partial data.
- Tracking paused.
- Tracking unavailable.
- Query failed.
- Data repaired or recovered.

Refreshing SHOULD preserve valid prior data with a non-blocking refresh indicator instead of replacing the widget with a blank spinner.

Errors MUST explain what failed, whether tracking/data is still safe, and what the user can do. Technical diagnostics MAY be expandable or copyable.

## 28. Persistence and recovery requirements

- Layout, theme, saved views, and settings MUST be persisted separately from ephemeral hover or selection state.
- Configuration writes MUST be atomic or recoverable.
- A malformed saved layout MUST fall back to a known default and preserve the invalid data for diagnostics where practical.
- Database migrations MUST occur outside normal frame rendering with visible progress if they are not immediate.
- The current open activity interval MUST be recoverable after an unexpected exit as accurately as platform information permits.
- The app MUST not lose tracking data because a chart failed to render.

## 29. Telemetry, logging, and diagnostics

OpenManic SHOULD be usable without cloud telemetry. Local diagnostics SHOULD include:

- Tracker state transitions and adapter errors.
- Database migration and write failures.
- Background job lifecycle.
- Snapshot/query durations.
- Slow-frame or slow-widget diagnostics in development builds.
- Layout migration failures.

Logs MUST avoid recording sensitive window titles unless the user explicitly enables detailed diagnostics and is warned.

## 30. Testing requirements

### 30.1 Unit tests

- Pomodoro state transitions and time calculations.
- Date-range navigation.
- Category assignment rules.
- Layout validation and migration.
- Time-to-pixel and pixel-to-time transforms.
- Cross-band boundary alignment and schedule-bracket time transforms.
- Once, Specific weekday, and Custom schedule recurrence expansion across date and daylight-saving boundaries.
- Schedule overlap and conflict validation.
- Selection/filter reducers.
- Event ordering and stale-result rejection.

### 30.2 Integration tests

- Tracker event -> interval persistence -> snapshot -> UI update.
- Tracking continues while the window is hidden.
- Background import progress, cancellation, and failure.
- Layout edit, save, restore, and invalid-layout fallback.
- Pomodoro behavior across window hide and simulated sleep/resume.
- Timeline schedule creation reflected in Calendar and Calendar edits reflected in Timeline.
- Schedule Save confirmation, rejection rollback, recurrence edit, and deletion.

### 30.3 GUI tests

- Navigation and retained screen state.
- Non-technical usability review of first launch, tracking status, timeline hover, categorization, schedule creation, and Pomodoro start.
- Progressive-disclosure review confirming that technical details are discoverable without appearing in the default workflow.
- Timeline hover/selection and shared filtering.
- Continuous three-band rendering without internal segment labels or accidental blank gaps.
- Band-specific hover information for Category, Activity-state, and Application strips.
- Bracket overlay creation, exact start/end feedback, popup validation, and keyboard editing.
- Keyboard focus traversal.
- Compact and expanded widget presentations.
- Empty/loading/error states.
- Theme token application.
- Scaling at supported factors.

### 30.4 Performance tests

- 10,000-interval timeline interaction.
- Large application/category lists.
- Rapid foreground-app changes.
- Three independently segmented timeline bands plus overlapping schedule brackets.
- Concurrent tracking, import, and overview query.
- Event coalescing when the UI is deliberately slowed.

## 31. Security and privacy acceptance

- Tracking MUST be local by default.
- The product MUST have no organization, administrator, manager, team-reporting, or employee-monitoring mode.
- Personal schedules, categories, focus sessions, and activity history MUST belong only to the local individual profile in the MVP.
- The user MUST be able to pause tracking immediately.
- Excluded apps MUST not have new activity details persisted beyond the declared exclusion behavior.
- Window-title collection MUST be configurable.
- Export actions MUST clearly state destination and included range.
- Destructive data deletion MUST identify scope and require confirmation unless readily undoable.
- Plugins and remote code are not permitted in the MVP widget system.

## 32. Delivery plan

### Phase 0: architecture and visual direction

- Confirm domain/UI dependency rules.
- Define commands, events, snapshots, job states, widget definitions, layout schema, and theme tokens.
- Produce approved low-fidelity flows and separate visual-direction explorations.
- Define Windows tracking and privacy behavior.

Exit criteria: architecture review completed; visual scaffold is not treated as final design; acceptance datasets exist.

### Phase 1: Windows vertical slice

- Windows foreground-application adapter.
- Interval recording and SQLite persistence.
- Application shell and tray lifecycle.
- Today screen with live timeline and application-usage widget.
- Continuous three-band Category/Activity-state/Application timeline renderer and hover model.
- Shared dark theme tokens.
- Tracking pause/resume and core error states.

Exit criteria: tracking continues while hidden; live intervals appear; stored history survives restart; UI does not block on storage.

### Phase 2: focus and categorization

- Pomodoro/focus state machine and widget.
- Categories screen with search, filters, multi-select, and bulk assignment.
- Category-aware timeline and summaries.
- Notifications and focus overlays.
- Personal schedule domain/service with Once, Specific weekday, and Custom recurrence.
- Timeline schedule creation, bracket overlays, editor popup, persistence, and deletion.

Exit criteria: timer remains accurate while hidden; categorization propagates to summaries without direct database work in widgets; schedule edits reconcile with authoritative state and render at exact shared time coordinates.

### Phase 3: overview and calendar

- Day/week/month/year/custom Overview ranges.
- Dominant allocation visualization and shared selections.
- Saved views.
- Day Calendar with focus and personal schedule overlays.
- Calendar schedule creation/editing using the same editor and authoritative rules as Timeline.

Exit criteria: large ranges load progressively; stale queries cannot overwrite newer selections.

### Phase 4: dashboard customization

- Explicit layout-edit mode.
- Add/remove/reorder/resize supported widgets.
- Versioned layout persistence and fallback.
- Compact/expanded widget presentations.
- Appearance settings and light/system themes.

Exit criteria: layouts survive restart and scaling changes; all default widgets remain usable at supported sizes.

### Phase 5: hardening and release

- Accessibility review.
- Performance profiling and regression budgets.
- Import/export and recovery testing.
- Installer, autostart, permissions, update strategy, and release diagnostics.

Exit criteria: all MVP acceptance criteria pass on supported Windows versions and test hardware.

## 33. MVP acceptance criteria

The MVP is acceptable when all of the following are true:

1. OpenManic records foreground application intervals on Windows while the main window is visible, hidden, or minimized.
2. Pausing and resuming tracking is immediate, visible, and reliable.
3. The Today timeline is one continuous graph containing aligned Category, Activity-state, and Application bands with no text inside colored segments and no accidental gaps between adjacent time sections.
4. Hovering a section in any band opens one concise pointer-adjacent information box with the correct band value, exact start/end, and duration.
5. The timeline displays live and historical intervals and supports pan, pointer-anchored zoom, reset, hover, click selection, and drag range selection across all three aligned bands.
6. The user can create a schedule interval from Timeline, see precise provisional start/end feedback and a bracket-style overlay, and save it as Once, Specific weekday, or Custom through the schedule popup.
7. The same authoritative schedule interval appears in Calendar and can be edited or deleted from either Timeline or Calendar without modifying recorded activity.
8. Selecting a timeline segment or range updates compatible summary widgets consistently.
9. The application usage widget reports correct totals and percentages for the active range.
10. The user can search applications and assign categories individually and in bulk.
11. Overview supports Day, Week, Month, Year, and Custom ranges; its shared selection updates supporting data and saved views restore their defined range, grouping, filters, sort, and configuration.
12. Calendar day view displays tracked activity, focus sessions, and personal schedule intervals distinctly and can navigate a selected recorded block to corresponding timeline context.
13. The Pomodoro supports configured duration, start/end planning, explicit start, pause/resume, completion, and cancellation without depending on repaint frequency.
14. The UI remains interactive while importing, aggregating, persisting data, and expanding schedule recurrence.
15. Background jobs expose progress or meaningful activity, cancellation where safe, and clear failure states.
16. In explicit Today layout-edit mode, users can add, remove, reorder, and resize supported widget instances; Save persists the edited layout, Cancel restores the prior layout, and Reset restores defaults.
17. The dashboard reflows at 720, 1024, and 1440 logical pixels and at 125%, 150%, 175%, and 200% scaling without losing the saved desktop arrangement.
18. Layout configuration is versioned; invalid layouts and missing widget renderers fall back safely without preventing startup.
19. Essential functions are keyboard accessible and custom visuals have accessible equivalents.
20. The app clearly communicates active, paused, unavailable, excluded, idle, and missing tracking states.
21. A 10,000-raw-interval test range meets the approved p95 budget for three-band timeline pan, zoom, hover, selection, drag-range selection, and schedule-overlay rendering.
22. No storage, tracking, schedule persistence/recurrence expansion, import, or full-history aggregation work blocks the egui thread.
23. Hiding the window leaves tracking and focus services active, while explicit Quit follows the checkpoint-and-flush shutdown lifecycle.
24. The MVP contains no team, organization, administrator, manager, shared-dashboard, or employee-monitoring workflow.
25. A non-technical user can complete first launch, understand whether tracking is active, inspect an interval, categorize an application, create a one-time schedule interval, and start a Pomodoro without needing implementation terminology or technical setup.
26. A technical user can discover supported exact timestamps, executable identity, data location, export, advanced settings, and diagnostics without those details overwhelming the default interface.
27. Temporal, proportional, and schedule relationships use graphical presentation where it materially improves understanding, while every essential value and action remains available through clear text, structured data, or conventional controls where appropriate.

## 34. Architecture questions for the next planning session

The next architecture discussion should resolve these questions without reopening the egui decision:

1. Will background orchestration use Tokio, dedicated threads, or a hybrid, and why?
2. What are the exact `Command`, `DomainEvent`, `JobEvent`, and `ViewSnapshot` contracts?
3. How are stale query results identified and discarded?
4. Which state transitions are optimistic, and how are failures reconciled?
5. How is the current open interval checkpointed and recovered?
6. What are the SQLite schema and migration boundaries?
7. What is the exact responsive grid and placement algorithm?
8. How are widget renderer registrations, versions, minimum sizes, and configuration migrations represented?
9. How are custom-painted timeline elements exposed through AccessKit or an equivalent structured view?
10. What caches exist, who owns them, and what invalidates them?
11. How is platform sleep/resume handled for tracking and focus timers?
12. Which privacy-sensitive values are collected, persisted, exported, and logged?
13. What are the exact `ScheduleRule`, `ScheduleOccurrence`, recurrence, exception, and migration schemas?
14. How are local-time schedule rules expanded across daylight-saving changes, time-zone changes, and effective date boundaries?
15. How are the independently segmented Category, Activity-state, and Application bands projected onto one gap-free shared time range?
16. What final domain names replace or confirm `active`, `inactive`, and `powered_off`, and how do they map to Paused, Excluded, Unavailable, and Unknown/Missing states?
17. How do normal range selection, schedule creation, schedule editing, pan, and zoom gestures arbitrate without ambiguity?
18. Which technical details belong in default, secondary, advanced, and diagnostic presentation layers, and what user research validates that division?

## 35. Source documents

This document consolidates and supersedes the GUI product requirements contained in:

- `docs/gui-tiebreaker-and-architecture.md`
- `docs/rust-gui-evaluation.md`

Those documents remain useful for framework comparison and earlier reasoning. This document is the working source of truth for egui GUI behavior and product requirements.

## 36. Verification record

An independent requirements verifier reviewed this document on 2026-07-18 against the user request and both source documents. The review confirmed that:

- egui is treated as decided rather than compared again.
- The earlier visual scaffold is explicitly non-approved.
- Foreground and background responsibilities are separated in detail.
- The document is suitable as an architecture-planning handoff after resolving scope and ownership ambiguities.

The review identified release-scope, widget-customization, range-selection, state-ownership, worker-lifecycle, Pomodoro-scheduling, layout-ownership, widget-contract, theme-scope, and performance-verification gaps. Those findings were incorporated into Sections 4.4, 11.3, 14, 15, 16, 17.3, 21.4, 22, 23.5, 24, and 33.

Remaining open items are deliberate architecture or visual-design decisions listed in Sections 6.2 and 34, not silent assumptions.

Subsequent product-owner clarifications added the individual-only scope, non-technical primary audience, progressive disclosure for technical users, purposeful graphics-first emphasis, continuous three-band timeline, pointer-adjacent hover behavior, bracket-style schedule overlays, recurrence popup, and shared Timeline/Calendar scheduling requirements. These additions are captured in Sections 3-5, 7, 11, 19, 21-25, and 30-34.
