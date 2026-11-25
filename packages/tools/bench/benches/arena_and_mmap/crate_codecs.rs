//! Apples-to-apples adapters for `serde_json`, `rkyv` (0.8), and `musli-zerocopy` (0.1)
//! operating on the exact same Studio Workspace task schema and fixtures as `NVERELOC`
//! and `MmapOffsetArena`.

#![allow(dead_code)]

use std::path::Path;

use musli_zerocopy::{Buf, OwnedBuf, Ref, ZeroCopy};
use nve_engine::mmap_arena::{read_file_bytes, write_file_bytes, ReadOnlyMmap};
use nve_bench::arena_mmap::{fold_task_fields, OwnedTask};
use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};

// ============================================================================
// Baseline 1a (Real `serde_json`)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, SerdeSerialize, SerdeDeserialize)]
pub struct SerdeTask {
    pub id: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub priority: u32,
    pub status: u32,
    pub assignee_id: u32,
    pub flags: u32,
    pub title: String,
    pub body: String,
    pub tag: String,
}

impl From<&OwnedTask> for SerdeTask {
    fn from(t: &OwnedTask) -> Self {
        Self {
            id: t.id,
            created_at_ms: t.created_at_ms,
            updated_at_ms: t.updated_at_ms,
            priority: t.priority,
            status: t.status,
            assignee_id: t.assignee_id,
            flags: t.flags,
            title: t.title.clone(),
            body: t.body.clone(),
            tag: t.tag.clone(),
        }
    }
}

pub fn to_serde_tasks(tasks: &[OwnedTask]) -> Vec<SerdeTask> {
    tasks.iter().map(SerdeTask::from).collect()
}

pub fn write_serde_json(path: &Path, tasks: &[SerdeTask]) -> usize {
    let bytes = serde_json::to_vec(tasks).expect("serde_json encode");
    write_file_bytes(path, &bytes).expect("write serde_json");
    bytes.len()
}

pub fn read_serde_json(path: &Path) -> Vec<SerdeTask> {
    let bytes = read_file_bytes(path).expect("read serde_json");
    serde_json::from_slice(&bytes).expect("serde_json decode")
}

pub fn random_read_serde_tasks(tasks: &[SerdeTask], indices: &[usize]) -> u64 {
    let mut acc = 0u64;
    for &idx in indices {
        let t = &tasks[idx];
        acc = fold_task_fields(
            acc,
            t.id,
            t.updated_at_ms,
            t.priority,
            t.status,
            &t.title,
            &t.tag,
        );
    }
    acc
}

// ============================================================================
// Candidate A (`rkyv` 0.8)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Archive, RkyvSerialize, RkyvDeserialize)]
pub struct RkyvTask {
    pub id: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub priority: u32,
    pub status: u32,
    pub assignee_id: u32,
    pub flags: u32,
    pub title: String,
    pub body: String,
    pub tag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Archive, RkyvSerialize, RkyvDeserialize)]
pub struct RkyvWorkspace {
    pub tasks: Vec<RkyvTask>,
}

pub fn to_rkyv_workspace(tasks: &[OwnedTask]) -> RkyvWorkspace {
    RkyvWorkspace {
        tasks: tasks
            .iter()
            .map(|t| RkyvTask {
                id: t.id,
                created_at_ms: t.created_at_ms,
                updated_at_ms: t.updated_at_ms,
                priority: t.priority,
                status: t.status,
                assignee_id: t.assignee_id,
                flags: t.flags,
                title: t.title.clone(),
                body: t.body.clone(),
                tag: t.tag.clone(),
            })
            .collect(),
    }
}

pub fn write_rkyv_workspace(path: &Path, ws: &RkyvWorkspace) -> usize {
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(ws).expect("rkyv serialize");
    write_file_bytes(path, bytes.as_slice()).expect("write rkyv");
    bytes.len()
}

pub fn open_rkyv_mmap_validated(path: &Path) -> (ReadOnlyMmap, usize) {
    let mmap = ReadOnlyMmap::open(path).expect("mmap rkyv");
    let archived =
        rkyv::access::<ArchivedRkyvWorkspace, rkyv::rancor::Error>(mmap.as_slice())
            .expect("rkyv bytecheck");
    let count = archived.tasks.len();
    (mmap, count)
}

/// # Safety
/// File must contain a valid `rkyv` archive produced by [`write_rkyv_workspace`].
pub unsafe fn open_rkyv_mmap_unchecked(path: &Path) -> (ReadOnlyMmap, usize) {
    let mmap = ReadOnlyMmap::open(path).expect("mmap rkyv");
    // SAFETY: Caller guarantees valid archive bytes and page-aligned mmap base.
    let archived = unsafe { rkyv::access_unchecked::<ArchivedRkyvWorkspace>(mmap.as_slice()) };
    let count = archived.tasks.len();
    (mmap, count)
}

