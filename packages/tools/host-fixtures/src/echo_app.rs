//! `libnve_host_fixture_app`: a minimal `nve_get_interface` app for host FFI suites (Dart,
//! Swift, Java, C++) that must see real app-emitted events cross the runtime C ABI.
//!
//! `set_props` returns `true` whenever the prop keys or values differ from the previous call and
//! queues one `Custom` event named `props` (node 1) whose `NveBinValue` payload is the structured
//! map `{count: i64, keys: [str…], ok: true, ratio: 0.5}`. A truthy `malformed` prop first queues
//! a `Custom` event named `malformed` whose `NveBinValue` payload is invalid (`[0xFF]`), so hosts
//! prove one malformed custom event payload never drops the rest of the drained batch.

use nve_abi::{
    BinWriter, EventBatchBuf, FrameId, NodeId, NvePropsRef, SurfaceId, TimestampUs,
    ValidatedEvents,
};
use nve_engine::NveApp;

static MALFORMED_PAYLOAD: &[u8] = &[0xFF];

#[derive(Default)]
struct EchoApp {
    surface_id: SurfaceId,
    /// `key=value` signature of the last props, to report "changed".
    last: String,
    emitted: EventBatchBuf,
    drained: EventBatchBuf,
}

impl NveApp for EchoApp {
    fn create(surface_id: SurfaceId) -> Self {
        Self {
            surface_id,
            ..Self::default()
        }
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        &[]
    }

    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
        false
    }

    fn set_props(&mut self, props: NvePropsRef<'_>) -> bool {
        // `NveValueRef`'s `Debug` prints the whole value (nested elements and entries), so equal
        // signatures mean equal props, not only equal lengths.
        let signature = format!(
            "{:?}",
            props.try_iter().map(|e| e.map_err(|_| ())).collect::<Vec<_>>()
        );
        if signature == self.last {
            return false;
        }
        self.last = signature;
        let malformed = props
            .get("malformed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let keys: Vec<&str> = props.iter().map(|(k, _)| k).collect();
        let mut writer = BinWriter::with_capacity(64);
        writer.map(|m| {
            m.key("count").i64(keys.len() as i64);
            m.key("keys").array(|a| {
                for key in &keys {
                    a.item().str(key);
                }
            });
            m.key("ok").bool(true);
            m.key("ratio").f64(0.5);
        });
        let payload = writer.finish().unwrap_or_default();

        self.emitted.clear();
        if malformed {
            self.emitted.push_custom(
                self.surface_id,
                NodeId::ROOT,
                TimestampUs(0),
                "malformed",
                MALFORMED_PAYLOAD,
            );
        }
        self.emitted.push_custom(
            self.surface_id,
            NodeId::ROOT,
            TimestampUs(0),
            "props",
            &payload,
        );
        true
    }

    fn drain_emitted_events(&mut self) -> ValidatedEvents<'_> {
        std::mem::swap(&mut self.drained, &mut self.emitted);
        self.emitted.clear();
        self.drained.as_validated()
    }
}

nve_engine::export_nve_app!(EchoApp);

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::{BinValueView, NveMapEntry, NveValue};

    #[test]
    fn echo_app_dedupes_props_and_emits_bin_value_and_malformed_events() {
        let mut app = EchoApp::create(SurfaceId::new(7, 0));
        assert!(app.drain_emitted_events().is_empty());

        let entries = [
            NveMapEntry::new("count", NveValue::i64(3)),
            NveMapEntry::new("label", NveValue::from_str("hi")),
        ];
        // SAFETY: `entries` stays live for the duration of `set_props`.
        let props = unsafe { NvePropsRef::from_raw_parts(entries.as_ptr(), entries.len() as u32) };
        assert!(app.set_props(props));
        assert!(!app.set_props(props), "identical props are deduplicated");

        let batch = app.drain_emitted_events();
        assert_eq!(batch.len(), 1);
        let ev = batch.iter().next().expect("emitted props event");
        assert!(ev.is_custom());
        assert_eq!(ev.surface(), SurfaceId::new(7, 0));
        assert_eq!(ev.node(), NodeId::ROOT);
        assert_eq!(ev.custom_name(), "props");
        let val = BinValueView::parse(ev.custom_data()).expect("valid BinValue payload");
        assert_eq!(val.get("count").and_then(|v| v.as_i64()), Some(2));
        assert_eq!(val.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(val.get("ratio").and_then(|v| v.as_f64()), Some(0.5));
        let keys: Vec<&str> = val
            .get("keys")
            .and_then(|v| v.as_array())
            .expect("keys array")
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(keys, vec!["count", "label"]);
        assert!(app.drain_emitted_events().is_empty(), "drain is destructive");

        let malformed_entries = [NveMapEntry::new("malformed", NveValue::bool(true))];
        // SAFETY: `malformed_entries` stays live for the duration of `set_props`.
        let malformed_props = unsafe {
            NvePropsRef::from_raw_parts(malformed_entries.as_ptr(), malformed_entries.len() as u32)
        };
        assert!(app.set_props(malformed_props));
        let mixed = app.drain_emitted_events();
        assert_eq!(mixed.len(), 2);
        let mut iter = mixed.iter();
        let first = iter.next().expect("malformed event");
        let second = iter.next().expect("props event");
        assert_eq!(first.custom_name(), "malformed");
        assert!(BinValueView::parse(first.custom_data()).is_err());
        assert_eq!(second.custom_name(), "props");
        let second_val =
            BinValueView::parse(second.custom_data()).expect("second event payload intact");
        assert_eq!(second_val.get("count").and_then(|v| v.as_i64()), Some(1));
    }
}
