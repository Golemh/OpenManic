# Contributing to OpenManic

Thanks for wanting to help. OpenManic is maintained by two people, so a short
issue before a big pull request saves everyone time.

## Before your first pull request

We ask every contributor to sign a Contributor License Agreement (CLA) once.
In short:

- You keep the copyright on what you write.
- You give us the right to use, change and relicense your contribution,
  including under the commercial license described in the README.
- Contributions are unpaid. If we ever earn money from a commercial license,
  it goes to the two maintainers, and we'll say publicly that it happened.
- You confirm the code is yours to give, not your employer's or copied from
  somewhere else.

Why we do this: OpenManic is GPL, so anyone can use it for free. The CLA is
what lets us also sell a closed-source license to a company that asks for one,
without having to chase down every past contributor first.

## Building and testing

OpenManic runs on Windows 11. The Rust toolchain is pinned in
`rust-toolchain.toml`, and nothing else needs installing.

```
cargo run --release        # build and start the app
cargo xtask quality        # what has to pass before a PR is merged
```

`quality` runs rustfmt, `cargo check`, clippy with warnings as errors, the
tests, and rustdoc. The lints are strict on purpose: no `unwrap` anywhere, no
`expect` or `panic!` outside tests, every public item documented, and every `unsafe`
block carrying a `// SAFETY:` comment.

## Where things go

The crate layout and the rule behind it are in the README. In short: the UI
crate only draws snapshots and sends actions, so anything that touches the
database or Windows belongs in `openmanic-storage-sqlite` or
`openmanic-platform`. For bigger changes, read the matching spec in
[`docs/gui/spec/`](docs/gui/spec/README.md) first.

Tests have to be deterministic: no network, no sleeps, no real wall-clock time.
