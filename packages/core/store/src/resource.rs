//! `ResourceStore`: refcounted, `xxh64`-deduplicated variable-length blobs (SVG pictures, draw
//! lists, shadow lists, gradients, state rules, transitions, attribute maps) referenced from row
//! words by `ResourceId`.
//!
//! In-process backends read the typed value directly (zero encode/decode); values are encoded
//! only for dedup hashing and when a frame crosses a wire boundary. `EPHEMERAL` resources skip
//! hashing and are rewritten in place (120 Hz live canvas draw lists).
//!
//! Replica stores (wire / `NVE2` receivers such as the host runtime) keep every defined
//! resource's wire bytes, so hosts can read them zero-copy ([`ResourceStore::bytes`]). A kind
//! without a codec registered in this process is mirrored as raw bytes instead of rejecting the
//! frame: typed reads miss, bytes and re-encoding still work.

use std::any::{Any, TypeId};
use std::sync::{Arc, LazyLock, RwLock};

use nve_abi::Key64;
use nve_hash::xxh64;
use rustc_hash::FxHashMap;

use crate::id::ResourceId;

const RES_PENDING_FREE: u8 = 1 << 0;
const RES_IN_FREE: u8 = 1 << 1;
const RES_DEFINED_THIS_FRAME: u8 = 1 << 2;
const RES_CREATED_THIS_FRAME: u8 = 1 << 3;
const RES_RELEASED_THIS_FRAME: u8 = 1 << 4;
/// `Entry::encoded` holds the current value's wire bytes.
const RES_HAS_BYTES: u8 = 1 << 5;

/// Value of a replica entry whose kind has no codec in this process (bytes live in `encoded`).
struct RawResource;

static RAW_RESOURCE: LazyLock<AnyArc> = LazyLock::new(|| Arc::new(RawResource));

/// A typed blob that can live in the [`ResourceStore`] and cross the wire.
pub trait Resource: Send + Sync + 'static {
    /// Stable kind identity (`Key64::of("nve.box_shadows")`).
    const KIND: Key64;
    fn encode(&self, out: &mut Vec<u8>);
    fn decode(bytes: &[u8]) -> Option<Self>
    where
        Self: Sized;
}

type AnyArc = Arc<dyn Any + Send + Sync>;
type EncodeFn = fn(&(dyn Any + Send + Sync), &mut Vec<u8>);
type DecodeFn = fn(&[u8]) -> Option<AnyArc>;

#[derive(Clone, Copy)]
struct Codec {
    type_id: TypeId,
    encode: EncodeFn,
    decode: DecodeFn,
}

static CODECS: RwLock<Vec<(u64, Codec)>> = RwLock::new(Vec::new());

#[expect(
    clippy::disallowed_methods,
    reason = "Track 1B-1d ResourceStore type-erased codec boundary"
)]
fn encode_erased<R: Resource>(v: &(dyn Any + Send + Sync), out: &mut Vec<u8>) {
    if let Some(r) = v.downcast_ref::<R>() {
        r.encode(out);
    }
}

fn decode_erased<R: Resource>(bytes: &[u8]) -> Option<AnyArc> {
    R::decode(bytes).map(|r| Arc::new(r) as AnyArc)
}

/// Registers `R`'s codec so frames carrying it can be encoded/applied. Idempotent for the same
/// Rust type; panics if a different Rust type collides on `R::KIND`.
pub fn register_codec<R: Resource>() {
    let tid = TypeId::of::<R>();
    let mut reg = CODECS.write().unwrap_or_else(|e| e.into_inner());
    if let Some((_, existing)) = reg.iter().find(|(k, _)| *k == R::KIND.0) {
        assert_eq!(
            existing.type_id, tid,
            "Resource KIND collision on {:?} between distinct Rust types",
            R::KIND
        );
        return;
    }
    reg.push((
        R::KIND.0,
        Codec {
            type_id: tid,
            encode: encode_erased::<R>,
            decode: decode_erased::<R>,
        },
    ));
}

fn codec(kind: Key64) -> Option<Codec> {
    let reg = CODECS.read().unwrap_or_else(|e| e.into_inner());
    reg.iter().find(|(k, _)| *k == kind.0).map(|(_, c)| *c)
}

