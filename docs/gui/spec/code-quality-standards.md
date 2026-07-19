# OpenManic code quality and readability standard

- Status: MVP implementation standard
- Applies to: all Rust crates, tests, build support, migrations, and repository tooling

## 1. Purpose

This document turns code hygiene into an enforceable part of the architecture. Its goals are to make defects visible early, keep product concepts easy to find, and let a new contributor understand a change without first learning private conventions.

`MUST`, `MUST NOT`, `SHOULD`, and `MAY` are normative. A tool setting is not a substitute for judgment: automated checks establish the minimum bar, while reviews enforce ownership, clarity, and architectural fit.

## 2. Quality principles

1. **Readable ownership:** each product rule has one obvious owning module.
2. **Automated consistency:** formatting, linting, tests, documentation checks, and dependency policy run through one Rust-based entry point.
3. **Loud failure with graceful containment:** invalid states fail close to their source; recoverable runtime failures remain typed and user data is preserved.
4. **Narrow exceptions:** a suppression documents why the general rule does not fit this one expression, item, or module.
5. **Pinned behavior:** the Rust toolchain and dependency graph are pinned so a normal quality run is reproducible.
6. **No runtime tax:** development-only quality tools MUST NOT be linked into the OpenManic release artifact.

## 3. Required repository tooling

The repository root MUST contain:

```text
.cargo/config.toml       Cargo aliases, including `cargo xtask`
.editorconfig            Basic editor-independent whitespace rules
clippy.toml              Small, pinned Clippy configuration
deny.toml                Advisory, duplicate, and source policy
rust-toolchain.toml      Exact tested stable toolchain and components
rustfmt.toml             Stable rustfmt policy
tools/xtask/             Rust-only quality and repository automation
```

`rust-toolchain.toml` MUST pin an exact stable release and install `rustfmt` and `clippy`. Toolchain updates are reviewed changes and MUST pass the complete quality suite before merge.

The repository MUST NOT require Make, Just, Python, Node.js, or a shell-specific script for ordinary build and quality tasks. Native platform packaging MAY use platform tools when the delivery specification requires them, but the common developer path remains Cargo plus the pinned Rust toolchain.

## 4. Formatting

All hand-written Rust uses stable rustfmt. The initial configuration is:

```toml
edition = "2024"
style_edition = "2024"
max_width = 100
newline_style = "Unix"
use_field_init_shorthand = true
use_try_shorthand = true
```

Requirements:

- CI MUST run `cargo fmt --all -- --check`.
- Contributors run rustfmt rather than manually aligning code.
- Only stable rustfmt options are permitted. A nightly-only formatting option would make editor and CI output less predictable.
- `#[rustfmt::skip]` is limited to generated code or syntax whose shape is itself tested. A nearby comment MUST state why formatting would damage readability or correctness.
- `.editorconfig` MUST use UTF-8, final newlines, spaces, and trim trailing whitespace. Rust indentation is four spaces; Markdown MAY preserve intentional trailing spaces.

## 5. Workspace lint policy

The virtual workspace manifest owns the baseline. Every workspace member, including tools, MUST opt in with:

```toml
[lints]
workspace = true
```

The initial root policy is:

```toml
[workspace.lints.rust]
rust_2018_idioms = { level = "warn", priority = -1 }
missing_docs = "warn"
unreachable_pub = "warn"
unsafe_op_in_unsafe_fn = "deny"
unused_must_use = "deny"

[workspace.lints.rustdoc]
broken_intra_doc_links = "deny"
private_intra_doc_links = "deny"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
dbg_macro = "deny"
undocumented_unsafe_blocks = "deny"
allow_attributes = "deny"
allow_attributes_without_reason = "deny"
wildcard_imports = "warn"
excessive_nesting = "warn"
cognitive_complexity = "warn"
too_many_lines = "warn"
```

The `all` and `pedantic` groups are enabled. The complete `restriction` and `nursery` groups MUST NOT be enabled as groups: restriction lints are intended to be selected individually and can contradict each other, while nursery lints are not yet considered stable enough for a blanket policy.

