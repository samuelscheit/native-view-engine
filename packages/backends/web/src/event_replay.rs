//! Pre-hydration global event capture (`<script data-nve-replay>`) and atomic replay into
//! [`EventBatchBuf`] on hydration handoff.
//!
//! Policy before WASM boots:
//! - `<a href="...">` links (with non-empty href != `#`) are NEVER intercepted: browser navigates
//!   natively at 0ms.
//! - `<input>` / `<textarea>` / `<select>` typing is NEVER `preventDefault()`'d: browser updates
//!   `.value` at 0ms and 3-way `StateReconciler` preserves dirty controls on hydration.
//! - Interactive non-link clicks, form submits, and keyboard shortcuts (`Meta`/`Ctrl`/`Escape`/
//!   `Enter`) are captured in a bounded queue (`MAX_REPLAY_QUEUE_LEN = 64`).
//! - `window.__nveReplayDrain()` atomically detaches every listener, freezes & clears the queue,
//!   and returns the captured events once — guaranteeing zero double-firing across hydration.

use nve_abi::{EventBatchBuf, EventFlags, NodeId, SurfaceId, TimestampUs};

/// Maximum age (in milliseconds) of a pre-hydration event eligible for replay.
/// Events older than this threshold are dropped as stale.
pub const MAX_REPLAY_AGE_MS: u64 = 10_000;

/// Maximum number of pre-hydration events buffered by `<script data-nve-replay>`.
pub const MAX_REPLAY_QUEUE_LEN: usize = 64;

/// Minified, self-contained inline `<script data-nve-replay>` tag (< 1 KB) emitted right after
/// `<div id="app">...</div>` when [`crate::SsrDocumentOptions::enable_event_replay`] is `true`.
pub const PRE_HYDRATION_REPLAY_SCRIPT_TAG: &str = concat!(
    "<script data-nve-replay>",
    "(()=>{let W=window,a=document.getElementById('app'),q=[],d=0,K='keydown';if(a){",
    "let G=e=>(e=e.composedPath?.()[0]||e.target)?.closest?e:e?.parentElement,",
    "I=t=>(t=t?.closest('[data-nve-id]'))&&a.contains(t.getRootNode().host||t)?+t.dataset.nveId:0,",
    "P=o=>!d&&q.length<64&&(o.t=Date.now(),q.push(o)),",
    "R=(e,n,f)=>{let{key:k,metaKey:m,ctrlKey:c}=e;",
    "(m||c||k=='Escape'||f&&k=='Enter')&&(e.__n=1,(m||c)&&/^k$/i.test(k)&&e.preventDefault(),",
    "P({k:'k',id:n,key:k,m,c,sh:e.shiftKey,a:e.altKey}))},",
    "L=[['click',e=>{let t=G(e),h=t?.closest('a[href]')?.getAttribute('href').trim(),n=I(t);",
    "(!h||h=='#')&&n&&P({k:'p',id:n,x:e.clientX||0,y:e.clientY||0})}],",
    "[K,e=>R(e,I(G(e))||1,1)],",
    "['submit',e=>{e.preventDefault();e=G(e);",
    "P({k:'s',id:I(e)||1,v:(e?.querySelector('input,textarea,select')||e)?.value||''})}],",
    "['change',e=>{e=G(e)?.dataset;e&&(e.nveDirty=1)}],",
    "[K,e=>!e.__n&&R(e,1),W]],",
    "T=s=>L.map(([e,f,t])=>(t||a)[s+'EventListener'](e,f));",
    "T('add');W.__nveReplayDrain=()=>d?[]:(d=1,T('remove'),q.splice(0))}})();",
    "</script>"
);

const _: () = {
    assert!(PRE_HYDRATION_REPLAY_SCRIPT_TAG.len() < 1024);
};

/// Returns the pre-hydration `<script data-nve-replay>` tag bound to `mount_id` (reusing
/// [`PRE_HYDRATION_REPLAY_SCRIPT_TAG`] verbatim when `mount_id == "app"`).
pub fn pre_hydration_replay_script_tag(mount_id: &str) -> String {
    let sanitized: String = mount_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    if sanitized.is_empty() || sanitized == "app" {
        return PRE_HYDRATION_REPLAY_SCRIPT_TAG.to_owned();
    }
    PRE_HYDRATION_REPLAY_SCRIPT_TAG.replacen(
        "getElementById('app')",
        &format!("getElementById('{sanitized}')"),
        1,
    )
}

/// Typed pre-hydration user interaction drained atomically from `window.__nveReplayDrain()`.
#[derive(Debug, Clone, PartialEq)]
pub enum CapturedPreHydrationEvent {
    Press {
        node_id: NodeId,
        x: f32,
        y: f32,
        age_ms: u64,
    },
    KeyDown {
        node_id: NodeId,
        key: String,
        meta: bool,
        ctrl: bool,
        shift: bool,
        alt: bool,
        age_ms: u64,
    },
    Submit {
        node_id: NodeId,
        value: String,
        age_ms: u64,
    },
}

impl CapturedPreHydrationEvent {
    #[inline]
    pub fn node_id(&self) -> NodeId {
        match self {
            Self::Press { node_id, .. }
            | Self::KeyDown { node_id, .. }
            | Self::Submit { node_id, .. } => *node_id,
        }
    }

