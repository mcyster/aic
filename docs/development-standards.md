# Development Standards

The root [`AGENTS.md`](../AGENTS.md) translates these standards into concise instructions for coding agents.

## Priorities

1. Readability and understandability
2. Correctness through strong types
3. Simplicity
4. Performance supported by evidence

## Source Code

- Use descriptive, full-word names.
- Do not use single-letter names for project-defined variables, parameters, types, or generic parameters.
- Avoid abbreviations in project-defined names.
- Prefer standard Rust terminology where it is part of the language or ecosystem.
- Express intent through names and small, focused entities rather than source comments.
- Normalize external input at the boundary and represent validated values with dedicated types.
- Represent absence with `Option` and recoverable failure with `Result`.
- Keep public interfaces minimal.
- Prefer concrete implementations until multiple implementations create a demonstrated need for abstraction.
- Do not use unsafe Rust.
- Add dependencies only when they provide clear value over the standard library.

## Project Structure

- Organize code by feature or responsibility rather than by technical layer.
- Keep the executable entry point thin.
- Keep durable documentation in `docs/`.
- Record consequential architectural decisions in `docs/decisions/`.
- Keep unfinished proposals in `docs/ideas.md` until they are accepted or removed.
- Introduce a Cargo workspace only when the project has multiple independently useful packages.

## Required Checks

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```
