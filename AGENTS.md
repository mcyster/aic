# Repository Guidelines

## Priorities

Apply these priorities in order:

1. Readability and understandability
2. Correctness through strong types
3. Simplicity
4. Performance supported by evidence

The complete development standards are in [`docs/development-standards.md`](docs/development-standards.md). Follow them for every change.

## Rust Source

- Use descriptive, full-word names for project-defined types, functions, modules, variables, parameters, and generic parameters.
- Do not use project-defined abbreviations or single-letter names.
- Standard Rust and domain terminology such as `std`, `str`, `Ok`, `Err`, `Self`, and `tog` is allowed.
- Do not add comments to Rust source files. Extract complexity into well-named entities instead.
- Normalize external values at boundaries and convert them into validated project types.
- Use `Option` for absence and `Result` for recoverable failure.
- Keep public interfaces minimal.
- Keep provider-neutral contracts independent of concrete integrations; integrations depend on contracts, never the reverse.
- Classify values by lifetime before designing an API: object-stable values belong to the object, per-operation values are parameters, derived values are not passed twice, and produced values are returned.
- Prefer direct parameters and return values. Add input structs, result wrappers, callbacks, sinks, or streams only for a concrete current requirement.
- Prefer concrete implementations. Introduce traits and generic abstractions only when a demonstrated need exists.
- Do not use unsafe Rust.
- Prefer the standard library. Add a dependency only when its value justifies its maintenance and security cost.
- Follow idiomatic Rust unless it conflicts with an explicit repository standard.

## Structure

- Keep `src/main.rs` limited to application composition and process input or output.
- Organize modules by feature or responsibility, not by generic technical layers.
- Keep the project as one binary package until a concrete requirement justifies a library target or workspace.
- Keep durable documentation in `docs/`.
- Record consequential decisions in `docs/decisions/`.
- Keep unaccepted proposals in `docs/ideas.md`.

## Change Discipline

- Make the smallest complete change that satisfies the requirement.
- Treat exploratory ideas and future possibilities as non-requirements until explicitly accepted.
- Do not add speculative compatibility, configuration, abstractions, or extension points.
- When feedback changes ownership, lifetime, output hierarchy, or failure semantics, restate the complete boundary and remove invalid abstractions instead of layering another type onto them.
- When a simpler accepted direction gives up an earlier property, remove the superseded code, tests, and documentation rather than preserving it implicitly.
- Preserve existing behavior unless the task explicitly changes it.
- Add or update tests for observable behavior and validation rules.
- Do not commit or push unless explicitly requested.

## Required Validation

Run all checks inside the Nix development environment:

```console
nix develop --command cargo fmt --check
nix develop --command cargo clippy --all-targets --all-features -- -D warnings
nix develop --command cargo test --all-targets --all-features
```
