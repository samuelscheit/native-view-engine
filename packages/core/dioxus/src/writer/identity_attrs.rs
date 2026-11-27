//! Identity (`DebugId`: `testID`/`id`/`slot`) and streaming SSR boundary (`StreamBoundary`:
//! `data-nve-stream-slot`/`data-nve-suspense`) string attributes.

use nve_components::tables::{DebugId, NVE_STREAM_SLOT_ATTR, NVE_SUSPENSE_ATTR, StreamBoundary};
use nve_store::NodeId;

use super::StoreMutationTx;

impl StoreMutationTx<'_, '_> {
    /// Sets (`Some`) or clears (`None`) an identity / streaming-boundary string attribute.
    /// Returns `false` when `name` is not one of them. Clearing never attaches a missing table.
    pub(super) fn apply_identity_attr(
        &mut self,
        node: NodeId,
        name: &str,
        value: Option<&str>,
    ) -> bool {
        let v = value.unwrap_or("");
        match name {
            "testID" | "data-testid" | "test_id" | "id" | "slot" => {
                if value.is_none() && !self.tx.store().has::<DebugId>(node) {
                    return true;
                }
                let row = self.tx.edit::<DebugId>(node);
                match name {
                    "id" => row.id(v),
                    "slot" => row.slot(v),
                    _ => row.test_id(v),
                };
            }
            NVE_STREAM_SLOT_ATTR | NVE_SUSPENSE_ATTR => {
                if value.is_none() && !self.tx.store().has::<StreamBoundary>(node) {
                    return true;
                }
                let row = self.tx.edit::<StreamBoundary>(node);
                if name == NVE_STREAM_SLOT_ATTR {
                    row.stream_slot(v);
                } else {
                    row.suspense(v);
                }
            }
            _ => return false,
        }
        true
    }
}
