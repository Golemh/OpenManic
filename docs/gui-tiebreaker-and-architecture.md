# OpenManic GUI tie-breaker and architecture

Date: 2026-07-18

## Decision

Use **egui/eframe** for the first production vertical slice.

This is a project-specific choice, not a claim that egui is the best general-purpose Rust GUI. OpenManic's highest-risk UI work is a dynamic, densely interactive timeline plus a user-arranged dashboard of custom visual widgets. Those are the areas where egui has the shortest path from data to pixels and the least resistance to runtime composition.

Keep the domain, tracking, storage, aggregation, theme tokens, widget definitions, and saved layout independent from egui. Treat egui as a renderer and interaction adapter so that the decision remains reversible through the first vertical slice.

## Weighted comparison

Scores are 1-5 and intentionally weighted for OpenManic's proposed interface.

| Criterion | Weight | egui | iced | Slint |
| --- | ---: | ---: | ---: | ---: |
| Custom timeline and dense charts | 25% | 5.0 | 4.0 | 3.0 |
| Runtime widget composition and resizing | 20% | 5.0 | 3.5 | 2.5 |
| Product polish and design-system ergonomics | 15% | 3.5 | 3.5 | 5.0 |
| Background work and reactive progress | 15% | 4.0 | 5.0 | 4.0 |
| Accessibility and conventional controls | 10% | 3.5 | 3.0 | 4.0 |
| State-model discipline and testability | 10% | 3.5 | 5.0 | 4.0 |
| Ecosystem/API confidence for this spike | 5% | 4.5 | 3.0 | 4.0 |
| **Weighted result** | **100%** | **4.30** | **3.93** | **3.60** |

These numbers should only be overturned by a working spike, not by adding more abstract criteria.

## Practical pros and cons

### egui/eframe

Pros:

- Excellent fit for a custom timeline: allocate a rectangle, map time to x-coordinates, paint intervals, and use the same response for hover, selection, dragging, panning, and zooming.
- Immediate-mode rendering naturally supports a runtime list of heterogeneous dashboard widgets.
- Easy to build adaptive widgets that change their internal presentation from the rectangle they receive.
- Background workers can publish snapshots through channels and request a repaint without blocking the UI thread.
- One Rust codebase, native deployment, AccessKit integration through eframe, and useful UI testing support.
- Current egui styling includes global/local styles and widget classes, which makes a small internal design system more realistic than it was in older versions.

Cons:

- A polished consumer-app appearance will be custom work. Defaults look like a tool, not the reference screens.
- Immediate mode does not enforce clean state architecture. A large `App::ui` can become tangled unless screen controllers and renderers remain deliberately small.
- Dashboard drag/resize behavior is not a first-class application layout system. We should own a small grid layout model or adopt a docking library after the interaction is proven.
- Custom-painted visuals require explicit accessibility nodes, keyboard behavior, and testing.
- Major/minor egui releases can move APIs; pin versions and upgrade intentionally.

### iced

Pros:

- Best built-in architectural discipline: state, messages, update, view, tasks, and subscriptions align well with tracking and progress events.
- Canvas and custom widgets can implement the proposed timeline.
- Dynamic application themes and conventional responsive layout primitives are solid.
- Pure update logic is straightforward to test.

Cons:

- Advanced custom widgets expose more framework machinery: widget trees, layout nodes, renderer types, shells, overlays, and lifetimes.
- Heterogeneous runtime widget registries are possible but less pleasant because `Element` and messages are strongly typed through the view tree.
- A draggable/resizable dashboard still needs a custom layout/editor layer.
- The current online documentation includes development-version APIs, so version pinning and examples require care.

Choose iced instead if the team strongly prefers Elm-style state transitions and is willing to spend more effort on the timeline and dashboard editor.

### Slint

Pros:

