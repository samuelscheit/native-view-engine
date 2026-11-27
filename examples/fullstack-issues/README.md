# `examples/fullstack-issues`

Reference end-to-end fullstack application uniting NVE's sans-I/O CBOR HTTP RPC stack (`nve-rpc`, `nve-server`, `nve-http-hyper`), data-oriented query/mutation cache (`nve-query`, `nve-http-web`, `nve-query-dioxus`), progressive `<ActionForm<P>>`, and cross-platform `nve-ui` components.

## Layout

- [`api/`](./api) (`nve-example-issues-api`): pure-Rust `Procedure` contracts (`ListIssues`, `GetIssue`, `ListComments`, `CreateIssue`, `CloseIssue`, `RenameIssue`, `AddComment`), integer-keyed CBOR structs, open `Unknown(u32)` enums for mobile version skew, `FormDecode`, and static `IssueTag` invalidations (`wasm32-unknown-unknown` & `wasm32-wasip2` clean).
- [`client/`](./client) (`nve-example-issues-client`): 100% platform-agnostic Dioxus UI (`IssuesApp`, `IssuesWorkspace`) using `use_query`, optimistic `use_action::<CloseIssue>()`, `<ActionForm<CreateIssue>>` / `<ActionForm<RenameIssue>>` / `<ActionForm<AddComment>>`, and `<FormHiddenField>`.
- [`server/`](./server) (`nve-example-issues-server`): Hyper HTTP/1.1 / Axum-mountable server with dual `HttpOnly` Cookie + `Bearer` auth, fail-closed `CsrfPolicy`, opt-in `CorsPolicy` (`NVE_ISSUES_CORS_ORIGIN`), progressive `FormRoute`s (`303 See Other` / `422`), and optional static `/pkg/issues/*` WASM bundle serving (`NVE_ISSUES_WASM_DIR`).
- [`e2e/run_e2e.mjs`](./e2e/run_e2e.mjs): zero-dependency Headless Chrome CDP E2E suite verifying live `FetchTransport` edge cases, hydrated `HttpOnly` cookie auth, optimistic close, 0ms local + server `422` validation with draft preservation, cross-origin CORS preflight, and true 0-JS HTML `<form>` `303` fallback.

## Running Locally

```bash
# 1. Build the Web WASM bundle for the Issues app
./hosts/web-shell/build.sh --dev issues

# 2. Run the reference HTTP server serving both 0-JS SSR and the WASM bundle
NVE_ISSUES_WASM_DIR=hosts/web-shell/pkg/issues cargo run --locked -p nve-example-issues-server

# 3. Run the full native + live Headless Chrome CDP E2E suite
cargo test --locked -p nve-example-issues-server
node examples/fullstack-issues/e2e/run_e2e.mjs
```
