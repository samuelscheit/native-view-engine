#![cfg(not(target_arch = "wasm32"))]
//! Headless behavioral integration tests for `nve-query-dioxus`:
//! 1. Zero-input async frame wake (Blocker B1 guard): a background thread resolves a query with
//!    ZERO user events, triggering `DioxusSurface::poll_work` and producing the updated frame!
//! 2. `use_query` key switch, `keep_previous_data`, dense recycled slot signals (0 leaks on drop).
//! 3. `use_action::run_optimistic` instant UI update + server invalidation auto-refetch.
//! 4. `<ActionForm<P>>` local 0ms `FormDecode` validation, CBOR RPC submit, server `FieldErrors`
//!    inline binding, draft preservation on error, and clean baseline commit on success.

use core::task::{Poll, Waker};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::RenderBackend;
use nve_query::{ManualClock, QueryClientConfig, QueryOptions, Shared};
use nve_query_dioxus::prelude::*;
use nve_rpc::{
    procedures, BoxFuture, CborCodec, DomainStatus, FieldErrors, FormDecode, FormPairs, Procedure,
    RawCall, RawReply, ReplyKind, RpcCodec, RpcStatus, Tag64, Transport, TransportError,
};
use nve_ui::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct Todo {
    #[n(0)]
    id: u64,
    #[n(1)]
    title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct RenameTodoInput {
    #[n(0)]
    id: u64,
    #[n(1)]
    title: String,
}

impl FormDecode for RenameTodoInput {
    fn from_form(form: &FormPairs) -> Result<Self, FieldErrors> {
        let mut errs = FieldErrors::new();
        let id = form
            .get("id")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or_else(|| {
                errs.push("id", "valid numeric id required");
                0
            });
        let title = form.get("title").unwrap_or("").trim().to_owned();
        if title.is_empty() {
            errs.push("title", "title must not be empty");
        }
        errs.into_result(Self { id, title })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
enum TodoErr {
    #[n(0)]
    Validation(#[n(0)] FieldErrors),
}

impl DomainStatus for TodoErr {
    fn rpc_status(&self) -> RpcStatus {
        RpcStatus::Unprocessable
    }

    fn field_errors(&self) -> Option<&FieldErrors> {
        match self {
            Self::Validation(fe) => Some(fe),
        }
    }
}

procedures! {
    query GetTodo = "todo.v1.Todos/Get" (u64) -> Todo, TodoErr,
        tags |id, sink| {
            sink.push(Tag64::keyed("todo", *id));
        };
    action RenameTodo = "todo.v1.Todos/Rename" (RenameTodoInput) -> Todo, TodoErr,
        invalidates |inp, sink| {
            sink.push(Tag64::keyed("todo", inp.id));
        };
}

/// Async transport whose queries can be resolved from another thread to prove zero-input wakeup.
#[derive(Clone, Default)]
struct AsyncDemoTransport {
    state: Arc<Mutex<TransportState>>,
}

struct TransportState {
    titles: std::collections::BTreeMap<u64, String>,
    defer_queries: bool,
    defer_actions: bool,
    reject_title: Option<String>,
    pending_wakers: Vec<Waker>,
    query_calls: usize,
    action_calls: usize,
}

impl Default for TransportState {
    fn default() -> Self {
        let mut titles = std::collections::BTreeMap::new();
        titles.insert(1, "Ship async wake".into());
        titles.insert(2, "Verify ActionForm".into());
        Self {
            titles,
            defer_queries: false,
            defer_actions: false,
            reject_title: None,
            pending_wakers: Vec::new(),
            query_calls: 0,
            action_calls: 0,
        }
    }
}

impl AsyncDemoTransport {
    fn release_queries(&self) {
        let wakers: Vec<Waker> = {
            let mut g = self.state.lock().unwrap();
            g.defer_queries = false;
            core::mem::take(&mut g.pending_wakers)
        };
        for w in wakers {
            w.wake();
        }
    }

    fn release_actions(&self) {
        let wakers: Vec<Waker> = {
            let mut g = self.state.lock().unwrap();
            g.defer_actions = false;
            core::mem::take(&mut g.pending_wakers)
        };
        for w in wakers {
            w.wake();
        }
    }
}

impl Transport for AsyncDemoTransport {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            if req.name == GetTodo::NAME {
                let id: u64 = CborCodec::default().decode_bounded(&req.body).unwrap();
                core::future::poll_fn(|cx| {
                    let mut g = state.lock().unwrap();
                    if g.defer_queries {
                        g.pending_wakers.push(cx.waker().clone());
                        Poll::Pending
                    } else {
                        Poll::Ready(())
                    }
                })
                .await;
                let title = {
                    let mut g = state.lock().unwrap();
                    g.query_calls += 1;
                    g.titles
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| format!("Todo #{id}"))
                };
                let mut body = Vec::new();
                CborCodec::default()
                    .encode_into(&Todo { id, title }, &mut body)
                    .unwrap();
                return Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }

            if req.name == RenameTodo::NAME {
                let input: RenameTodoInput =
                    CborCodec::default().decode_bounded(&req.body).unwrap();
                core::future::poll_fn(|cx| {
                    let mut g = state.lock().unwrap();
                    if g.defer_actions {
                        g.pending_wakers.push(cx.waker().clone());
                        Poll::Pending
                    } else {
                        Poll::Ready(())
                    }
                })
                .await;
                let mut g = state.lock().unwrap();
                g.action_calls += 1;
                if g.reject_title.as_deref() == Some(input.title.as_str()) {
                    let mut fe = FieldErrors::new();
                    fe.push("title", "server rejected reserved title");
                    let mut body = Vec::new();
                    CborCodec::default()
                        .encode_into(&TodoErr::Validation(fe), &mut body)
                        .unwrap();
                    return Ok(RawReply {
                        http_status: 422,
                        kind: ReplyKind::Domain,
                        invalidated_tags: Vec::new(),
                        invalidate_all: false,
                        body: Bytes::from(body),
                    });
                }
                g.titles.insert(input.id, input.title.clone());
                let updated = Todo {
                    id: input.id,
                    title: input.title,
                };
                let mut body = Vec::new();
                CborCodec::default().encode_into(&updated, &mut body).unwrap();
                return Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: vec![Tag64::keyed("todo", input.id)],
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }

            Err(TransportError::config("unknown procedure"))
        })
    }
}

