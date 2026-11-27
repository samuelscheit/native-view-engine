# `nve-cli`

Developer CLI binary `nve` (`packages/tools/cli`, **Tools layer**) providing `doctor`, `inspect`, `bench`, `dev`, `pair`, `build`, `codegen`, and `docs`.

## Key Entry Points

- Binary `nve` (`src/main.rs`) subcommands: `doctor`, `inspect`, `bench`, `dev`, `pair`, `build`, `codegen abi [--check]`, `docs`
- Integration tests: `tests/docs_cli_tests.rs`, `tests/pair_live_ws.rs` (workspace architectural invariants live in `packages/tools/arch-invariants`)

```bash
cargo run --locked -p nve-cli -- codegen abi --check
cargo run --locked -p nve-cli -- doctor
```

## Dependency Invariants

- **Depends on**: core, backend, framework, and example crates to drive codegen, docs builds, benchmarks, and live dev/preview workflows.
- **Must NOT be depended on by**: any library crate under `packages/{core,backends,framework,modules}/`.

## Verification

```bash
cargo test --locked -p nve-arch-invariants
cargo test --locked -p nve-cli
```
