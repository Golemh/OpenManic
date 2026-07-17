# Rust GUI Evaluation for OpenManic

Date: 2026-07-17

## Current direction

OpenManic will likely use **egui/eframe with Rust and SQLite**. This keeps the application in one language, suits a lightweight data-oriented desktop tool, and provides the custom 2D drawing needed for the timeline.

The final choice should be confirmed with a small prototype of the timeline, since matching a highly polished consumer-style interface requires more custom styling in egui than it would in an HTML/CSS frontend.

## Required visualizations

### Interactive activity timeline

The main visualization is a time-coordinate canvas containing interval rectangles. Each activity session is positioned using its start time, end time, and lane:

```text
pixel_x     = (event_time - visible_start) / visible_duration * canvas_width
pixel_width = event_duration / visible_duration * canvas_width
```

The basic rectangles are simple to draw. A complete timeline also needs:

- Horizontal zooming and panning
- Time tick and grid generation
- Hover tooltips
- Click and drag selection
- A current-time cursor
- Application or category colors
- Filtering by application, tag, or category
- Combining very small adjacent intervals at low zoom levels
- Drawing only visible intervals as the history grows

egui exposes a low-level 2D painting API suitable for a custom timeline. `egui_plot` also provides coordinate transforms, custom time-axis formatting, grids, zooming, panning, and cursor lines, although this visualization will probably be a custom widget rather than an off-the-shelf plot.