struct FlagWaker(AtomicBool, AtomicUsize);

impl std::task::Wake for FlagWaker {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::Release);
        self.1.fetch_add(1, Ordering::Relaxed);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.store(true, Ordering::Release);
        self.1.fetch_add(1, Ordering::Relaxed);
    }
}

fn render_html(surface: &mut DioxusSurface, web: &mut WebBackend, frame: &mut u64) -> String {
    *frame += 1;
    let (_, _, buf) = surface.render_frame(*frame);
    web.submit_frame(&buf).unwrap();
    web.render_html()
}

fn pump_all_async(
    surface: &mut DioxusSurface,
    web: &mut WebBackend,
    waker: &Waker,
    frame: &mut u64,
) -> usize {
    let mut rendered = 0;
    for _ in 0..16 {
        if !surface.poll_work(waker) {
            return rendered;
        }
        *frame += 1;
        let (_, _, buf) = surface.render_frame(*frame);
        web.submit_frame(&buf).unwrap();
        rendered += 1;
    }
    assert!(
        !surface.poll_work(waker),
        "pump_all_async exhausted 16 iterations without settling — probable infinite render/refetch loop!"
    );
    rendered
}

#[derive(Props, Clone, PartialEq)]
struct DemoAppProps {
    client: ErasedQueryClient,
    show_child: Signal<bool>,
    active_id: Signal<u64>,
    form_ctrl: Signal<FormController>,
}

#[component]
fn TodoChild(id: u64) -> Element {
    let q = use_query_with::<GetTodo>(
        id,
        QueryOptions::default()
            .stale_time_ms(60_000)
            .keep_previous_data(true),
    );
    let label = match q.data() {
        Some(todo) => format!("TODO:{}:{}", todo.id, todo.title),
        None => "LOADING".to_owned(),
    };
    rsx! {
        text { test_id: "todo-label", "{label}" }
    }
}

