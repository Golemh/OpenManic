# Native UI fixture measurement procedure

`native-ui-fixture` is an isolated eframe renderer-comparison harness. It uses
the deterministic OM-030 fixture generator and never represents its results as
OpenManic product performance or release evidence by itself.

## Build separate renderer artifacts

Build each renderer into a different target directory. Never enable both
renderer features in one process or substitute the other renderer after a
failure.

```powershell
$env:CARGO_TARGET_DIR = "target/native-ui/wgpu"
cargo build -p native-ui-fixture --release --no-default-features --features renderer-wgpu --locked

$env:CARGO_TARGET_DIR = "target/native-ui/glow"
cargo build -p native-ui-fixture --release --no-default-features --features renderer-glow --locked
```

Record the byte size and SHA-256 of each `.exe` with the exact external command
used. The JSONL output also records a stable `fnv1a64` content checksum only as
an association aid; it is not a cryptographic artifact identity.

## Named hardware manifest

Copy `reference-hardware-manifest.template.json` to an untracked, machine-
specific path. Fill every field before running a release-evidence campaign. The
fixture reports the path basename and checksum, while the full manifest and
measurement outputs remain outside source control unless a later release task
explicitly requests a compact report.

Results with a missing field, VM, headless session, debug-like build, or unnamed
hardware are diagnostic observations only.

## Cold and warm launch campaigns

1. Use one named physical Windows 11 reference machine, display mode, power
   mode, driver, security configuration, and release artifact for a campaign.
2. For cold runs, record the approved cache-control method in the manifest. A
   restart-and-settle protocol or a separately identified cache-control tool is
   acceptable; if caches cannot be controlled, label the results warm-only
   diagnostic rather than cold.
3. For warm runs, first prime the executable and common libraries. Then launch
   a new process for each sample; do not reuse an already-running fixture.
4. Run the dense fixture for at least 30 launch samples and 300 post-warm-up
   frames per renderer. Repeat representative `normal-workday`,
   `three-segmented-bands`, `schedule-dst-overnight`, and
   `simultaneous-overlays` runs to verify the selected fixture path.
5. Pass the git revision, lockfile hash, and named-hardware manifest reference
   to every invocation. Choose a fresh output file for each process because the
   fixture never overwrites a report.

```powershell
target/native-ui/wgpu/release/native-ui-fixture.exe `
  --scenario dense-10000-interval-range `
  --seed 2026030 `
  --frames 360 `
  --warmup-frames 60 `
  --git-revision <full-commit-sha> `
  --lockfile-hash <sha256-or-recorded-hash> `
  --environment-manifest <absolute-untracked-manifest-path> `
  --output <absolute-untracked-report-path>
```

The first frame records `fixture_shell_ready_ns`: elapsed from fixture `main`
to its first painted eframe update. This is intentionally not the product
usable-shell measurement defined in the performance specification.

`ui_cpu_ns` covers native-fixture UI layout, interaction, and dense paint
preparation. `observed_frame_cadence_ns` is start-to-start eframe update
cadence, which includes event-loop, renderer submission, pacing, and scheduling
but is not direct GPU completion timing. Do not relabel either metric.

## Memory observation hooks

The report emits deterministic checkpoints named `shell_ready`,
`idle_after_warmup`, and `dense_interaction`. The fixture does not link a
process-inspection dependency, so each record explicitly says `not_collected`.
Use a documented external Windows working-set/RSS tool at those checkpoints and
add its version, command, units, and values to the machine-specific manifest.

## JSONL interpretation

Every line has `schema_version: 1` and a `record` discriminator. Relevant
records are `run`, `environment`, `artifact`, `fixture`, `shell_ready`, `frame`,
`memory_checkpoint`, `summary`, and `outcome`.

The `outcome` record is `renderer_failure` when the selected renderer/event loop
returns an error. It always records `fallback_attempted: false`; compare WGPU
and Glow only through their separately built executions.

For each metric, discard the stated warm-up frames and sort the remaining valid
samples. Use nearest-rank percentiles with zero-based index `ceil(p * n) - 1`.
The report emits p50/p95 using that method and names the sample count. Do not
combine incomplete, failed, or different-environment runs.
