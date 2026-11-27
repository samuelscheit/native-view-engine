//! Crate-internal hydration invariants (echo-suppression baselines, replay filtering) that need
//! `pub(crate)` backend state.

use nve_engine::StoreBackend;
use nve_store::{ChangeSet, NodeId, NodeStore};

use crate::WebBackend;
use crate::hydrate::SELECT_BASE_CASES;
use crate::hydrate_native_parse::parse_ssr_html_tree;

/// The headless parser derives every shared [`SELECT_BASE_CASES`] base value exactly as browsers
/// do (the live Chrome `nve_probe_select_hydration` E2E runs the same table).
#[test]
fn headless_select_base_value_matches_the_shared_browser_parity_table() {
    for case in SELECT_BASE_CASES {
        let html = format!(
            "<div data-nve-id=\"1\"><select data-nve-id=\"2\" {}>{}</select></div>",
            case.attrs, case.options
        );
        let tree = parse_ssr_html_tree(&html);
        let select = tree
            .arena
            .iter()
            .find(|n| n.tag == "select")
            .expect("select parsed");
        assert_eq!(
            select.base_control_value.as_deref(),
            Some(case.expected),
            "{case:?}"
        );
        assert_eq!(
            select.live_control_value.as_deref(),
            Some(case.expected),
            "{case:?}"
        );
    }
}

/// A dirty adopted `<input>` keeps the user's pre-hydration value through the deferred commit;
/// the echo baseline must follow the 3-way resolved value, so typing the server value back
/// afterwards is a real edit (`TextChange`), not a swallowed echo.
#[test]
fn deferred_hydrated_control_keeps_echo_baseline_on_resolved_local_value() {
    let build = |store: &mut NodeStore, value: &str| {
        let mut changes = ChangeSet::default();
        let mut tx = store.tx();
        let input = tx.create();
        tx.set_children(NodeId::ROOT, &[input]);
        tx.edit::<nve_components::tables::TextInput>(input).value(value);
        tx.commit_into(&mut changes);
        (input, changes)
    };
    let mut server_store = NodeStore::new();
    let (input, changes) = build(&mut server_store, "draft");
    let mut server = WebBackend::new();
    server.commit(&server_store, &changes).unwrap();
    let ssr = server.render_ssr(&server_store);

    let mut client = WebBackend::hydrate_from_ssr(&ssr);
    client.set_hydrated_dom_control_value(input.raw(), "user-edited");
    let mut client_store = NodeStore::new();
    let (client_input, changes) = build(&mut client_store, "draft");
    client.commit(&client_store, &changes).unwrap();
    assert_eq!(client.hydration_report().reconciled_dirty_controls, 1);
    assert_eq!(client.native_dom_control_value(client_input.raw()), Some("user-edited"));

    let mut sync = client.input_sync.borrow_mut();
    assert_eq!(
        sync.on_input_event(client_input.raw(), "user-edited".into(), false),
        None,
        "the reconciled local value is the baseline: re-reporting it is an echo"
    );
    assert_eq!(
        sync.on_input_event(client_input.raw(), "draft".into(), false),
        Some("draft".into()),
        "editing back to the server value must emit TextChange"
    );
}
