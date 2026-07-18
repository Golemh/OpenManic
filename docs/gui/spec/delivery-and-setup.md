# OpenManic MVP delivery and setup specification

## 1. Goals

The Windows MVP must be usable without installing a language runtime, database server, background service, or package manager.

Distribution goals:

- One self-contained artifact per supported OS/architecture.
- Bundled Rust application and SQLite implementation.
- No account, network setup, or online activation.
- Data beside the artifact by default.
- User-selectable data location.
- Manual executable replacement for updates.
- One running instance per signed-in user.

NixOS is a post-Windows target. Its one-file artifact remains an evidence-driven packaging spike because Linux graphics/display dependencies and the Nix store make portability materially different from Windows.

## 2. Windows artifact

Initial release target:

```text
Windows 11
x86-64
portable OpenManic executable
one selected egui renderer
bundled SQLite
```

The artifact MUST NOT require:

- Administrator rights.
- An installer.
- A system SQLite installation.
- Python, Node.js, Java, .NET runtime setup, or a sidecar service.
- Windows App SDK runtime installation.
- Network access.

Native Windows system DLLs and APIs are expected platform dependencies. SQLite may be compiled and linked through `rusqlite`’s bundled support.

## 3. Data-root resolution

### 3.1 Resolution order

Resolve the data root before opening SQLite:

1. Explicit `--data-dir <path>` command-line override.
2. `OPENMANIC_DATA_DIR` environment override for advanced/automated use.
3. Valid per-user bootstrap locator created after the user selected a custom directory.
4. `OpenManicData` beside the actual release artifact when writable.
5. Blocking first-launch data-directory chooser when the artifact parent is not writable.

An override is validated before replacing a valid existing location. Invalid or unavailable overrides produce a clear recovery choice; they do not silently create a fresh empty store elsewhere.

### 3.2 Default layout

```text
<artifact directory>/
├── OpenManic.exe
└── OpenManicData/
    ├── openmanic.sqlite3
    ├── backups/
    ├── logs/
    ├── cache/
    │   └── icons/
    ├── imports/
    │   └── reports/
    └── crash/
```

The live `openmanic.sqlite3-wal` and `openmanic.sqlite3-shm` files may appear while the app is running. The app does not treat them as disposable copies.

### 3.3 Bootstrap locator

The owner approved a tiny per-user locator when the artifact directory is unwritable or the user explicitly chooses another directory.

Windows stores only:

- Bootstrap schema version.
- Selected data-root path.
- Stable store ID, when known, to detect accidental path reuse.

It may use a current-user registry key or an equivalently small per-user configuration file. It MUST NOT contain activity, titles, categories, schedules, focus history, layout, or other substantive data.

On future NixOS support, the equivalent locator MAY live under the user’s XDG configuration directory. The same data-minimization rule applies.

### 3.4 Writeability and locking

Before tracking begins, validate:

- Directory exists or can be created.
- Create/write/rename/delete of a temporary probe file.
- SQLite locking/WAL compatibility.
- Enough free space for startup/migration.
- Location is not a network share.
- Exclusive OpenManic data-root lock can be acquired.

If validation fails, show the cause and open the chooser. Do not claim tracking is active.

## 4. First launch

First launch sequence:

1. Resolve or ask for a data directory.
2. Open/create the store and run immediate migrations.
3. Explain that OpenManic records the foreground application locally.
4. Explain window-title collection and that titles are stabilized/deduplicated.
5. Show where data is stored and how to change it.
6. Show tracking, close-to-tray, and login-start defaults.
7. Accept defaults or open settings.
8. Start tracking only after consent and adapter readiness.

No account, organization, server, SQL configuration, or compositor configuration is part of Windows first launch.

## 5. Single instance

Windows permits one OpenManic process per signed-in user.

Startup order:

1. Acquire/check a current-user-scoped instance mutex.
2. If another instance exists, connect to its current-user-ACL local activation pipe.
3. Send `Activate` and exit successfully.
4. If this is the owner, resolve/lock the data root before opening SQLite.
5. Refuse a second writer if the data-root lock is already held even when activation coordination fails.

Local activation is not a general IPC API and is not accessible across user sessions.

## 6. Close, tray, and exit

Defaults approved by the owner:

- Tracking begins after first-launch consent.
- Closing the main window hides it to the tray while background tracking is enabled.
- The first close-to-tray action informs the user that OpenManic is still running and shows how to quit/change the setting.
- The tray menu exposes Open, Pause/Resume Tracking, Start Focus Session, and Quit.
- Explicit Quit begins coordinated shutdown.

The tray notification may be suppressed by Windows, so the close policy also remains discoverable in the app and settings.

## 7. Start at login

Start-at-login is optional and one-click.

Windows implementation:

- Write the correctly quoted current executable path plus `--background` to the current-user Run key.
- Never require administrator access.
- If the portable executable moves, detect and offer to repair/remove the stale entry.
- Reflect actual configured state in Settings.
- State that Windows may delay or disable startup entries.

The login-start action launches the normal application process hidden/background according to the approved preference; it does not install a service.

## 8. Manual updates

The MVP performs no network update check.

Update workflow:

1. Explicitly Quit OpenManic.
2. Replace the executable with the new Windows artifact.
3. Launch it against the existing `OpenManicData` directory or selected location.
4. Run compatible migrations with the backup/recovery policy.

The app MUST detect an attempted downgrade to a binary that cannot understand the newer database and fail into recovery rather than modifying it.

