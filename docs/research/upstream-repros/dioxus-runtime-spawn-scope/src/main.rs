//! Minimal repro: a task spawned with the public `Runtime::spawn(scope, fut)` is not
//! registered in `scope.spawned_tasks`, so unmounting `scope` neither cancels nor drops it.
//! Usage: cargo run -- runtime-spawn | in-scope-spawn
#![allow(non_snake_case)]

#[cfg(feature = "main")]
extern crate dioxus_vdom as dioxus_core;
#[cfg(not(feature = "main"))]
use dioxus_core_macro::rsx;
#[cfg(feature = "main")]
use dioxus_vdom_macro::rsx;
#[cfg(feature = "main")]
extern crate dioxus_signals_main as dioxus_signals;

use dioxus_core::{current_scope_id, spawn, Element, NoOpMutations, Runtime, ScopeId, VirtualDom};
use std::cell::Cell;

thread_local! {
    static SHOW_CHILD: Cell<bool> = const { Cell::new(true) };
    static CHILD_SCOPE: Cell<Option<ScopeId>> = const { Cell::new(None) };
}

fn App() -> Element {
    let show = SHOW_CHILD.with(Cell::get);
    rsx! { if show { Child {} } }
}

fn Child() -> Element {
    CHILD_SCOPE.with(|c| c.set(Some(current_scope_id())));
    rsx! {}
}

/// Prints when the future is dropped, so a leak is observable.
struct DropProbe;
impl Drop for DropProbe {
    fn drop(&mut self) {
        println!("  future dropped");
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "runtime-spawn".into());
    let mut dom = VirtualDom::new(App);
    dom.rebuild_in_place();
    let child = CHILD_SCOPE.with(Cell::get).expect("Child mounted");
    println!("mode={mode} child={child:?}");

    let probe = DropProbe;
    let fut = async move {
        let _probe = probe;
        println!("  task polled");
        std::future::pending::<()>().await;
    };
    let task = dom.in_runtime(|| match mode.as_str() {
        // Public API, documented: "Whenever the component that owns this future is dropped,
        // the future will be dropped as well."
        "runtime-spawn" => Runtime::current().spawn(child, fut),
        // Workaround: the global `spawn` registers the task in the scope's spawned_tasks.
        "in-scope-spawn" => Runtime::current().in_scope(child, || spawn(fut)),
        other => panic!("unknown mode {other}"),
    });
    println!("spawned {task:?} onto {child:?}");

    // Unmount Child in the same frame in which the task is queued.
    SHOW_CHILD.with(|s| s.set(false));
    dom.mark_dirty(ScopeId::APP);
    println!("render_immediate (App rerenders first, removing Child, then dirty tasks are popped)");
    dom.render_immediate(&mut NoOpMutations);
    println!("render_immediate returned");

    // The scope is gone: a correctly owned task must have been dropped by now.
    dom.render_immediate(&mut NoOpMutations);
    println!("end of main (dropping VirtualDom)");
    drop(dom);
    println!("done");
}