- Best visual authoring and clearest foreground/backend separation through `.slint` components, properties, callbacks, and Rust services.
- Strong reusable component story, live preview, built-in palettes/style metrics, and good conventional UI/accessibility declarations.
- Likely the fastest route to a polished, consistent shell when the screen tree is mostly known at compile time.
- Thread-safe event-loop invocation provides a clear path for worker progress.

Cons:

- The compiled component tree is less natural for an open-ended runtime dashboard containing arbitrary widget types. A fixed enum of known widget variants works, but every new type tends to add another UI branch.
- The timeline is possible, but the data-to-custom-pixels workflow is less direct than egui or iced Canvas.
- User-resizable, freely rearranged cards need an application-specific layout editor rather than falling naturally out of the declarative markup.
- It introduces a second UI language and build boundary.
- The licensing options are reasonable for desktop applications, but they are an extra policy surface that egui and iced do not introduce.

Choose Slint instead if visual designers must work independently in declarative files and installable/custom runtime widget types are no longer a priority.

## Tie-breaker acceptance test

Do not spend weeks building three applications. The egui spike should earn the decision by passing these tests:

1. Render 10,000 activity intervals while panning and zooming the visible range without obvious input lag.
2. Hover/select a segment, filter all summary widgets from that selection, and clear it with keyboard and pointer input.
3. Reflow the default dashboard at 720, 1024, and 1440 logical pixels without clipped labels or unusable cards.
4. Enter layout-edit mode, change widget spans, reorder widgets, persist the layout, and restore it.
5. Run a simulated background import while immediately showing Running, current progress, cancellation, and partial chart updates.
6. Verify focus order, accessible labels/values, 125-200% scaling, and light/dark theme contrast.

If egui fails primarily on polish or accessibility, prototype the same shell in Slint. If it fails primarily because application state becomes hard to reason about, prototype the same timeline in iced.

## Product structure

```text
openmanic-app
  composition root, startup, tray, lifecycle

openmanic-domain
  ActivitySegment, Application, Category, FocusSession
  commands, events, validation, category rules

openmanic-tracker
  ActivitySource trait
  Windows / macOS / X11 / compositor adapters

openmanic-storage
  SQLite repositories, migrations, CSV import/export

openmanic-analytics
  range queries, aggregation, downsampling, view snapshots

openmanic-widgets
  WidgetDefinition, WidgetInstance, WidgetConfig
  DashboardLayout, grid placement, serialized view presets

openmanic-theme
  semantic tokens, chart palettes, per-widget overrides

openmanic-ui-egui
  shell, screen controllers, widget renderer registry
  custom timeline, charts, layout editor, accessibility
```

Initially these may be modules in one crate. Split them into crates only where compilation boundaries or platform-specific dependencies provide a real benefit.

## Runtime flow

```text
OS activity adapters -+
Pomodoro timer -------+-> domain events -> storage/analytics worker
CSV import -----------+                         |
                                                | Arc<ViewSnapshot>
                                                v
UI event drain -> AppModel -> screen controller -> widget renderers
      ^                |                              |
      +---- commands --+------------------------------+
```

The UI drains a bounded event queue without blocking. High-frequency updates use a latest-value snapshot or coalescing channel; they do not enqueue one repaint per sample.

## Core state shapes

```rust
pub struct AppModel {
    pub route: Route,
    pub range: TimeRange,
    pub selection: Selection,
    pub dashboard: DashboardLayout,
    pub theme: ThemeId,
    pub jobs: JobStates,
}

pub struct ViewSnapshot {
    pub revision: u64,
    pub timeline: Arc<[TimelineSegment]>,
    pub app_totals: Arc<[AppTotal]>,
    pub category_totals: Arc<[CategoryTotal]>,
    pub pomodoro: PomodoroSnapshot,
}

pub struct WidgetInstance {
    pub id: WidgetId,
    pub kind: WidgetKind,
    pub placement: GridPlacement,
    pub config: WidgetConfig,
    pub appearance: AppearanceOverrides,
}
```

`ViewSnapshot` should contain presentation-ready, immutable data. A renderer should never query SQLite or perform a large aggregation during a frame.

