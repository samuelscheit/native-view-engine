use wasm_bindgen::prelude::*;
#[wasm_bindgen]
extern "C" { fn render(v: i32); }

#[cfg(not(any(feature = "syc", feature = "rg", feature = "dx")))]
mod imp {
    use super::*;
    use std::cell::Cell;
    thread_local!(static C: Cell<i32> = const { Cell::new(0) });
    #[wasm_bindgen] pub fn start() { render(0) }
    #[wasm_bindgen] pub fn click() { C.with(|c| { c.set(c.get() + 1); render(c.get()) }) }
}

#[cfg(feature = "syc")]
mod imp {
    use super::*;
    use sycamore_reactive::*;
    thread_local!(static S: std::cell::Cell<Option<Signal<i32>>> = const { std::cell::Cell::new(None) });
    #[wasm_bindgen] pub fn start() {
        let root = create_root(|| {
            let s = create_signal(0);
            let d = create_memo(move || s.get() * 2);
            create_effect(move || render(d.get()));
            S.with(|c| c.set(Some(s)));
        });
        std::mem::forget(root);
    }
    #[wasm_bindgen] pub fn click() { S.with(|c| if let Some(s) = c.get() { s.set(s.get() + 1) }) }
}

#[cfg(feature = "rg")]
mod imp {
    use super::*;
    use reactive_graph::{computed::Memo, effect::Effect, owner::Owner, signal::RwSignal, traits::*};
    thread_local!(static S: std::cell::Cell<Option<RwSignal<i32>>> = const { std::cell::Cell::new(None) });
    #[wasm_bindgen] pub fn start() {
        let owner = Owner::new();
        owner.set();
        let s = RwSignal::new(0);
        let d = Memo::new(move |_| s.get() * 2);
        Effect::new_isomorphic(move |_| render(d.get()));
        S.with(|c| c.set(Some(s)));
        std::mem::forget(owner);
    }
    #[wasm_bindgen] pub fn click() { S.with(|c| if let Some(s) = c.get() { s.set(s.get_untracked() + 1) }) }
}

#[cfg(feature = "dx")]
mod imp {
    use super::*;
    use dioxus_core::*;
    use dioxus_core_macro::rsx;
    use dioxus_signals::*;
    pub mod dioxus_elements {
        pub mod elements { pub use super::div; }
        #[allow(non_camel_case_types)] pub struct div;
        impl div { pub const TAG_NAME: &'static str = "div"; pub const NAME_SPACE: Option<&'static str> = None; }
        pub mod events {}
    }

    struct W;
    impl WriteMutations for W {
        fn append_children(&mut self, _: ElementId, _: usize) {}
        fn assign_node_id(&mut self, _: &'static [u8], _: ElementId) {}
        fn create_placeholder(&mut self, _: ElementId) {}
        fn create_text_node(&mut self, v: &str, _: ElementId) { render(v.len() as i32) }
        fn load_template(&mut self, _: Template, _: usize, _: ElementId) {}
        fn replace_node_with(&mut self, _: ElementId, _: usize) {}
        fn replace_placeholder_with_nodes(&mut self, _: &'static [u8], _: usize) {}
        fn insert_nodes_after(&mut self, _: ElementId, _: usize) {}
        fn insert_nodes_before(&mut self, _: ElementId, _: usize) {}
        fn set_attribute(&mut self, _: &'static str, _: Option<&'static str>, _: &AttributeValue, _: ElementId) {}
        fn set_node_text(&mut self, v: &str, _: ElementId) { render(v.len() as i32) }
        fn create_event_listener(&mut self, _: &'static str, _: ElementId) {}
        fn remove_event_listener(&mut self, _: &'static str, _: ElementId) {}
        fn remove_node(&mut self, _: ElementId) {}
        fn push_root(&mut self, _: ElementId) {}
    }
    thread_local!(static VD: std::cell::RefCell<Option<VirtualDom>> = const { std::cell::RefCell::new(None) });
    thread_local!(static SIG: std::cell::Cell<Option<Signal<i32>>> = const { std::cell::Cell::new(None) });
    fn app() -> Element {
        let s = use_hook(|| Signal::new(0));
        SIG.with(|c| c.set(Some(s)));
        rsx! { div { "{s}" } }
    }
    #[wasm_bindgen] pub fn start() {
        let mut vd = VirtualDom::new(app);
        vd.rebuild(&mut W);
        VD.with(|v| *v.borrow_mut() = Some(vd));
    }
    #[wasm_bindgen] pub fn click() {
        if let Some(mut s) = SIG.with(|c| c.get()) { s += 1; }
        VD.with(|v| if let Some(vd) = v.borrow_mut().as_mut() { vd.mark_all_dirty(); vd.render_immediate(&mut W) });
    }
}
