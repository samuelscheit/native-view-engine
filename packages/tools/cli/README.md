# `nve-cli`

Developer CLI binary `nve` (`packages/tools/cli`, **Tools layer**) providing `doctor`, `inspect`, `bench`, `dev`, `pair`, `build`, `codegen`, `docs`, and the mechanical architectural invariant test suite (`tests/arch_invariants.rs`).

## Key Entry Points

- Binary `nve` (`src/main.rs`) subcommands: `doctor`, `inspect`, `bench`, `dev`, `pair`, `build`, `codegen abi [--check]`, `docs`
- Integration tests: `tests/arch_invariants.rs`, `tests/docs_cli_tests.rs`, `tests/pair_live_ws.rs`

```bash
cargo run --locked -p nve-cli -- codegen abi --check
cargo run --locked -p nve-cli -- doctor
```

## Dependency Invariants

- **Depends on**: core, backend, framework, and example crates to drive codegen, docs builds, benchmarks, and live dev/preview workflows.
- **Must NOT be depended on by**: any library crate under `packages/{core,backends,framework,modules}/`.

## Verification

```bash
cargo test -p nve-cli --test arch_invariants
cargo test -p nve-cli
```