    /// Returns this event retargeted at `node_id` (SSR -> client `NodeId` remapping).
    #[inline]
    pub fn with_node_id(mut self, node_id: NodeId) -> Self {
        match &mut self {
            Self::Press { node_id: n, .. }
            | Self::KeyDown { node_id: n, .. }
            | Self::Submit { node_id: n, .. } => *n = node_id,
        }
        self
    }

    #[inline]
    pub fn age_ms(&self) -> u64 {
        match self {
            Self::Press { age_ms, .. }
            | Self::KeyDown { age_ms, .. }
            | Self::Submit { age_ms, .. } => *age_ms,
        }
    }
}

/// Pure-Rust pre-hydration replay filter: preserves chronological order while dropping events
/// older than [`MAX_REPLAY_AGE_MS`] or targeting nodes absent from the hydrated tree.
pub fn drain_valid_pre_hydration_events(
    events: Vec<CapturedPreHydrationEvent>,
    mut node_exists: impl FnMut(NodeId) -> bool,
) -> Vec<CapturedPreHydrationEvent> {
    events
        .into_iter()
        .filter(|ev| ev.age_ms() <= MAX_REPLAY_AGE_MS && node_exists(ev.node_id()))
        .collect()
}

/// Appends a validated [`CapturedPreHydrationEvent`] into `out`.
pub fn push_pre_hydration_event(
    ev: &CapturedPreHydrationEvent,
    surface_id: SurfaceId,
    timestamp: TimestampUs,
    target_node_id: NodeId,
    out: &mut EventBatchBuf,
) {
    match ev {
        CapturedPreHydrationEvent::Press { x, y, .. } => {
            out.push_press(surface_id, target_node_id, timestamp, *x, *y);
        }
        CapturedPreHydrationEvent::KeyDown {
            key,
            meta,
            ctrl,
            shift,
            alt,
            ..
        } => {
            let flags = EventFlags::from_modifiers(*meta, *ctrl, *shift, *alt, false);
            out.push_key_down(surface_id, target_node_id, timestamp, key, "", flags, 0);
        }
        CapturedPreHydrationEvent::Submit { value, .. } => {
            out.push_submit(surface_id, target_node_id, timestamp, value);
        }
    }
}

#[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
export function nve_drain_pre_hydration_replay(push_press, push_key, push_submit) {
  try {
    const fn = window && window.__nveReplayDrain;
    if (typeof fn !== 'function') return;
    const now = Date.now();
    const list = fn() || [];
    for (let i = 0; i < list.length; i++) {
      const e = list[i];
      if (!e || typeof e !== 'object') continue;
      const age = Math.max(0, now - (Number(e.t) || 0));
      const id = (Number(e.id) || 0) >>> 0;
      if (e.k === 'p') {
        push_press(id, Number(e.x) || 0, Number(e.y) || 0, age);
      } else if (e.k === 'k') {
        push_key(id, String(e.key || ''), !!e.m, !!e.c, !!e.sh, !!e.a, age);
      } else if (e.k === 's') {
        push_submit(id, String(e.v || ''), age);
      }
    }
  } catch (_) {}
}
")]
extern "C" {
    fn nve_drain_pre_hydration_replay(
        push_press: &mut dyn FnMut(u32, f32, f32, f64),
        push_key: &mut dyn FnMut(u32, String, bool, bool, bool, bool, f64),
        push_submit: &mut dyn FnMut(u32, String, f64),
    );
}

/// Atomically invokes `window.__nveReplayDrain()` on `wasm32`, detaching pre-hydration listeners
/// and returning all captured events as typed [`CapturedPreHydrationEvent`] values.
#[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
pub(crate) fn drain_browser_pre_hydration_events() -> Vec<CapturedPreHydrationEvent> {
    let out = std::cell::RefCell::new(Vec::new());
    let mut push_press = |node_id: u32, x: f32, y: f32, age_ms: f64| {
        let Some(node_id) = NodeId::from_raw(node_id) else {
            return;
        };
        out.borrow_mut().push(CapturedPreHydrationEvent::Press {
            node_id,
            x,
            y,
            age_ms: if age_ms.is_finite() && age_ms >= 0.0 {
                age_ms as u64
            } else {
                u64::MAX
            },
        });
    };
    let mut push_key =
        |node_id: u32, key: String, meta: bool, ctrl: bool, shift: bool, alt: bool, age_ms: f64| {
            let Some(node_id) = NodeId::from_raw(node_id) else {
                return;
            };
            out.borrow_mut().push(CapturedPreHydrationEvent::KeyDown {
                node_id,
                key,
                meta,
                ctrl,
                shift,
                alt,
                age_ms: if age_ms.is_finite() && age_ms >= 0.0 {
                    age_ms as u64
                } else {
                    u64::MAX
                },
            });
        };
    let mut push_submit = |node_id: u32, value: String, age_ms: f64| {
        let Some(node_id) = NodeId::from_raw(node_id) else {
            return;
        };
        out.borrow_mut().push(CapturedPreHydrationEvent::Submit {
            node_id,
            value,
            age_ms: if age_ms.is_finite() && age_ms >= 0.0 {
                age_ms as u64
            } else {
                u64::MAX
            },
        });
    };
    nve_drain_pre_hydration_replay(&mut push_press, &mut push_key, &mut push_submit);
    out.into_inner()
}
