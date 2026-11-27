# rust-analyzer: string-parsed proc-macro tokens get out-of-file spans

**Verdict: already reported** — https://github.com/rust-lang/rust-analyzer/issues/22482 (OPEN, `C-bug`).
"Panics with "invalid offset" errors (code: -32603) on "Go to definition" calls". A 2026-11-25 comment
by RedKinda gives a minimal repro (`#[proc_macro]` returning `format!("{}pub struct …", "\n".repeat(5000)).parse()`)
on 0.3.3130; same root cause as ours. No fixing PR found.

**Genuine bug**: `proc_macro` docs (`impl FromStr for TokenStream`): "All tokens in the parsed stream get
`Span::call_site()` spans." rust-analyzer master (efc6ca7, 2026-11-27) `crates/proc-macro-api/src/token_stream.rs`
`TokenStream::from_str` calls `span.derive_ranged(range)`; `impl SpanLike for span::Span` keeps the call-site
`anchor`/`ctx` but replaces `range` with the token's byte offset inside the parsed string → ranges past the
call site / EOF → `line-index` `invalid offset` panics in `to_proto::position` (goto-definition, codeLens/resolve impls lens).
Present since at least 2025-12 (`crates/proc-macro-srv/src/token_stream.rs` @ c3afa44); still on master.
(The trait's own doc says it exists "for allowing tests to parse tokenstreams with dynamic span ranges".)

**Related (same symptom, unconfirmed cause)**: #22161 (open), #18881 (open).

**Comment on #22482 would add**: (1) exact root cause + file/function and the documented contract it violates,
(2) a derive-macro trigger via `codeLens/resolve` (impls lens), not only goto-definition, (3) still on
0.4.3133 (9c74e78e01, 2026-11-23) and master efc6ca7, (4) suggested fix: in `from_str` use the call-site span
unchanged for `span::Span` (derive_ranged = identity, like the `SpanId` impl), keeping ranged spans only in tests.

Our mitigation: `packages/core/abi-macros/src/lib.rs` `parse_at_call_site` re-spans parsed tokens to `Span::call_site()`.

Note: a fixture run before the stop rule arrived (since deleted) confirmed it on 0.4.3133: goto-definition on a
method from `#[derive]` emitting a ~4 KiB string via `str::parse` → `-32603 request handler panicked: invalid offset`
(`line-index-0.1.2/src/lib.rs:111:35`); the same derive with tokens re-spanned to `Span::call_site()` → OK.
