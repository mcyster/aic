# aic

`aic` provides command-line access to agentic services.

The first command invokes a stubbed conversation turn:

```console
aic turn "Explain ownership in Rust"
```

## Development

Enter the development environment with direnv:

```console
direnv allow
```

Alternatively, enter it directly:

```console
nix develop
```

Run the project checks:

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Detailed project documentation is in [`docs/`](docs/README.md).
