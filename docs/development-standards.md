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
- Validate external input at the boundary and represent valid values with dedicated types.
- Represent absence with `Option` and recoverable failure with `Result`.
- Keep public interfaces minimal.
- Keep provider-neutral contracts independent of concrete integrations; integrations depend on contracts, never the reverse.
- Classify values by lifetime before designing an interface: stable values belong to the object, per-operation values are parameters, derived values are not passed twice, and produced values are returned.
- Prefer direct parameters and return values. Introduce input structs, result wrappers, callbacks, sinks, channels, or streams only when a concrete current requirement justifies them.
- Prefer concrete implementations until multiple implementations create a demonstrated need for abstraction.
- Do not use unsafe Rust.

## Conventional Solutions

- Prefer conventional, idiomatic Rust that is readily understood by other Rust developers.
- Use standard-library types, traits, and functions when they adequately represent the problem. Standard-library facilities do not require prior design discussion.
- Do not introduce a custom type, abstraction, utility, or implementation merely to avoid a normal standard-library facility.
- Use strong domain types when they enforce a genuine domain distinction or invariant, not when they duplicate an established general-purpose type.
- Do not recreate established library functionality solely to avoid a dependency.
- Before adding a dependency, explain the capability missing from the standard library, identify the conventional crate, describe its maintenance and security cost, compare it with reasonable standard-library or local implementations, and discuss the choice.
- When requirements do not force a specialized representation, use the conventional representation.
- Document any persistent or external requirement for a specific unit, encoding, precision, or compatibility contract before introducing a custom type or dependency for it.

## Domain Types

- Make invalid domain values unrepresentable through normal construction.
- Keep domain fields private and provide constructors that enforce all invariants.
- Use `Result` and typed errors when construction can fail.
- Do not panic or assert when rejecting invalid external input.
- Preserve valid external values exactly unless normalization is explicitly part of the domain contract. Validation may inspect a normalized form, such as trimming to detect blank text, without changing a valid value.
- Treat deserialized data as untrusted. When derived Serde deserialization bypasses constructors, validate values while reconstructing the containing domain object.
- Use `From` and `from_*` only for infallible conversions. Use `TryFrom`, `try_*`, or another clearly fallible constructor for conversions that can fail.
- Put conversion knowledge on the containing or destination type. Inner domain types must not depend on outer systems that contain, persist, or transport them.
- Expose every contractually readable value through typed, immutable accessors. Do not add mutable accessors merely for convenience.
- Keep extension data readable to compatible consumers while preventing mutation.
- Test constructor invariants, persistence round trips, and reconstruction boundaries.

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