CI converts warnings into failures using:

```powershell
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Because the toolchain is pinned, this strict gate changes only during a deliberate toolchain update. The update change fixes new findings or records narrow justified expectations before it lands.

### 5.1 Clippy configuration

The initial `clippy.toml` is:

```toml
allow-expect-in-consts = false
allow-expect-in-tests = true
allow-unwrap-in-consts = false
allow-unwrap-in-tests = false
allow-dbg-in-tests = false
cognitive-complexity-threshold = 20
too-many-lines-threshold = 80
type-complexity-threshold = 200
excessive-nesting-threshold = 4
doc-valid-idents = [
    "OpenManic",
    "egui",
    "eframe",
    "SQLite",
    "Win32",
    "Wayland",
    "Sway",
    "AUMID",
    "WGPU",
    "Jiff",
    "..",
]
```

The complexity, nesting, and line thresholds are refactoring signals, not claims that every longer function is defective. The configuration stays small because Clippy configuration compatibility follows the pinned toolchain and can evolve between releases.

### 5.2 Unsafe Rust

The domain, application, storage, UI, and composition crates MUST declare:

```rust
#![forbid(unsafe_code)]
```

The platform crate MAY use unsafe Rust only inside private adapter/FFI modules where a safe API is not available. Each unsafe block MUST have a `// SAFETY:` comment naming the preconditions, lifetime or ownership facts, and thread requirements that make the operation valid. Safe wrappers MUST keep raw handles and pointers from crossing the adapter boundary.

The platform crate still denies `unsafe_op_in_unsafe_fn` and `clippy::undocumented_unsafe_blocks`. An `unsafe fn` does not make operations inside it implicitly acceptable.

## 6. Lint exception policy

A lint exception is acceptable only after the code has been made as clear as practical.

- Prefer `#[expect(lint_name, reason = "...")]` over `#[allow(...)]`.
- Apply the expectation to the smallest expression or item that needs it.
- The reason MUST explain why this instance is clearer, safer, or required by an external API; repeating the lint name is not a reason.
- A crate- or module-wide exception requires an architectural reason such as generated bindings or a contained FFI surface, plus a tracking issue when it is temporary.
- `cfg_attr` MAY scope an expectation to one supported target.
- Unfulfilled expectations are failures under the warnings-as-errors policy, which removes obsolete suppressions.
- Generated code is excluded at the generator boundary, not through broad exceptions in hand-written modules.

Example:

```rust
#[expect(
    clippy::too_many_arguments,
    reason = "Win32 callback signature is fixed by the operating-system ABI"
)]
unsafe extern "system" fn foreground_hook(/* ABI parameters */) {
    // ...
}
```

`#[allow(...)]` is reserved for generated code or for a compiler compatibility case where `#[expect]` cannot express the required scope. It MUST include an adjacent explanation.

## 7. Naming and module readability

Names MUST use the product vocabulary in the specification.

- Modules are product concepts or specific technical boundaries: `activity`, `schedule`, `projection`, `windows`, `sqlite`.
- Avoid vague containers such as `common`, `core`, `helpers`, `manager`, `misc`, `processor`, `stuff`, or `utils`. A qualified name such as `schedule_validation` is acceptable when it states ownership.
- Types are nouns; operations are verbs; predicates read affirmatively, such as `is_tracking_enabled`.
- Include units in a name or, preferably, in a newtype: `FrameBudget`, `UtcMicros`, `RetryDelay`.
- Replace multiple Boolean parameters with an options struct or an enum whose variants name the behavior.
- Do not use one-letter names outside tiny mathematical scopes, indices, or conventional closures where meaning is immediate.
- Public APIs use the narrowest useful visibility. Prefer `pub(crate)` until another crate has a demonstrated need.
- Production modules MUST NOT use wildcard imports. Tests MAY use a local `use super::*` when the tested surface remains obvious.