References: [egui overview](https://github.com/emilk/egui), [egui plot capabilities](https://docs.rs/egui_plot/latest/egui_plot/struct.Plot.html)

### Application usage summary

The secondary visualization is a conventional list containing:

- Application icon and name
- Total active duration
- Percentage of the filtered period
- A horizontal percentage bar
- Sorting and optional selection or filtering

This does not need a chart library. It can be built from normal egui rows, with each bar width calculated from the application's percentage of total tracked time.

## Assessment against project priorities

| Requirement | Rust and egui assessment |
| --- | --- |
| Cross-platform UI | Good across Windows, macOS, and Linux |
| Cross-platform activity tracking | Possible, but necessarily platform-specific |
| Developer setup | Cohesive through Cargo, with native toolchain requirements |
| End-user setup | Good once installers are produced |
| Appearance | Good, but a polished custom design requires deliberate styling |
| OS integration | Excellent, particularly on Windows |
| Data handling | Excellent |
| Runtime performance and memory | Excellent |
| Background tracking | Excellent |
| GUI ecosystem maturity | Suitable for this application, though less consolidated than Qt or web UI |

## Cross-platform considerations

The UI is not the main portability challenge. egui/eframe supports Windows, macOS, Linux, web, and Android. The foreground-application tracker is inherently platform-specific.

### Windows

Windows is a strong first target. Microsoft's `windows` crate exposes Win32, process, window-management, UI Automation, notifications, and related APIs directly to Rust.

Reference: [Microsoft Rust Windows API](https://microsoft.github.io/windows-docs-rs/)

### macOS

Foreground-window tracking is possible, but some information requires user-granted permissions. Window titles may require Screen Recording permission, while other monitoring features can involve Accessibility permission.

### Linux

X11 generally permits foreground-window inspection. Wayland intentionally does not expose a universal interface for enumerating every window or discovering the active window, so support must be compositor-specific. A current cross-platform Rust library supports KDE/KWin and Hyprland on Wayland and otherwise falls back to X11, illustrating why generic Wayland support should not be promised.

Reference: [active-win-pos-rs platform notes](https://docs.rs/crate/active-win-pos-rs/0.11.0)

Tracking should therefore be hidden behind a platform-neutral interface:

```rust
trait ActivitySource {
    fn current_activity(&self) -> Result<ActiveWindow, ActivityError>;
}
```

Expected adapters include:

```text
WindowsActivitySource
MacOsActivitySource
X11ActivitySource
KdeWaylandActivitySource
HyprlandActivitySource
```

The recommended rollout is Windows first, followed by explicitly named Linux environments rather than a claim of universal Linux support.

## Appearance and interaction

egui is particularly productive for developer tools, visualizers, and data-heavy desktop applications. Its immediate-mode model makes application state and custom drawing straightforward. Its default theme looks like a tool UI, but spacing, colors, typography, borders, rounding, and widgets can all be customized.

For OpenManic, the visual design should be treated as a small internal design system rather than scattered widget overrides. Shared theme values should cover:

- Surface and panel colors
- Text hierarchy
- Application and category colors
- Spacing and row density
- Corner radii and strokes
- Hover, selected, and focused states
- Timeline grid and cursor styling

The timeline prototype should validate text clarity, dense event rendering, high-DPI behavior, zoom interaction, and responsiveness with a large history before the GUI choice is made final.

## OS integration

Rust can directly access:

- Win32 and Windows Runtime APIs
- Processes and foreground windows
- File systems and permissions
- System tray menus
- Autostart facilities
- Native notifications
- Global shortcuts
- Idle detection
- IPC and local sockets
- Native libraries through C-compatible FFI

Platform calls that require `unsafe` should be isolated inside the platform adapters. The tracking, aggregation, storage, and UI layers can remain safe Rust.

## Data handling

Rust has strong libraries for this workload:

- Strongly typed activity records
- `chrono` or `time` for timestamps
- Serde for configuration and interchange formats
- `csv` for typed CSV import and export
- SQLite through `rusqlite`
- Iterators and maps for lightweight aggregation
- Polars or Arrow later if advanced analytics becomes necessary

References: [Serde data model](https://serde.rs/data-model.html), [Rust CSV support](https://docs.rs/csv/latest/csv/tutorial/)

### Storage recommendation

CSV should not be the primary live database. Use:

- **SQLite** as canonical activity storage
- **CSV** for import and export
- **JSON or TOML** for editable configuration and category rules

SQLite provides transactions, indexes, range queries, relational categories, schema migrations, and better crash resistance. `rusqlite` can bundle SQLite with the application so the user does not install it separately.

References: [rusqlite](https://github.com/rusqlite/rusqlite), [transaction handling](https://docs.rs/rusqlite/latest/rusqlite/struct.Transaction.html)

A starting schema could contain:

```text
applications
  id, executable_path, display_name, icon_key

activity_segments
  id, application_id, window_title, started_at, ended_at

categories
  id, name, color

application_categories
  application_id, category_id
```

Activity should be stored as intervals rather than one row per second. When the foreground application changes, the current interval is closed and a new one begins. This reduces writes and directly matches the timeline representation.

## GUI ecosystem maturity

Rust GUI development is mature enough to ship this application, but the ecosystem is less consolidated than Qt, .NET/WPF, SwiftUI, or web UI.

Current strengths include:

- Mature application logic and data processing
- Mature OS bindings and native interoperability
- Productive custom GPU and canvas rendering
- egui's strong fit for tools, dashboards, and visualizers
- Lightweight native deployment

Areas that still require care include:

- Accessibility support varies by framework
- Native platform behavior may require explicit implementation
- APIs are less standardized across GUI libraries
- Ready-made business widgets and visualization packages are fewer than in the web ecosystem
- Achieving a highly polished design can require custom widgets and painting

For OpenManic, these limitations are acceptable because the interface is compact and dominated by two custom data visualizations rather than a large collection of conventional business forms.

## Options considered

1. **Tauri 2, a web frontend, Rust, and SQLite** provides the easiest route to a polished interface and the largest visualization ecosystem, but introduces a second language and a webview-based UI.
2. **egui/eframe, Rust, and SQLite** keeps the application in one language, is lightweight, and is a strong fit for a data-oriented desktop tool. This is the likely choice.
3. **Slint, Rust, and SQLite** offers an attractive declarative native-style UI, but has a smaller ecosystem of visualization components.

Iced is capable, but its advanced documentation and learning experience are currently less approachable. It is not the leading option for this project unless its Elm-like application architecture becomes a specific requirement. See the [Iced documentation](https://docs.iced.rs/).

## Recommended first milestone

Build a Windows-only vertical slice with:

1. A Windows `ActivitySource` adapter
2. Interval-based activity recording
3. SQLite persistence
4. A custom egui timeline with pan, zoom, hover, and a current-time cursor
5. The application usage summary list
6. A shared dark theme

This prototype will test the two highest-risk decisions—OS tracking and timeline interaction—while keeping the architecture ready for additional operating-system adapters.