The documentation SHOULD recommend keeping the previous executable until the new version opens the store successfully. Store backup, not executable rollback alone, is the protection for destructive migrations.

## 9. Data-directory move

The user may select a new directory from Settings. The operation follows the coordinated move in [Data model](data-model.md).

User experience:

- Show source and destination.
- Validate before starting.
- Display progress and Cancelling state where safe.
- Keep the source intact until destination verification succeeds.
- Atomically update the locator only after verification.
- Resume tracking only after the new writer is ready.
- On failure, reopen the original store and explain what happened.

Moving a live SQLite file with ordinary Explorer copy is not the supported in-app move path.

## 10. Windows build profile

Release builds SHOULD:

- Use the pinned Rust toolchain and committed lockfile.
- Compile only `platform-windows` and the selected renderer.
- Bundle SQLite with explicit features.
- Exclude `dev-tools`, profiler UI, Linux display backends, and unused renderer code.
- Embed version, git revision, architecture, schema compatibility, and renderer metadata for diagnostics.
- Embed application icon/version resources.
- Use panic unwinding according to the reliability policy.
- Produce checksums alongside downloadable artifacts even though the app itself is offline.

Code signing is strongly desirable for Windows trust/SmartScreen experience but is an operational release decision, not a runtime dependency.

## 11. NixOS post-Windows target

### 11.1 Supported environment

Initial planned support:

```text
NixOS 26.05
x86-64
stock Sway 1.11 Wayland session
direct Sway IPC focus adapter
```

The packaging spike MUST verify the stock Sway version and entire closure on the target release. Generic Wayland, GNOME, KDE, and Hyprland are not included in this support claim.

### 11.2 Avoid a flake-required workflow

The end-user setup SHOULD NOT require a flake. The preferred release is a downloaded self-contained artifact.

If a Nix expression is required as a fallback, prefer a conventional package/derivation workflow that does not require the user to enable flakes. A flake MAY be used internally only if the build pipeline cannot reasonably avoid it; it must not become the required install/run interface without a later owner decision.

### 11.3 Artifact candidates

Evaluate in this order:

1. Nix-built self-contained AppImage/closure artifact.
2. Another single-file Nix closure bundle with acceptable startup behavior.
3. Conventional non-flake Nix package/derivation fallback.

Reject as release baseline without evidence:

- Raw conventionally linked ELF that assumes FHS loader/library paths.
- Ordinary FHS-built AppImage assumed to work on NixOS.
- Experimental bundle with unacceptable cold-start overhead.
- A package that silently writes into the immutable Nix store.

The owner accepts the normal one-time Linux action of marking a downloaded artifact executable.

### 11.4 Data location under NixOS

- A self-contained AppImage derives its artifact parent from the original artifact path (for example the `APPIMAGE` environment value), not its read-only mounted image path.
- A Nix-store executable cannot write data beside itself.
- When the parent is read-only, OpenManic opens a directory chooser and stores only the bootstrap locator in per-user XDG configuration.
- All substantive data remains under the chosen root.

### 11.5 Required packaging spike

On a clean offline NixOS VM:

- Test launch with no development packages.
- Test FUSE/user namespaces enabled and disabled.
- Test downloaded file before/after executable permission.
- Test Wayland and X11 eframe features.
- Compare WGPU and Glow with AMD, Intel, and NVIDIA where available.
- Measure artifact/closure size, cold/warm shell time, idle memory/CPU, and timeline frame time.
- Test Sway IPC reconnect and native Wayland/Xwayland applications.
- Test tray behavior under a realistic StatusNotifier host.
- Test custom data location, artifact move, and read-only parent recovery.
- Test optional session autostart without assuming every minimal Sway environment processes XDG autostart files.

The one-file NixOS experience is not advertised until this spike passes. Failure to make it reliable triggers the conventional non-flake package fallback and a documented directory chooser.

## 12. Backup and portability expectations

- Copying a fully closed `OpenManicData` directory is a portable data move/backup.
- While OpenManic is running, use the in-app SQLite backup function.
- CSV is interchange, not full backup.
- A data directory is never placed on a network share in the MVP.
- If removable storage is selected, warn about disconnect risk and verify locking.
- The executable may be moved independently when the default beside-artifact data folder moves with it or a custom locator remains valid.

## 13. Setup acceptance checks

Windows release:

- Runs on a clean supported Windows 11 x86-64 test system without installing a runtime/database.
- Creates data beside the executable when writable.
- Prompts for a location and remembers only the locator when not writable.
- Opens the existing instance on a second launch.
- Continues tracking while hidden.
- Explains first close-to-tray behavior.
- Can enable/disable login start without elevation.
- Survives executable replacement with store migration/backup.
- Makes no network request in first launch or normal use.

Future NixOS release:

- Meets the declared Sway capability scope.
- Does not require a flake from the user.
- Has measured, documented one-file or fallback package behavior.
- Never attempts to write substantive data into the Nix store/mounted artifact.

## 14. Primary references

- [Windows Run/RunOnce behavior](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys)
- [`Shell_NotifyIcon`](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyicona)
- [SQLite online backup](https://www.sqlite.org/backup.html)
- [Nix store](https://nix.dev/manual/nix/2.28/store/index.html)
- [AppImage architecture](https://docs.appimage.org/reference/architecture.html)
- [AppImage environment variables](https://docs.appimage.org/packaging-guide/environment-variables.html)
- [`nix bundle`](https://nix.dev/manual/nix/2.24/command-ref/new-cli/nix3-bundle)