`lib.rs` and `mod.rs` files act as small facades. They SHOULD expose ownership and important types, not contain large implementations. A file is split when doing so creates distinct concepts or makes navigation and testing clearer; there is no arbitrary maximum file length.

## 8. Function and control-flow clarity

- A function has one describable responsibility and one primary level of abstraction.
- Prefer early returns and explicit state transitions over deeply nested branches.
- A nesting depth above four, cognitive complexity above 20, or a function above roughly 80 lines triggers review and usually extraction. A justified `#[expect]` is valid when keeping an algorithm together is clearer.
- Use enums and exhaustive matches for lifecycle, adapter capability, and activity states. Do not encode state machines as loosely related Boolean fields.
- Validate at the boundary and pass validated types inward. Do not repeat stringly typed validation across screens and workers.
- Iterator chains are useful while their order and failure behavior remain obvious. Split a dense chain into named steps when it becomes difficult to inspect or debug.
- Avoid hidden work in conversions. A `From` implementation MUST be infallible and unsurprising; use `TryFrom` or a named operation for validation, I/O, allocation-heavy work, or policy decisions.
- No mutable global state or service locator is permitted. Ownership and shared state are explicit in constructor parameters and wiring.

## 9. Comments and Rust documentation

Comments explain intent, invariants, ownership, trade-offs, or external constraints. They MUST NOT merely translate the next line into English.

Each library crate MUST begin with `//!` documentation describing:

1. what the crate owns;
2. what it deliberately does not own;
3. its allowed dependency direction; and
4. its threading or persistence assumptions when applicable.

Every public item needs useful `///` documentation. Public fallible APIs document `# Errors`; deliberate panic conditions document `# Panics`; unsafe APIs document `# Safety`. Small pure public APIs SHOULD include compilable examples. Prefer intra-doc links over duplicated type names.

Non-obvious concurrency protocols and state machines require module-level documentation and an invariant comment beside the authoritative state. SQL migrations describe the invariant they establish and any backfill behavior.

Documentation is checked with warnings denied:

```powershell
cargo test --workspace --locked
cargo doc --workspace --no-deps --locked
```

The default workspace test command includes library documentation tests. The `xtask` runner sets `RUSTDOCFLAGS=-D warnings` portably for the documentation command.

## 10. Errors, assertions, and failure behavior

Production code MUST NOT use `.unwrap()`, `.expect()`, `panic!`, `todo!`, or `unimplemented!` as routine error handling. Fallible boundaries return the typed crate errors defined in [Project structure](project-structure.md) and attach safe diagnostic sources.

Assertions remain required for internal invariants and preconditions whose violation is a programming defect:

- `debug_assert!` checks expensive or development-focused invariants.
- `assert!` is appropriate when continuing could corrupt state or silently produce invalid user data.
- Assertion messages state the invariant and include non-sensitive identifiers needed to locate the defect.
- A failure on a worker thread is still caught at its supervisor boundary, reported, and followed by the graceful-shutdown or degraded-mode behavior in [Performance and reliability](performance-and-reliability.md).

Tests MAY use `.expect("fixture or invariant-specific message")` because Clippy permits it in test code. Tests MUST NOT use `.unwrap()`; a descriptive expectation makes a fixture failure diagnosable. Tests use `assert_eq!` and related assertions when they produce clearer failure output than a Boolean assertion.

## 11. Concurrency, UI, and persistence hygiene

The architectural boundaries are also review rules:

- The egui update path MUST NOT perform SQLite access, platform calls, filesystem I/O, blocking waits, or unbounded computation.
- Channels are bounded and their overflow/backpressure behavior is documented at construction.
- A lock guard MUST NOT be held across database, filesystem, operating-system, callback, sleep, or channel-wait operations.
- If multiple locks are unavoidable, their acquisition order is documented once and tested where practical.
- Platform callbacks copy the minimum evidence into preallocated or bounded ingress and return immediately.
- Database transactions are short, have one named owner, and never call back into UI or platform code.
- Cancellation and shutdown paths are explicit and covered by tests; dropping a sender or thread handle is not an undocumented control protocol.
- Logs and errors follow the privacy rules: rapidly changing window titles are excluded from ordinary diagnostics.