struct Entry {
    kind: Key64,
    refs: u32,
    hash: u64,
    ephemeral: bool,
    flags: u8,
    value: Option<AnyArc>,
    /// Wire bytes when `RES_HAS_BYTES` is set: interned entries (dedup + wire sends skip
    /// re-encoding) and every replica entry (zero-copy host reads).
    encoded: Box<[u8]>,
}

/// Overwrites `slot` with `bytes`, reusing its allocation when the length is unchanged.
fn store_bytes(slot: &mut Box<[u8]>, bytes: &[u8]) {
    if slot.len() == bytes.len() {
        slot.copy_from_slice(bytes);
    } else {
        *slot = bytes.into();
    }
}

/// Refcounted blob store. Index `0` is reserved for [`ResourceId::NONE`].
pub struct ResourceStore {
    entries: Vec<Entry>,
    dedup: FxHashMap<(u64, u64), u32>,
    free: Vec<u32>,
    pending_free: Vec<u32>,
    pub(crate) defined: Vec<ResourceId>,
    pub(crate) released: Vec<ResourceId>,
    scratch: Vec<u8>,
}

impl Default for ResourceStore {
    fn default() -> Self {
        Self {
            entries: vec![Entry {
                kind: Key64(0),
                refs: 0,
                hash: 0,
                ephemeral: false,
                flags: 0,
                value: None,
                encoded: Box::new([]),
            }],
            dedup: FxHashMap::default(),
            free: Vec::new(),
            pending_free: Vec::new(),
            defined: Vec::new(),
            released: Vec::new(),
            scratch: Vec::new(),
        }
    }
}

impl ResourceStore {
    fn slot(&mut self) -> u32 {
        while let Some(i) = self.free.pop() {
            if let Some(e) = self.entries.get_mut(i as usize) {
                let was_in_free = e.flags & RES_IN_FREE != 0;
                e.flags &= !RES_IN_FREE;
                if was_in_free
                    && e.refs == 0
                    && e.value.is_none()
                    && (e.flags & RES_PENDING_FREE) == 0
                {
                    return i;
                }
            }
        }
        self.entries.push(Entry {
            kind: Key64(0),
            refs: 0,
            hash: 0,
            ephemeral: false,
            flags: 0,
            value: None,
            encoded: Box::new([]),
        });
        (self.entries.len() - 1) as u32
    }

    fn insert(
        &mut self,
        kind: Key64,
        hash: u64,
        ephemeral: bool,
        value: AnyArc,
        encoded: Box<[u8]>,
    ) -> ResourceId {
        let i = self.slot();
        let bytes_flag = if ephemeral { 0 } else { RES_HAS_BYTES };
        self.entries[i as usize] = Entry {
            kind,
            refs: 1,
            hash,
            ephemeral,
            flags: RES_DEFINED_THIS_FRAME | RES_CREATED_THIS_FRAME | bytes_flag,
            value: Some(value),
            encoded,
        };
        if !ephemeral {
            self.dedup.entry((kind.0, hash)).or_insert(i);
        }
        self.defined.push(ResourceId(i));
        ResourceId(i)
    }

    /// Interns `value`, returning an id holding one new reference. Equal content (same kind,
    /// `xxh64` hash, AND verified encoded-byte equality against cached bytes) shares one id
    /// without re-encoding the existing entry.
    pub fn intern<R: Resource>(&mut self, value: R) -> ResourceId {
        self.scratch.clear();
        value.encode(&mut self.scratch);
        let hash = xxh64(&self.scratch, R::KIND.0);
        if let Some(&i) = self.dedup.get(&(R::KIND.0, hash))
            && let Some(e) = self.entries.get_mut(i as usize)
            && e.value.as_deref().is_some_and(|v| v.is::<R>())
            && e.encoded.as_ref() == self.scratch.as_slice()
        {
            e.refs += 1;
            return ResourceId(i);
        }
        register_codec::<R>();
        let encoded: Box<[u8]> = self.scratch.as_slice().into();
        self.insert(R::KIND, hash, false, Arc::new(value), encoded)
    }