pub fn random_read_rkyv_mmap(mmap: &ReadOnlyMmap, indices: &[usize]) -> u64 {
    // SAFETY: Caller already validated `mmap` via `open_rkyv_mmap_validated`.
    let archived = unsafe { rkyv::access_unchecked::<ArchivedRkyvWorkspace>(mmap.as_slice()) };
    let mut acc = 0u64;
    for &idx in indices {
        let t = &archived.tasks[idx];
        acc = fold_task_fields(
            acc,
            t.id.to_native(),
            t.updated_at_ms.to_native(),
            t.priority.to_native(),
            t.status.to_native(),
            t.title.as_str(),
            t.tag.as_str(),
        );
    }
    acc
}

// ============================================================================
// Candidate B (`musli-zerocopy` 0.1)
// ============================================================================

#[derive(Debug, Clone, Copy, ZeroCopy)]
#[repr(C)]
pub struct MusliTask {
    pub id: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub priority: u32,
    pub status: u32,
    pub assignee_id: u32,
    pub flags: u32,
    pub title: Ref<str>,
    pub body: Ref<str>,
    pub tag: Ref<str>,
}

#[derive(Debug, Clone, Copy, ZeroCopy)]
#[repr(C)]
pub struct MusliWorkspace {
    pub tasks: Ref<[MusliTask]>,
}

pub fn build_musli_workspace(tasks: &[OwnedTask]) -> OwnedBuf {
    let mut est = 64 + tasks.len() * (std::mem::size_of::<MusliTask>() + 200);
    if est < 256 {
        est = 256;
    }
    let mut buf = OwnedBuf::with_capacity(est).expect("musli alloc");
    let root_uninit = buf
        .store_uninit::<MusliWorkspace>()
        .expect("musli store_uninit");

    let mut pod_tasks = Vec::with_capacity(tasks.len());
    for t in tasks {
        let title = buf.store_unsized(t.title.as_str()).expect("musli title");
        let body = buf.store_unsized(t.body.as_str()).expect("musli body");
        let tag = buf.store_unsized(t.tag.as_str()).expect("musli tag");
        pod_tasks.push(MusliTask {
            id: t.id,
            created_at_ms: t.created_at_ms,
            updated_at_ms: t.updated_at_ms,
            priority: t.priority,
            status: t.status,
            assignee_id: t.assignee_id,
            flags: t.flags,
            title,
            body,
            tag,
        });
    }
    let tasks_ref = buf.store_slice(&pod_tasks).expect("musli store_slice");
    buf.load_uninit_mut(root_uninit)
        .expect("musli load_uninit")
        .write(&MusliWorkspace { tasks: tasks_ref });
    buf
}

pub fn write_musli_workspace(path: &Path, tasks: &[OwnedTask]) -> usize {
    let buf = build_musli_workspace(tasks);
    write_file_bytes(path, buf.as_slice()).expect("write musli");
    buf.len()
}

/// Opens `mmap` and performs `musli-zerocopy`'s native O(1) lazy validation (root + tasks slice).
pub fn open_musli_mmap_lazy(path: &Path) -> (ReadOnlyMmap, usize) {
    let mmap = ReadOnlyMmap::open(path).expect("mmap musli");
    let buf = Buf::new(mmap.as_slice());
    let ws = buf.load_at::<MusliWorkspace>(0).expect("musli load root");
    let tasks = buf.load(ws.tasks).expect("musli load tasks");
    let count = tasks.len();
    (mmap, count)
}

/// Opens `mmap` and eagerly validates every task's `title`/`body`/`tag` UTF-8 & bounds.
pub fn open_musli_mmap_full_verify(path: &Path) -> (ReadOnlyMmap, usize) {
    let mmap = ReadOnlyMmap::open(path).expect("mmap musli");
    let buf = Buf::new(mmap.as_slice());
    let ws = buf.load_at::<MusliWorkspace>(0).expect("musli load root");
    let tasks = buf.load(ws.tasks).expect("musli load tasks");
    for t in tasks {
        let _ = buf.load(t.title).expect("musli verify title");
        let _ = buf.load(t.body).expect("musli verify body");
        let _ = buf.load(t.tag).expect("musli verify tag");
    }
    let count = tasks.len();
    (mmap, count)
}

pub fn random_read_musli_mmap(mmap: &ReadOnlyMmap, indices: &[usize]) -> u64 {
    let buf = Buf::new(mmap.as_slice());
    let ws = buf.load_at::<MusliWorkspace>(0).expect("musli root");
    let tasks = buf.load(ws.tasks).expect("musli tasks");
    let mut acc = 0u64;
    for &idx in indices {
        let t = &tasks[idx];
        let title = buf.load(t.title).expect("musli title");
        let tag = buf.load(t.tag).expect("musli tag");
        acc = fold_task_fields(
            acc,
            t.id,
            t.updated_at_ms,
            t.priority,
            t.status,
            title,
            tag,
        );
    }
    acc
}