## Screen plan

### Today

- Compact range navigator and date heading.
- Primary horizontal activity timeline, with activity state and category/application lanes.
- Application totals list with proportional bars.
- Time-distribution ring or category breakdown.
- Pomodoro card with duration presets, start/pause, stop, current phase, and optional planned start/end times.
- Layout edit mode, separate from normal interaction mode.

### Overview

- Day/week/month/year/custom range selector.
- One dominant allocation chart; clicking a period updates the other widgets.
- Saved view presets containing range, filters, grouping, and layout—not arbitrary SQL.
- Avoid a generic report builder in the first release.

### Categories

- Searchable application list with icons and current categories.
- Filters for uncategorized/category/application status.
- Multi-selection and a bulk `Assign category` action.
- Rule-based assignments later: executable path, process name, title match, and priority.
- Keep categories many-to-many in the data model even if the first UI exposes one primary category.

### Calendar

- Day view first, using a vertical time axis and activity/focus blocks.
- Focus sessions and Pomodoro intervals are overlays on tracked application activity, not replacements for it.
- Week view later after overlap, density, editing, and navigation are proven.

## Widget contract

Use a serializable definition plus an egui renderer registry. Do not serialize Rust trait objects.

```rust
pub trait WidgetRenderer {
    fn kind(&self) -> WidgetKind;

    fn show(
        &mut self,
        ui: &mut egui::Ui,
        instance: &WidgetInstance,
        snapshot: &ViewSnapshot,
        actions: &mut Vec<AppAction>,
    );
}
```

The layout stores widget IDs, kinds, spans, minimum sizes, order, and configuration. The registry maps stable kind identifiers such as `timeline.v1` or `pomodoro.v1` to code compiled into the application.

External native plugins should not be an MVP feature. If installable third-party widgets become important, prefer a versioned declarative spec or sandboxed WebAssembly interface over loading Rust dynamic libraries.

## Theme contract

Define semantic tokens independently from `egui::Visuals`:

```text
surface.canvas / panel / card / elevated
content.primary / secondary / disabled
interaction.primary / hover / pressed / focus
status.active / away / offline / success / warning / error
timeline.grid / cursor / selection
chart.series[0..N]
spacing, radius, stroke, typography, motion
```

Resolve appearance in this order:

```text
base theme -> widget-kind defaults -> saved widget overrides -> transient state
```

Map the resolved theme into egui globally, while custom painters read the same tokens directly. Application/category identity colors remain data, not theme values.

## Milestones

1. **Visual spike:** the scaffold in `prototypes/egui-dashboard`, followed by the full acceptance test above.
2. **Windows vertical slice:** foreground app adapter, interval recording, SQLite, Today timeline, app totals, tray lifecycle.
3. **Interaction slice:** selection/filter propagation, categories, Pomodoro, progress/cancel states.
4. **Customization slice:** saved dashboard layouts, resize/reorder, view presets, theme editor/import.
5. **Hardening:** large-history benchmarks, accessibility pass, crash recovery, import/export, packaging.

## Current official references

- [eframe application model and repaint behavior](https://docs.rs/eframe/latest/eframe/)
- [egui custom widget model](https://docs.rs/egui/latest/egui/widgets/trait.Widget.html)
- [egui style system](https://docs.rs/egui/latest/egui/style/struct.Style.html)
- [iced application, tasks, subscriptions, and themes](https://docs.iced.rs/iced/)
- [iced Canvas](https://docs.iced.rs/iced/widget/canvas/)
- [Slint components](https://docs.slint.dev/latest/docs/slint/guide/language/coding/file/)
- [Slint widget styles and palettes](https://docs.slint.dev/latest/docs/slint/reference/std-widgets/style/)
- [Slint accessibility properties](https://docs.slint.dev/latest/docs/slint/reference/common/)
- [Slint desktop licensing](https://slint.dev/terms-and-conditions)
