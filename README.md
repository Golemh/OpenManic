# OpenManic

OpenManic is a time tracker for your own computer. It runs quietly in the
Windows tray, notes which application is in front, and turns that into a
timeline of your day you can actually read.

There's no account and no server, and it doesn't make network requests.
Everything lives in one SQLite file in a folder you can see.

[Download](https://github.com/Golemh/OpenManic/releases/latest) ·
[Building from source](#building-from-source) ·
[Your data](#your-data)

## What it does

**Today.** The main screen is a timeline with three thin bands stacked on one
time axis: the category you were in, whether you were active, idle, paused or
powered off, and the exact application. Scroll to zoom, drag the viewfinder to
pan, click any segment for its start, end and duration, or drag to select a
range and let the rest of the dashboard narrow to it.

**Schedules.** Drag across the timeline to block out intended time ("Deep work,
9 to 11"). Schedules draw as brackets over what really happened, so the gap
between plan and reality is right there. They can repeat on chosen weekdays,
and editing a repeating one asks whether you mean this day, this day onward,
or every occurrence.

**Focus timer.** A Pomodoro-style timer with 25, 50 and 90 minute sessions,
started from the dashboard or the tray. Finished sessions are saved and show
up on the calendar.

**Categories.** Each app belongs to at most one category. OpenManic starts you
off with eight (Development, Communication, Design, Entertainment, Web
Browsing, AI Assistants, Productivity, Security & Utilities) and pre-assigns the
apps it recognises. After that, your assignments win. Search, filter and
reassign apps from the Categories screen.

**Overview and Calendar.** Totals by app and category over the last 7 days, 12
weeks or 12 months, with views you can save. The calendar is a
day view of activity, focus sessions and schedules together.

**A dashboard you can rearrange.** Hit *Edit layout* to add, remove, reorder
and resize widgets. Nothing moves until you save.

It also does the boring parts: hides to the tray when you close the window,
starts at sign-in if you ask (no admin rights), keeps one instance running,
and ignores app switches shorter than a few seconds so alt-tabbing doesn't
shred your timeline.

## Install

OpenManic needs Windows 11 on x86-64. It's a portable program with no
installer.

1. Grab `OpenManic-<version>-windows-x86_64.zip` from the
   [releases page](https://github.com/Golemh/OpenManic/releases).
2. Check it against the `.sha256` file next to it. Builds aren't signed yet,
   so SmartScreen may complain.
3. Extract it somewhere writable, such as `Documents\OpenManic`, and run
   `OpenManic.exe`.

Data goes in an `OpenManicData` folder beside the executable.

To update, quit from the tray menu, replace `OpenManic.exe` and leave
`OpenManicData` where it is. Keep the old executable until the new one has
opened once. Database upgrades take a backup first.

### Trying it with sample data

`demo/` has a few days of made-up activity. Copy `OpenManic.exe` into
`demo/` and run `Run-OpenManic-Demo.cmd`. It installs the sample database to
`C:\Users\Public\OpenManicDemo` (the sample is tied to that exact path) and
starts the app against it. Your real data isn't touched.

## Your data

OpenManic records:

- which application was in the foreground, and when
- whether you were active, idle (5 minutes with no input by default), paused,
  or away from the computer
- window titles

Window titles are the sensitive part: they can hold document names, email
subjects, chat names. They stay in the local database, CSV export asks before
it includes them, and diagnostic bundles never contain them.

OpenManic does not take screenshots, log keystrokes, or look inside browsers.
Chrome, Edge, Firefox and Brave count as Web Browsing, whatever site is open.

From Settings you can:

- **Export and import CSV.** Import merges into your existing data and reports
  any rows it rejected. It never replaces what you have.
- **Back up and restore.** Backups are verified before they count as done, and
  are safe to take while the app is running. Copying the live SQLite files is
  not.
- **Move the data folder.** The new folder has to be empty. The old one is kept
  until the copy checks out.
- **Export a diagnostics bundle** for bug reports. It leaves out app names,
  file paths and window titles.

If you'd rather keep data somewhere else from the start:

```
OpenManic.exe --data-dir D:\Tracking
```

or set `OPENMANIC_DATA_DIR`. When neither is set, OpenManic uses the folder
you last moved it to, then `OpenManicData` beside the executable, and asks
you if that isn't writable. Don't put the data folder on a network share.

## Building from source

You need Rust. The toolchain is pinned in `rust-toolchain.toml` (1.97.1), so
`rustup` will fetch the right one. SQLite is compiled in, and nothing else
needs installing.

```
git clone https://github.com/Golemh/OpenManic
cd OpenManic
cargo run --release
```

The default build uses the wgpu renderer. To use OpenGL instead:

```
cargo run --release --no-default-features --features renderer-glow,platform-windows
```

Repository checks go through `cargo xtask`:

| Command | Does |
|---|---|
| `cargo xtask quality` | fmt, check, clippy (`-D warnings`), tests, rustdoc, doc links |
| `cargo xtask dependency-check` | cargo-deny against `deny.toml` |
| `cargo xtask release-check` | all of the above, plus feature and release builds |
| `cargo xtask package-windows` | builds the portable ZIP and its checksum into `dist/` |

Pushing a `v*` tag runs the same packaging in GitHub Actions and attaches the
ZIP to a release.

### How it's put together

```
crates/
  openmanic                  the executable: startup, data folder, wiring
  openmanic-domain           intervals, categories, schedules, focus sessions
  openmanic-application      commands, projections, background runtime
  openmanic-platform         Windows foreground tracking, tray, autostart
  openmanic-storage-sqlite   schema, migrations, single writer, backups
  openmanic-ui-egui          egui screens and the timeline renderer
tools/                       xtask, fixture generators, renderer benchmarks
docs/gui/                    product requirements and technical specs
```

The UI never touches the database or the OS directly. It draws immutable
snapshots and sends back typed actions. Tracking, storage and aggregation run
on background threads, so a heavy import or a long history doesn't stall the
window. The UI crate forbids `unsafe`; the Windows API calls live behind safe
wrappers in `openmanic-platform`.

The specs in [`docs/gui/spec/`](docs/gui/spec/README.md) are worth reading
before a large change.

## Status

Early. 0.1.x works day to day on Windows 11, but expect rough edges and
possibly schema changes between versions. Back up before upgrading.

Linux support (NixOS on Sway first) is planned. The code is split so it only
needs a new platform adapter, but none exists yet.

On the list after that: a calendar week view, automatic category rules, and
theme customisation.

## Contributing

Bug reports and pull requests are welcome. [CONTRIBUTING.md](CONTRIBUTING.md)
covers building, testing and the one piece of paperwork: before your first
pull request is merged, you sign a Contributor License Agreement. It lets us
offer OpenManic under a commercial license as well as the GPL (see below).
Contributions are unpaid.

## License

OpenManic is free software under the [GNU GPL v3](LICENSE). You can use it,
change it and share it, commercially too, as long as anything you distribute
stays under the GPL with its source available.

If you want to ship a closed-source version, we offer a separate commercial
license. Ask by [opening an issue](https://github.com/Golemh/OpenManic/issues/new?template=commercial-license.md).
Both maintainers read every request together, in public. We'll keep a list
here of any commercial license we grant and who it went to.
