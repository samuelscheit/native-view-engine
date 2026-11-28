//! `libnve_host_fixture_app`: a minimal `nve_get_interface` app for host FFI suites (Dart,
//! Swift, Java, C++) that must see real app-emitted events cross the runtime C ABI.
//!
//! `set_props` returns `true` whenever the prop keys or values differ from the previous call and
//! queues one `Custom` event named `props` (node 1) whose value is the structured map
//! `{count: i64, keys: [str…], ok: true, ratio: 0.5}`. A truthy `malformed` prop first queues a
//! `Custom` event whose name is invalid UTF-8 (`bad\xFFname`), so hosts prove one malformed
//! event never drops the rest of the drained batch.

use nve_abi::{NveDirectEvent, NveEventBatchRef, NveMapEntry, NvePropsRef, NveStrSlice, NveValue};
use nve_engine::NveApp;

static MALFORMED_NAME: &[u8] = b"bad\xFFname";

#[derive(Default)]
struct EchoApp {
    surface_id: u32,
    /// `key=value` signature of the last props, to report "changed".
    last: String,
    /// Storage the queued events' values borrow; rebuilt only by `set_props` (the next call
    /// after a drain, so drained borrows stay valid until then).
    keys: Vec<String>,
    key_values: Vec<NveValue>,
    entries: Vec<NveMapEntry>,
    emitted: Vec<NveDirectEvent>,
    drained: Vec<NveDirectEvent>,
}

impl NveApp for EchoApp {
    fn create(surface_id: u32) -> Self {
        Self { surface_id, ..Self::default() }
    }

    fn render(&mut self, _frame_id: u64) -> &[u8] {
        &[]
    }

    fn dispatch_events(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        false
    }

    fn set_props(&mut self, props: NvePropsRef<'_>) -> bool {
        // `NveValueRef`'s `Debug` prints the whole value (nested elements and entries), so equal
        // signatures mean equal props, not only equal lengths.
        let signature = format!("{:?}", props.try_iter().map(|e| e.map_err(|_| ())).collect::<Vec<_>>());
        if signature == self.last {
            return false;
        }
        self.last = signature;
        let malformed = props.get("malformed").and_then(|v| v.as_bool()).unwrap_or(false);
        self.emitted.clear();
        self.keys = props.iter().map(|(k, _)| k.to_owned()).collect();
        self.key_values = self.keys.iter().map(|k| NveValue::from_str(k)).collect();
        self.entries = vec![
            NveMapEntry::new("count", NveValue::i64(self.keys.len() as i64)),
            NveMapEntry::new("keys", NveValue::array(&self.key_values)),
            NveMapEntry::new("ok", NveValue::bool(true)),
            NveMapEntry::new("ratio", NveValue::f64(0.5)),
        ];
        if malformed {
            let bad = NveStrSlice { ptr: MALFORMED_NAME.as_ptr(), len: MALFORMED_NAME.len() };
            self.emitted.push(NveDirectEvent {
                name: bad,
                ..NveDirectEvent::custom(self.surface_id, 1, 0, "", NveValue::null())
            });
        }
        self.emitted.push(NveDirectEvent::custom(self.surface_id, 1, 0, "props", NveValue::map(&self.entries)));
        true
    }

    fn drain_emitted_events(&mut self) -> &[NveDirectEvent] {
        self.drained = std::mem::take(&mut self.emitted);
        &self.drained
    }
}

nve_engine::export_nve_app!(EchoApp);