## 12. Test hygiene

Tests are deterministic, behavior-oriented, and located at the narrowest useful layer.

- Test names state the behavior and condition, for example `closing_active_interval_uses_shutdown_timestamp`.
- Domain state transitions and boundary values use table-driven or property tests where they improve coverage.
- Time-dependent code receives a clock or explicit timestamp. Ordinary tests MUST NOT depend on wall-clock timing or arbitrary sleeps.
- Platform and storage adapters use contract tests shared across implementations where possible.
- SQLite tests use isolated temporary databases and exercise real migrations and constraints.
- UI tests operate on actions, view models, snapshots, and pure layout calculations. A small number of native smoke tests cover composition.
- Bug fixes add a regression test that fails for the original behavior unless the failure cannot be reproduced deterministically; that exception is explained in the change.
- Test fixtures are small and named. Large performance datasets are produced by the Rust fixture generator from committed configuration.
- Tests do not access the network. Development advisory checks are separate from application tests.

The MVP does not impose a coverage percentage. Changed behavior requires meaningful tests, and coverage trends MAY be used to find blind spots without rewarding low-value assertions.

## 13. Dependency hygiene

- Workspace dependencies have one reviewed version in `[workspace.dependencies]`; members inherit it.
- `Cargo.lock` is committed and `--locked` is used in CI and release checks.
- Default features are disabled for platform-sensitive or large dependencies unless each enabled feature is intentional.
- Platform dependencies use target-specific Cargo tables so Windows artifacts do not compile Linux stacks and vice versa.
- A Git dependency requires an exact revision, a documented reason, and an exit plan. Published crates are preferred.
- New build scripts, proc macros, native libraries, or dependencies that add background runtimes receive explicit review.
- `cargo tree -d` and `cargo tree -e features` are reviewed before release to find duplicate versions and accidental features.
- `cargo deny check advisories bans sources` is a required CI check. Its advisory database is refreshed in CI; this has no effect on the application's offline runtime.
- License data MAY be inventoried, but licenses are not an MVP quality gate because the product decision currently sets no license restriction. Adding a license gate requires an explicit policy decision.

## 14. Rust-only quality runner

`tools/xtask` is a small workspace binary. `.cargo/config.toml` defines:

```toml
[alias]
xtask = "run --package xtask --"
```

Required commands:

| Command | Purpose |
| --- | --- |
| `cargo xtask quality` | Run the ordinary format, check, lint, test, doc, and specification-link gates using only the pinned Rust toolchain |
| `cargo xtask docs-check` | Validate internal Markdown links, required headings, Mermaid fences, and normative cross-references under `docs/gui` |
| `cargo xtask dependency-check` | Invoke the separately pinned `cargo-deny` tool for advisory, duplicate, and source policy |
| `cargo xtask release-check` | Run quality plus dependency policy, the supported feature matrix, release build, artifact-size report, and platform smoke-test prerequisites |

The runner prints each exact Cargo command before executing it and returns the first failing exit status. It MUST NOT install tools, rewrite source, access user application data, or silently apply fixes. An optional `cargo xtask fix` MAY exist for explicit local use, but CI only checks.

