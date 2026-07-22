# OpenManic

OpenManic is a lightweight Windows desktop app that tracks how much time you spend in each application. It records the foreground application (and, optionally, window titles) entirely on your own machine, then shows the breakdown on a timeline and in per-app and per-category statistics. There is no account, no server, no installer, and no background service — just a single portable executable that keeps all of its data in a folder beside itself.

This README is written for judges and first-time users: it covers how to run the released build, how to build from source, and how to load the bundled sample database so you can see a populated dashboard immediately.

## Requirements

- Windows 11, 64-bit (x86-64).
- No runtime, database server, or admin rights are required to run the released build.
- To build from source you additionally need the Rust toolchain pinned in `rust-toolchain.toml` (installed automatically by `rustup` when you build).

## Quick start (recommended: run the release build)

1. Download the latest `OpenManic-v0.1.0-windows-x86_64.zip` and its `.sha256` checksum from [GitHub Releases](https://github.com/Golemh/OpenManic/releases).
2. Extract the **entire** ZIP to a writable folder, for example `Documents\OpenManic`.
3. Double-click `OpenManic.exe`.
4. Complete the short first-launch screen: it explains that OpenManic records your foreground application locally and lets you accept the defaults or open settings.
5. Keep `OpenManic.exe` in that folder. On first run it creates an `OpenManicData` folder beside itself and stores everything there.

Windows SmartScreen may warn that the executable is unsigned. You can verify the download against the accompanying `.sha256` file before running it.

Once tracking is on, the **Today** timeline fills in live as you switch between applications, and the app list and statistics update alongside it. Closing the window hides OpenManic to the tray so it keeps tracking; use the tray menu to pause tracking, start a focus session, or quit.

## Try it with the sample database (for judges)

If you would rather see a fully populated dashboard right away instead of waiting for live data to accumulate, use the bundled demo dataset in the [`demo/`](demo/) folder.

1. Copy `OpenManic.exe` from the release into the `demo/` folder, next to `Run-OpenManic-Demo.cmd`.
2. Double-click `Run-OpenManic-Demo.cmd`.
3. Complete the one-time first-launch consent screen.
4. Use the timeline's date controls to step back to the sample days (around **18–20 July 2026**) to see several days of seeded activity across Development, Browsing, Communication, and Design categories.

The launcher points OpenManic at a fixed data directory (`C:\Users\Public\OpenManicDemo`) and installs the sample `openmanic.sqlite3` there on first run.

Why the fixed path: OpenManic derives a store identity from the *exact* path of its data directory and checks it every time the store is opened (this is how it detects an accidentally copied or moved store). The sample database is built for that one path, so the demo must be launched against it — which is exactly what `Run-OpenManic-Demo.cmd` does. To start from an empty store instead, just run `OpenManic.exe` on its own and it will create a fresh `OpenManicData` folder beside itself.

The sample database is generated deterministically by [`scripts/generate-demo-database.py`](scripts/generate-demo-database.py), which reproduces the app's own migration-checksum and store-identity algorithms so the file passes OpenManic's on-open validation. Regenerate it any time with:

```powershell
python scripts/generate-demo-database.py . demo/demo-data/openmanic.sqlite3
```

## Build from source

The release pipeline is driven through `cargo xtask`. From the repository root on Windows with the pinned Rust toolchain installed:

```powershell
# Build the portable release exe and produce dist/OpenManic-v<version>-windows-x86_64.zip
# (+ .sha256 and BUILD-INFO.txt)
cargo xtask package-windows
```

That wraps `scripts/package-windows.ps1`, which builds with the WGPU renderer and Windows platform adapter and stages the ZIP under `dist/`. Pushing a tag such as `v0.1.0` runs the Windows release workflow (`.github/workflows/windows-release.yml`) and publishes the generated files to GitHub Releases; the workflow can also be started manually to produce a downloadable Actions artifact without publishing a release.

To build or run the executable directly without packaging:

```powershell
# Build the release executable
cargo build -p openmanic --release --no-default-features --features "renderer-wgpu,platform-windows" --locked

# Or just run it during development
cargo run -p openmanic --no-default-features --features "renderer-wgpu,platform-windows"
```

The produced executable is `target/release/openmanic.exe`. A `renderer-glow` feature is available as an OpenGL fallback if WGPU is unavailable on a given machine.

The full pre-release gate (Windows only), which checks both renderers, builds the release, packages it, and prints the manual smoke checklist, is:

```powershell
cargo xtask release-check
```

## Where your data lives

By default OpenManic keeps everything in an `OpenManicData` folder beside the executable:

```text
OpenManic.exe
OpenManicData/
├── openmanic.sqlite3        # your local SQLite store (STRICT schema)
├── backups/
├── logs/
└── ...
```

Everything is local and offline. To back up or move data, quit OpenManic first (so the SQLite files are closed) and copy the whole `OpenManicData` folder, or use the backup controls in Settings while it is running. Data is never placed on a network share. You can also point OpenManic at a different location with `--data-dir <path>` or the `OPENMANIC_DATA_DIR` environment variable.

## How we built it with Codex and GPT-5.6

OpenManic was implemented through an agent-driven workflow using **OpenAI Codex** powered by **GPT-5.6**, following the execution plan in [`docs/gui/spec/implementation-plan.md`](docs/gui/spec/implementation-plan.md) and the operating rules in [`docs/gui/implementation/agent-execution-strategy.md`](docs/gui/implementation/agent-execution-strategy.md) and [`AGENTS.md`](AGENTS.md).

Rather than one monolithic prompt, the work was structured as a supervised pipeline:

- **Specification first.** The product requirements and technical specs under `docs/gui/` were treated as the source of truth. GPT-5.6 was not allowed to invent product behavior — when a task hit a missing decision or a contract conflict, the agent stopped and escalated instead of guessing.
- **A primary integrator plus bounded implementation agents.** A primary agent owned the task graph, the shared `Cargo.toml`/`Cargo.lock`, and the cross-crate public contracts, and performed all Git integration. Delegated "Terra" implementation agents (Codex / GPT-5.6) each worked a single bounded task in its own isolated Git worktree, editing only an explicit writable-path allowlist so that concurrent tasks could never collide.
- **Evidence-gated acceptance.** No change was accepted just because the model said it was correct. Each task ran the narrowest deterministic checks that could disprove it, and each batch had to pass `cargo xtask quality` (formatting, the strict Clippy lint set in `Cargo.toml`, docs, and dependency policy) before integration. At phase and gate boundaries an independent verifier agent reviewed the base-to-head diff read-only before the primary made the final accept / reject / repair decision.
- **Clean, reviewable history.** Work landed as small, conventionally-scoped commits tagged with task IDs (`feat(tracking): [OM-210] ...`) on `codex/*` branches, so every change traces back to a specific task in the plan.

In short, GPT-5.6 through Codex did the implementation, but the architecture, the crate boundaries, and every acceptance decision were governed by the written specification and a human-supervised integration process — which is what kept a fairly large Rust workspace (domain, application, platform-adapter, SQLite storage, and egui UI crates) coherent.

## Repository layout

```text
crates/
├── openmanic                 # executable: bootstrap, CLI, data-root resolution, composition
├── openmanic-domain          # core domain types
├── openmanic-application     # application services / ports
├── openmanic-platform        # OS foreground-tracking adapters (Windows)
├── openmanic-storage-sqlite  # STRICT SQLite store, migrations, backup/recovery
└── openmanic-ui-egui         # egui dashboard, timeline, settings
docs/                         # product requirements and technical specifications
scripts/                      # packaging and demo-database tooling
demo/                         # sample database + launcher for judges
tools/                        # xtask, fixture-generator, and dev utilities
```
