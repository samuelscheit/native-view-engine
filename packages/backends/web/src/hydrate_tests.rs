//! Crate-internal hydration invariants (echo-suppression baselines, replay filtering) that need
//! `pub(crate)` backend state.

use nve_engine::StoreBackend;
use nve_store::{ChangeSet, NodeId, NodeStore};

use crate::WebBackend;

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