Ordinary quality expands to at least:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo xtask docs-check
```

`cargo check` and Clippy compile benchmark targets through `--all-targets`; the ordinary test stage does not execute them. Performance benchmarks belong to the scheduled and release checks in Section 15.

The runner sets environment variables through `std::process::Command`, so the workflow behaves the same from PowerShell and POSIX shells.

`cargo-deny` is a development/CI executable rather than a Rust toolchain component. CI MUST install an explicitly pinned version and run `cargo xtask dependency-check`; the subcommand reports a clear installation command when the executable is absent. It is intentionally outside ordinary `cargo xtask quality`, preserving the one-step fresh-checkout workflow while keeping dependency policy mandatory for CI and releases.

Feature checks MUST enumerate supported combinations rather than using `--all-features`, because renderer and platform selections intentionally cannot all form one artifact. For the Windows MVP, both WGPU and Glow builds are checked separately, while the selected release renderer receives the complete test and smoke-test path.

## 15. Scheduled and release-only checks

These checks are valuable but too slow or platform-specific for every edit:

- Run Miri on pure domain/application crates with the pinned compatible nightly in a scheduled job. Miri does not replace Windows FFI tests and MUST NOT be treated as validation of unsupported system APIs.
- Run performance fixtures and compare the budgets in [Performance and reliability](performance-and-reliability.md).
- Build the Windows portable release artifact and record compressed and unpacked size.
- Exercise single-instance, tray, autostart, suspend/resume, shutdown, and database-recovery scenarios on a Windows 11 runner.
- Compile the alternative renderer and supported target feature matrices.
- Review dependency duplicates, enabled features, advisories, and sources.

Nightly tools are isolated from the ordinary stable build. Their failure does not justify weakening stable compiler or Clippy checks; it is triaged according to whether the issue is in OpenManic, the tool, or an unsupported boundary.

## 16. Review checklist

Every material change answers these questions:

- Is the product rule in the owning module, with no frontend/backend boundary violation?
- Can names and file placement be understood using terms from the specification?
- Are invalid states prevented by types or validated once at a boundary?
- Are errors typed, actionable, privacy-safe, and covered by graceful failure behavior?
- Does new concurrency have bounded queues, explicit ownership, and a tested shutdown path?
- Does UI work stay within the frame path budget and avoid blocking calls?
- Are public APIs minimal and documented, and are comments explaining why rather than what?
- Are lint expectations narrow and genuinely justified?
- Do tests cover success, boundary, failure, and regression behavior without sleeps or network access?
- Are dependency features, target scope, and release-size impact understood?
- Do `cargo xtask quality` and the relevant platform/feature checks pass?

Reviewers SHOULD request a rename or module move when discoverability is poor even if the code compiles. Readability is an acceptance condition, not optional polish.

## 17. Acceptance criteria

The code-quality standard is implemented when:

- a fresh checkout needs only the pinned Rust toolchain for normal checks;
- all workspace crates inherit the root lint table;
- formatting and warnings-as-errors checks are reproducible;
- unsafe code is impossible outside the private platform boundary;
- every lint suppression has a narrow scope and reason;
- `cargo xtask quality` exposes and runs the complete ordinary gate;
- documentation links and rustdoc warnings fail CI;
- tests avoid network, wall-clock dependence, and opaque unwrap failures;
- dependency advisories, duplicate policy, and sources are checked without adding runtime dependencies; and
- a contributor can locate a product concept by its specification name rather than searching generic manager/helper modules.

## 18. Primary references

- [Cargo workspaces and workspace lint inheritance](https://doc.rust-lang.org/stable/cargo/reference/workspaces.html)
- [Cargo manifest lint configuration and priorities](https://doc.rust-lang.org/cargo/reference/manifest.html#the-lints-section)
- [Clippy usage and lint-group guidance](https://doc.rust-lang.org/clippy/usage.html)
- [Clippy lint configuration options](https://doc.rust-lang.org/stable/clippy/lint_configuration.html)
- [Rust compiler lint levels and expectations](https://doc.rust-lang.org/stable/rustc/lints/levels.html)
- [Rust diagnostic attributes and lint reasons](https://doc.rust-lang.org/reference/attributes/diagnostics.html)
- [rustfmt configuration and style editions](https://github.com/rust-lang/rustfmt)
- [rustdoc lints](https://doc.rust-lang.org/rustdoc/lints.html)
- [rustdoc documentation tests](https://doc.rust-lang.org/rustdoc/documentation-tests.html)
- [Cargo test target selection](https://doc.rust-lang.org/cargo/commands/cargo-test.html#target-selection)
- [cargo-deny checks](https://embarkstudios.github.io/cargo-deny/checks/index.html)
- [Miri](https://github.com/rust-lang/miri)