#[component]
fn DemoApp(props: DemoAppProps) -> Element {
    let show = *props.show_child.read();
    let id = *props.active_id.read();
    rsx! {
        ThemeProvider {
            theme: UiTheme::light(),
            QueryClientProvider {
                client: props.client.clone(),
                Col {
                    if show {
                        TodoChild { id: id }
                    }
                    ActionForm::<RenameTodo> {
                        test_id: "rename-form",
                        action: "/todos/rename".to_owned(),
                        controller: Some(props.form_ctrl),
                        FormHiddenField { name: "id".to_owned(), value: id.to_string() }
                        FormField {
                            name: "title".to_owned(),
                            test_id: "title-input".to_owned(),
                            label: "Title".to_owned(),
                        }
                        FormSubmitButton {
                            test_id: "rename-submit".to_owned(),
                            "Save"
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn zero_input_async_wake_signal_recycling_and_action_form_work_end_to_end() {
    let transport = AsyncDemoTransport::default();
    transport.state.lock().unwrap().defer_queries = true;
    transport.state.lock().unwrap().reject_title = Some("FORBIDDEN".into());

    let clock = Shared::new(ManualClock::new(1_000));
    let client = erased_query_client_with_clock(
        transport.clone(),
        clock,
        QueryClientConfig::default(),
    );

    let mut frame = 0u64;
    let mut web = WebBackend::new();
    let wake_flag = Arc::new(FlagWaker(AtomicBool::new(false), AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&wake_flag));

    type TestHandles = (Signal<bool>, Signal<u64>, Signal<FormController>);
    type SharedHandles = std::rc::Rc<std::cell::RefCell<Option<TestHandles>>>;

    // Create reactive root control signals inside surface's ROOT scope via a thin wrapper component.
    #[component]
    fn RootWrapper() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        let show_child = use_signal(|| true);
        let active_id = use_signal(|| 1u64);
        let form_ctrl = use_signal(|| {
            FormController::with_persistence("rename-draft", MemoryFormDraftStorage::new())
        });
        dioxus_core::use_hook(|| {
            let handles = dioxus_core::consume_context::<SharedHandles>();
            *handles.borrow_mut() = Some((show_child, active_id, form_ctrl));
        });
        rsx! {
            DemoApp {
                client: client,
                show_child: show_child,
                active_id: active_id,
                form_ctrl: form_ctrl,
            }
        }
    }

    let handles_box: SharedHandles = std::rc::Rc::new(std::cell::RefCell::new(None));

    let mut surface = DioxusSurface::new(1, RootWrapper)
        .with_context(client.clone())
        .with_context(std::rc::Rc::clone(&handles_box));

    // Frame 1: deferred query renders LOADING; arm host waker via `poll_work`.
    let html1 = render_html(&mut surface, &mut web, &mut frame);
    assert!(html1.contains("LOADING"), "initial frame must show LOADING: {html1}");
    assert!(html1.contains("method=\"post\""), "ActionForm emits 0-JS form: {html1}");
    assert!(html1.contains("action=\"/todos/rename\""), "{html1}");
    assert!(html1.contains("type=\"hidden\""), "FormHiddenField emits hidden input: {html1}");
    let _ = pump_all_async(&mut surface, &mut web, &waker, &mut frame);
    wake_flag.0.store(false, Ordering::Release);

    // Resolve the deferred query from a SEPARATE background OS thread with ZERO user input events!
    let t_bg = transport.clone();
    std::thread::spawn(move || {
        t_bg.release_queries();
    })
    .join()
    .unwrap();

    // Host waker MUST have fired automatically with zero user clicks/keypresses (Blocker B1 guard!).
    assert!(
        wake_flag.0.load(Ordering::Acquire),
        "resolving an async query on a background thread MUST wake DioxusSurface's host waker!"
    );
    let pumped = pump_all_async(&mut surface, &mut web, &waker, &mut frame);
    assert!(pumped >= 1, "expected at least 1 async frame after background wake");
    let html2 = web.render_html();
    assert!(
        html2.contains("TODO:1:Ship async wake"),
        "async query result must render without any user input: {html2}"
    );

    let (mut show_sig, mut id_sig, mut ctrl_sig) = handles_box.borrow().unwrap();

    // 2. Key switch `1 -> 2` with deferred Todo #2 proves `keep_previous_data` holds Todo #1
    //    as placeholder while #2 is in flight, then switches cleanly once #2 arrives.
    transport.state.lock().unwrap().defer_queries = true;
    surface.in_root_scope(|| {
        id_sig.set(2);
    });
    let html_switching = render_html(&mut surface, &mut web, &mut frame);
    let _ = pump_all_async(&mut surface, &mut web, &waker, &mut frame);
    assert!(
        html_switching.contains("TODO:1:Ship async wake")
            && web.render_html().contains("TODO:1:Ship async wake"),
        "keep_previous_data must retain previous key's data while new key fetches: {}",
        web.render_html()
    );
    transport.release_queries();
    let _ = pump_all_async(&mut surface, &mut web, &waker, &mut frame);
    assert!(web.render_html().contains("TODO:2:Verify ActionForm"));
    assert_eq!(client.active_slot_count(), 1);

    // 3. ActionForm local 0ms `FormDecode` validation (empty title -> 0 network calls, inline error!).
    let actions_before = transport.state.lock().unwrap().action_calls;
    let form_node = web
        .iter_nodes()
        .find_map(|(id, el)| {
            matches!(
                el.raw_props.get("testID"),
                Some(nve_abi::RawPropValue::String(s)) if s == "rename-form"
            )
            .then_some(id)
        })
        .expect("rename-form present");
    let submit_ev = nve_abi::NativeEvent {
        surface_id: 1,
        node_id: form_node,
        timestamp_us: 0,
        kind: nve_abi::NativeEventKind::Submit,
        payload: nve_abi::EventPayload::Text(String::new()),
    };
    assert!(surface.dispatch_events(std::slice::from_ref(&submit_ev)));
    let _ = render_html(&mut surface, &mut web, &mut frame);
    assert_eq!(
        transport.state.lock().unwrap().action_calls,
        actions_before,
        "local FormDecode error must NOT fire a network RPC call"
    );
    assert!(
        web.render_html().contains("title must not be empty"),
        "local FormDecode error must render inline"
    );

    // 4. ActionForm server 422 `FieldErrors`: preserves dirty draft & renders server error inline!
    surface.in_root_scope(|| {
        ctrl_sig.write().set_text("title", "FORBIDDEN");
    });
    let _ = render_html(&mut surface, &mut web, &mut frame);
    assert!(surface.dispatch_events(std::slice::from_ref(&submit_ev)));
    let _ = render_html(&mut surface, &mut web, &mut frame);
    let _ = pump_all_async(&mut surface, &mut web, &waker, &mut frame);
    assert!(
        web.render_html().contains("server rejected reserved title"),
        "server FieldErrors must bind inline"
    );
    let still_dirty = surface.in_root_scope(|| ctrl_sig.read().is_dirty());
    assert!(
        still_dirty,
        "failed async ActionForm submit MUST preserve dirty user draft!"
    );

    // 5. ActionForm success: commits clean baseline, clears draft, invalidates tag, auto-refetches!
    surface.in_root_scope(|| {
        ctrl_sig.write().set_text("title", "Shipped Cleanly");
    });
    let _ = render_html(&mut surface, &mut web, &mut frame);
    assert!(surface.dispatch_events(&[submit_ev]));
    let _ = render_html(&mut surface, &mut web, &mut frame);
    let _ = pump_all_async(&mut surface, &mut web, &waker, &mut frame);
    let html_done = web.render_html();
    assert!(
        html_done.contains("TODO:2:Shipped Cleanly"),
        "successful ActionForm must invalidate tag and auto-refetch Todo #2: {html_done}"
    );
    let now_clean = surface.in_root_scope(|| !ctrl_sig.read().is_dirty());
    assert!(now_clean, "successful ActionForm submit commits clean baseline");

    // 6. Unmounting TodoChild drops subscription and recycles slot Signal (0 leaks!).
    surface.in_root_scope(|| {
        show_sig.set(false);
    });
    let _ = render_html(&mut surface, &mut web, &mut frame);
    assert_eq!(client.active_slot_count(), 0);
}

#[test]
fn use_action_run_optimistic_updates_immediately_and_rolls_back_on_error() {
    let transport = AsyncDemoTransport::default();
    {
        let mut g = transport.state.lock().unwrap();
        g.defer_actions = true;
        g.reject_title = Some("REJECT_ME".into());
    }

    let clock = Shared::new(ManualClock::new(1_000));
    let client = erased_query_client_with_clock(
        transport.clone(),
        clock,
        QueryClientConfig::default(),
    );

    #[component]
    fn OptView() -> Element {
        let q = use_query::<GetTodo>(1);
        let rename = use_action::<RenameTodo>();
        let title = q
            .data()
            .map(|t| t.title.clone())
            .unwrap_or_else(|| "LOADING".to_owned());
        let has_err = rename.error().is_some();
        rsx! {
            Col {
                text { test_id: "opt-title", "OPT:{title}:ERR:{has_err}" }
                Button {
                    test_id: "opt-fail-btn",
                    on_press: move |_| {
                        rename.run_optimistic(
                            RenameTodoInput {
                                id: 1,
                                title: "REJECT_ME".into(),
                            },
                            |tx| {
                                let _ = tx.update_query::<GetTodo>(&1, |old| Todo {
                                    id: old.id,
                                    title: "OPTIMISTIC_FLASH".into(),
                                });
                            },
                        );
                    },
                    "Trigger"
                }
            }
        }
    }

    #[component]
    fn OptRoot() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            ThemeProvider {
                theme: UiTheme::light(),
                QueryClientProvider {
                    client: client,
                    OptView {}
                }
            }
        }
    }

    let mut frame = 0u64;
    let mut web = WebBackend::new();
    let wake_flag = Arc::new(FlagWaker(AtomicBool::new(false), AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&wake_flag));
    let mut surface = DioxusSurface::new(1, OptRoot).with_context(client);

    let _ = render_html(&mut surface, &mut web, &mut frame);
    let _ = pump_all_async(&mut surface, &mut web, &waker, &mut frame);
    assert!(web.render_html().contains("OPT:Ship async wake:ERR:false"));

    let btn = web
        .iter_nodes()
        .find_map(|(id, el)| {
            matches!(
                el.raw_props.get("testID"),
                Some(nve_abi::RawPropValue::String(s)) if s == "opt-fail-btn"
            )
            .then_some(id)
        })
        .expect("opt-fail-btn present");

    // Click triggers `run_optimistic`: synchronous patch appears immediately while RPC is in flight!
    let press_ev = nve_abi::NativeEvent {
        surface_id: 1,
        node_id: btn,
        timestamp_us: 0,
        kind: nve_abi::NativeEventKind::Press,
        payload: nve_abi::EventPayload::Point { x: 1.0, y: 1.0 },
    };
    assert!(surface.dispatch_events(&[press_ev]));
    let _ = render_html(&mut surface, &mut web, &mut frame);
    let _ = pump_all_async(&mut surface, &mut web, &waker, &mut frame);
    let mid = web.render_html();
    assert!(
        mid.contains("OPT:OPTIMISTIC_FLASH:ERR:false"),
        "optimistic patch must render while action RPC is still pending: {mid}"
    );

    // Release server rejection -> rolls back optimistic patch and exposes action error.
    transport.release_actions();
    let _ = pump_all_async(&mut surface, &mut web, &waker, &mut frame);
    let after = web.render_html();
    assert!(
        after.contains("OPT:Ship async wake:ERR:true"),
        "rejected optimistic action must roll back title and expose error: {after}"
    );
}
