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
- Keep provider-neutral contracts independent of concrete integrations; integrations depend on contracts, never the reverse.
- Classify values by lifetime before designing an interface: stable values belong to the object, per-operation values are parameters, derived values are not passed twice, and produced values are returned.
- Prefer direct parameters and return values. Introduce input structs, result wrappers, callbacks, sinks, channels, or streams only when a concrete current requirement justifies them.
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

## Design Revision

- Treat explored ideas and future possibilities as non-requirements until they are explicitly accepted.
- Require each nontrivial abstraction to trace to a current requirement or accepted invariant.
- When feedback changes ownership, lifetime, output hierarchy, or failure semantics, restate the complete boundary before editing further.
- Remove abstractions that no longer follow from the revised model instead of preserving them through additional wrappers or compatibility layers.
- When an accepted simplification gives up an earlier property, remove that superseded property from code, tests, and authoritative documentation.

## Required Checks

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```