    /// Replaces the value of ephemeral `old` in place when it is uniquely owned (no hashing, no
    /// new id, and zero `Arc::new` allocation when the `Arc` has no external clones); otherwise
    /// allocates a fresh ephemeral id. Returns the id now holding the value.
    pub fn set_ephemeral<R: Resource>(&mut self, old: ResourceId, value: R) -> ResourceId {
        if let Some(e) = self.entries.get_mut(old.0 as usize)
            && !old.is_none()
            && e.ephemeral
            && e.refs == 1
            && e.kind == R::KIND
        {
            if let Some(arc) = e.value.as_mut()
                && let Some(any_mut) = Arc::get_mut(arc)
                && let Some(slot) = any_mut.downcast_mut::<R>()
            {
                *slot = value;
            } else {
                e.value = Some(Arc::new(value));
            }
            // The typed value changed in place: any mirrored bytes are stale.
            e.flags &= !RES_HAS_BYTES;
            if e.flags & RES_DEFINED_THIS_FRAME == 0 {
                e.flags |= RES_DEFINED_THIS_FRAME;
                self.defined.push(old);
            }
            return old;
        }
        register_codec::<R>();
        self.insert(R::KIND, 0, true, Arc::new(value), Box::new([]))
    }

    /// Adds a reference to `id`.
    #[inline]
    pub fn retain(&mut self, id: ResourceId) {
        if !id.is_none()
            && let Some(e) = self.entries.get_mut(id.0 as usize)
        {
            e.refs += 1;
        }
    }

    /// Drops a reference; the slot is recycled at the next frame boundary.
    pub fn release(&mut self, id: ResourceId) {
        if id.is_none() {
            return;
        }
        let Some(e) = self.entries.get_mut(id.0 as usize) else {
            return;
        };
        debug_assert!(e.refs > 0, "resource {id:?} over-released");
        if e.refs == 0 {
            return;
        }
        e.refs -= 1;
        if e.refs == 0 {
            if !e.ephemeral
                && self.dedup.get(&(e.kind.0, e.hash)).copied() == Some(id.0)
            {
                self.dedup.remove(&(e.kind.0, e.hash));
            }
            if e.flags & RES_PENDING_FREE == 0 {
                e.flags |= RES_PENDING_FREE;
                self.pending_free.push(id.0);
            }
            if e.flags & RES_CREATED_THIS_FRAME == 0 && e.flags & RES_RELEASED_THIS_FRAME == 0 {
                e.flags |= RES_RELEASED_THIS_FRAME;
                self.released.push(id);
            }
        }
    }

