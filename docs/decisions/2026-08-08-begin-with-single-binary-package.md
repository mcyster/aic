# Begin with a Single Binary Package

## Why

The project starts with one executable and one conversation command. Its provider and transport requirements are not yet known.

## Decision

Use one Cargo package containing one binary target. Organize its code into modules by responsibility. Do not create a library target, workspace, or provider abstraction until a concrete requirement establishes the need.

## Consequences

The initial structure stays easy to navigate. Reusable library or package boundaries can be introduced later based on observed responsibilities.
