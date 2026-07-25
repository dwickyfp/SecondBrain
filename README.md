# SecondBrain

SecondBrain is a Rust-first project for a local-first knowledge system with Markdown as its durable representation.

This repository currently contains the initial Rust workspace and a smoke-tested `secondbrain-core` crate. Product features described in `docs/` are design and implementation targets, not completed functionality.

## Toolchain

The workspace pins Rust 1.91.1 with the `rustfmt` and `clippy` components through `rust-toolchain.toml`.

## Quality gates

Run the same checks used by CI from the repository root:

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo deny --locked check -D warnings
```

The dependency-policy gate uses `cargo-deny` 0.20.2. Install that exact version when it is not already available:

```bash
cargo install cargo-deny --version 0.20.2 --locked
```