    /// Typed value of `id`.
    #[inline]
    #[expect(
        clippy::disallowed_methods,
        reason = "Track 1B-1d ResourceStore type-erased read boundary"
    )]
    pub fn get<R: Resource>(&self, id: ResourceId) -> Option<&R> {
        self.entries
            .get(id.0 as usize)?
            .value
            .as_deref()?
            .downcast_ref::<R>()
    }

    /// Shared handle to the value of `id` (cheap clone for backends that cache it).
    #[inline]
    pub fn get_arc<R: Resource>(&self, id: ResourceId) -> Option<Arc<R>> {
        self.entries
            .get(id.0 as usize)?
            .value
            .clone()?
            .downcast::<R>()
            .ok()
    }

    pub fn kind(&self, id: ResourceId) -> Option<Key64> {
        self.entries.get(id.0 as usize).map(|e| e.kind)
    }

    pub fn is_live(&self, id: ResourceId) -> bool {
        self.entries.get(id.0 as usize).is_some_and(|e| e.refs > 0)
    }

    /// Wire encoding of `id` (`false` when it holds no value or a typed value without a codec).
    pub fn encode(&self, id: ResourceId, out: &mut Vec<u8>) -> bool {
        let Some(e) = self.entries.get(id.0 as usize) else {
            return false;
        };
        if e.value.is_some() && e.flags & RES_HAS_BYTES != 0 {
            out.extend_from_slice(&e.encoded);
            return true;
        }
        match (codec(e.kind), e.value.as_deref()) {
            (Some(c), Some(v)) => {
                (c.encode)(v, out);
                true
            }
            _ => false,
        }
    }

    /// Zero-copy wire bytes of defined `id`, when this store holds them (replica entries,
    /// interned entries; not in-process ephemeral values, which are never encoded).
    pub fn bytes(&self, id: ResourceId) -> Option<&[u8]> {
        let e = self.entries.get(id.0 as usize)?;
        (e.value.is_some() && e.flags & RES_HAS_BYTES != 0).then_some(&*e.encoded)
    }

    /// Number of resource slots (live, free, and the reserved slot `0`); every `ResourceId` is
    /// below it.
    pub fn slots(&self) -> usize {
        self.entries.len()
    }

    /// Whether `id` holds a value (defined and not yet recycled).
    pub(crate) fn is_defined(&self, id: ResourceId) -> bool {
        self.entries.get(id.0 as usize).is_some_and(|e| e.value.is_some())
    }

    pub fn is_ephemeral(&self, id: ResourceId) -> bool {
        self.entries.get(id.0 as usize).is_some_and(|e| e.ephemeral)
    }

    /// Mirrors a resource defined by a wire frame at exactly `id` (replica stores), keeping its
    /// bytes. A kind without a codec here is mirrored raw; a known kind must decode.
    pub(crate) fn define_at(
        &mut self,
        id: ResourceId,
        kind: Key64,
        ephemeral: bool,
        bytes: &[u8],
    ) -> bool {
        let value = match codec(kind) {
            Some(c) => match (c.decode)(bytes) {
                Some(v) => v,
                None => return false,
            },
            None => Arc::clone(&RAW_RESOURCE),
        };
        let i = id.0 as usize;
        if i == 0 {
            return false;
        }
        while self.entries.len() <= i {
            self.entries.push(Entry {
                kind: Key64(0),
                refs: 0,
                hash: 0,
                ephemeral: false,
                flags: 0,
                value: None,
                encoded: Box::new([]),
            });
        }
        let prev = &self.entries[i];
        if !prev.ephemeral
            && prev.value.is_some()
            && self.dedup.get(&(prev.kind.0, prev.hash)).copied() == Some(id.0)
        {
            self.dedup.remove(&(prev.kind.0, prev.hash));
        }
        let e = &mut self.entries[i];
        let refs = e.refs;
        let already_defined = e.flags & RES_DEFINED_THIS_FRAME != 0;
        let was_pending = e.flags & RES_PENDING_FREE != 0;
        let preserved = e.flags & (RES_IN_FREE | RES_PENDING_FREE | RES_RELEASED_THIS_FRAME);
        let hash = if ephemeral { 0 } else { xxh64(bytes, kind.0) };
        let mut encoded = std::mem::take(&mut e.encoded);
        store_bytes(&mut encoded, bytes);
        let mut flags = RES_DEFINED_THIS_FRAME | RES_HAS_BYTES | preserved;
        if refs == 0 {
            flags |= RES_PENDING_FREE;
        }
        *e = Entry {
            kind,
            refs,
            hash,
            ephemeral,
            flags,
            value: Some(value),
            encoded,
        };
        if refs == 0 && !was_pending {
            self.pending_free.push(id.0);
        }
        if !ephemeral {
            self.dedup.entry((kind.0, hash)).or_insert(id.0);
        }
        if !already_defined {
            self.defined.push(id);
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn free_len(&self) -> usize {
        self.free.len()
    }

    #[cfg(test)]
    pub(crate) fn pending_free_len(&self) -> usize {
        self.pending_free.len()
    }

    /// Frees slots released during the previous frame and drops their values.
    pub(crate) fn end_frame(&mut self) {
        for &id in &self.defined {
            if let Some(e) = self.entries.get_mut(id.0 as usize) {
                e.flags &= !(RES_DEFINED_THIS_FRAME | RES_CREATED_THIS_FRAME);
            }
        }
        for &id in &self.released {
            if let Some(e) = self.entries.get_mut(id.0 as usize) {
                e.flags &= !RES_RELEASED_THIS_FRAME;
            }
        }
        for i in self.pending_free.drain(..) {
            let e = &mut self.entries[i as usize];
            e.flags &= !(RES_PENDING_FREE | RES_CREATED_THIS_FRAME | RES_RELEASED_THIS_FRAME);
            if e.refs == 0 {
                if !e.ephemeral
                    && e.value.is_some()
                    && self.dedup.get(&(e.kind.0, e.hash)).copied() == Some(i)
                {
                    self.dedup.remove(&(e.kind.0, e.hash));
                }
                e.value = None;
                e.encoded = Box::new([]);
                if e.flags & RES_IN_FREE == 0 {
                    e.flags |= RES_IN_FREE;
                    self.free.push(i);
                }
            }
        }
        self.defined.clear();
        self.released.clear();
    }

    /// Number of live resources.
    pub fn live(&self) -> usize {
        self.entries.iter().filter(|e| e.refs > 0).count()
    }
}
