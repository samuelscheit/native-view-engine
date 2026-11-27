# dioxus: `Runtime::spawn(scope, fut)` task is not owned by `scope`

Verdict: **genuine upstream bug**. Reproduced on crates.io `dioxus-core 0.7.10` and on DioxusLabs/dioxus `main` @ `736f4ae77ab0d940e9c115c415fc7b77c5895226` (2026-11-27, where `dioxus-core` is renamed to `dioxus-vdom` 0.8.0-alpha.4). No existing issue or PR was found (searched below).

## Contract vs. code

`Runtime::spawn` is public (`pub fn spawn(&self, scope: ScopeId, task) -> Task`, `tasks.rs:153` in both versions). Its doc comment says:

> Whenever the component that owns this future is dropped, the future will be dropped as well.

But only `ScopeContext::spawn` (`pub(crate)`, the path behind the global `spawn()`) inserts the task into `scope.spawned_tasks`. `Runtime::spawn` → `spawn_task_of_type_inner` doesn't. `Runtime::remove_scope` cancels only `spawned_tasks`, so the task outlives its scope:

- **Debug:** if the task's wakeup was queued before the scope unmounted, `dirty_tasks` keeps an entry keyed by the dead scope. `pop_task` then panics on `debug_assert!(self.scopes.contains(..))` (`scheduler.rs:155` in 0.7.10, `scheduler.rs:187` on main).
- **Release:** the future is polled after its owner unmounted (with the dead `ScopeId` pushed on the scope stack) and is dropped only with the `VirtualDom`. That is a leak. If the `ScopeId` slot is reused, the task also runs under a different component (the assert comment says "the scope id may have been reallocated").

`Runtime::spawn_isomorphic` has the same gap.

## Fixture

`src/main.rs`: `App` renders `Child` while a thread-local flag is true. After `rebuild_in_place` the program:

1. spawns a pending future onto `Child`'s scope, either with `Runtime::current().spawn(child, fut)` (`runtime-spawn`) or with `Runtime::current().in_scope(child, || spawn(fut))` (`in-scope-spawn`, the control);
2. clears the flag, calls `mark_dirty(ScopeId::APP)` and then `render_immediate`.

`App` reruns first because it is higher in the tree, so `Child` unmounts. Then the queued task is popped. A `DropProbe` inside the future prints when the future is dropped.

`./run.sh` (main comes from a git dependency pinned to `736f4ae`) runs both modes × {0.7.10, main} × {debug, release} and writes the log to `out/run.log`. Versions used: rustc 1.98.1, cargo 1.98.1, macOS arm64.

### Expected

For both modes, the future is dropped when `Child` unmounts (inside the first `render_immediate`), it is never polled afterwards, and nothing panics.

### Actual (from `out/run.log`)

```
=== v0710 debug runtime-spawn
mode=runtime-spawn child=ScopeId(4)
spawned Task { id: DefaultKey(1v1), .. } onto ScopeId(4)
render_immediate (App rerenders first, removing Child, then dirty tasks are popped)
thread 'main' panicked at .../dioxus-core-0.7.10/src/scheduler.rs:155:9:
assertion failed: self.scopes.contains(tasks.order.id.0)
  future dropped
exit=101
=== v0710 debug in-scope-spawn
render_immediate (App rerenders first, removing Child, then dirty tasks are popped)
  future dropped
render_immediate returned
exit=0
=== v0710 release runtime-spawn
render_immediate (App rerenders first, removing Child, then dirty tasks are popped)
  task polled                      <- polled after its scope was removed
render_immediate returned
end of main (dropping VirtualDom)
  future dropped                   <- leaked until VirtualDom drop
exit=0
=== main debug runtime-spawn
thread 'main' panicked at upstream-main/packages/vdom/src/scheduler.rs:187:9:
assertion failed: self.scopes.contains(order.id.index())
exit=101
=== main release runtime-spawn
  task polled
render_immediate returned
end of main (dropping VirtualDom)
  future dropped
exit=0
(in-scope-spawn: correct on all four configurations)
```

## Tracker search (DioxusLabs/dioxus)

I searched issues and PRs with `gh search issues/prs` for: `Runtime::spawn`, `spawned_tasks`, `pop_task`, `pop_task debug_assert`, `scopes.contains`, `remove_scope`, `scheduler.rs assertion failed`, `spawn scope unmount`, and `task spawned scope unmounted panic`. I also ran a web search and checked the git log of `tasks.rs` on main. Nothing matches. These are related but different:

- https://github.com/DioxusLabs/dioxus/pull/1710 (merged): "Make cx.spawn poll the task before returning `TaskId`". It introduced the scope-level `spawned_tasks` bookkeeping.
- https://github.com/DioxusLabs/dioxus/pull/3201 (merged): "Don't run effects from dead recycled scopes". It fixed the same class of bug for effects, and `remove_task` was taught to purge `dirty_tasks`. Tasks spawned via `Runtime::spawn` never reach that path.
- https://github.com/DioxusLabs/dioxus/pull/5865 (open): "Keep wait_for_work from spinning forever on a task that wakes itself". Unrelated.

## Draft upstream report

- **Where:** https://github.com/DioxusLabs/dioxus/issues/new?template=bug_report.md (label `bug`, `core`). File a new issue; there is nothing to comment on.
- **Title:** `Runtime::spawn(scope, fut) does not tie the task to scope: debug_assert panic in pop_task / leaked task after unmount`

**Body:**

> **Problem**
> `Runtime::spawn(scope, fut)` (public) documents "Whenever the component that owns this future is dropped, the future will be dropped as well", but unlike `ScopeContext::spawn` (used by global `spawn()`) it never inserts the task into `scope.spawned_tasks`. `Runtime::remove_scope` only cancels tasks in `spawned_tasks`, so the task survives the scope's unmount. In debug builds, if the task was already queued in `dirty_tasks`, `VirtualDom::pop_task` panics with `assertion failed: self.scopes.contains(order.id.index())` (`packages/vdom/src/scheduler.rs:187`, `dioxus-core 0.7.10 scheduler.rs:155`). In release builds the future is polled after its owner is gone and is only dropped with the `VirtualDom`. If the ScopeId is recycled, it is polled under an unrelated component. `Runtime::spawn_isomorphic` has the same gap.
>
> **Steps To Reproduce**
> Minimal standalone crate (no renderer): mount `Child`, then `dom.in_runtime(|| Runtime::current().spawn(child_scope, pending_future))`, unmount `Child` and `render_immediate`. (Attach `src/main.rs` from this fixture.)
>
> **Expected behavior**
> The task is removed when `child_scope` is removed, exactly like `Runtime::current().in_scope(child_scope, || spawn(fut))`, which works.
>
> **Environment**: dioxus-core 0.7.10 and main @ 736f4ae (dioxus-vdom 0.8.0-alpha.4), rustc 1.98.1, macOS arm64, no renderer (`NoOpMutations`).
>
> **Suggested fix**
> Move the `spawned_tasks` insertion into `Runtime::spawn_task_of_type_inner`: `if let Some(s) = self.try_get_state(scope) { s.spawned_tasks.borrow_mut().insert(task_id) }` (or refuse to spawn onto a missing scope), and drop the now-duplicate insert in `ScopeContext::spawn{,_isomorphic}`.

## Sibling entries in docs/UPSTREAM.md (not upstream bugs)

**Event attribute `"on"` prefix mismatch: not a bug; the UPSTREAM.md entry is stale or inaccurate.** In both 0.7.10 and main, `WriteMutations::create_event_listener` / `remove_event_listener` already receive the unprefixed name. The diff code calls `to.create_event_listener(&attribute.name[2..], id)` (`diff/node.rs:533`). `Runtime::handle_event(name, ..)` also takes the unprefixed name and does the `attr.name.get(2..) == Some(name)` matching internally (`runtime.rs:434`, marked `TODO … settle on one`). The `"on"` prefix exists only on the raw `Attribute.name` of listener attributes, which is internal and consistent. NVE no longer strips anything: `packages/core/dioxus/src/writer/mutations.rs:206` maps `name` directly, and `writer/attributes.rs:92` ignores `AttributeValue::Listener`. The mitigation text pointing at `packages/core/dioxus/src/lib.rs` is outdated. Recommend deleting the entry.

**Nested `Text` wrapper/leaf split: intended behavior; NVE mapping choice.** `text { style: "...", "literal" }` is NVE's own element (not dioxus-html). `rsx!` always models it as `TemplateNode::Element { tag: "text", children: [TemplateNode::Text("literal")] }`: an element and a separate text node, the same as `div { "x" }` on the DOM. Dioxus has no concept of style inheritance. Whether a text-node child inherits its parent's text style is the renderer's job: the DOM does it through CSS inheritance, and NVE does it by propagating `text_color`/`font_size`. This is our renderer's responsibility, not an upstream issue. Recommend rewording the entry as an NVE design note, or moving it out of UPSTREAM.md.
