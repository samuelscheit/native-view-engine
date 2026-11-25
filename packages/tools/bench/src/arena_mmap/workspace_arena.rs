//! Studio Workspace benchmark adapter directly over production [`nve_engine::MmapOffsetArena`].
//!
//! Stores `WorkspaceRoot` + `ArenaTask` records using [`TrustedUtf8Str`] handles for O(1)
//! zero-rescan hot random reads, double-buffered crash-atomic `commit_root`, and online
//! compaction (`compact_workspace_in_place`).

use std::mem::{align_of, offset_of, size_of};
use std::path::Path;

pub use nve_engine::mmap_arena::{
    ArenaError, ArenaHeaderSlot, ArenaPod, MmapOffsetArena, OffsetPtr, OffsetSlice, OffsetStr,
    TrustedUtf8Str, ARENA_DATA_START, ARENA_MAGIC, ARENA_VERSION,
};

use super::{fold_task_fields, OwnedTask};

/// Root record of a live mmap Studio Workspace stored inside [`MmapOffsetArena`].
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WorkspaceRoot {
    pub workspace_id: u64,
    pub tasks: OffsetSlice<ArenaTask>,
    pub name: TrustedUtf8Str,
}

/// 64-byte, 8-byte-aligned live task record stored directly in `MAP_SHARED` memory.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ArenaTask {
    pub id: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub priority: u32,
    pub status: u32,
    pub assignee_id: u32,
    pub flags: u32,
    pub title: TrustedUtf8Str,
    pub body: TrustedUtf8Str,
    pub tag: TrustedUtf8Str,
}

// SAFETY: `#[repr(C)]` zero-padding POD structs composed solely of `ArenaPod` fields.
unsafe impl ArenaPod for WorkspaceRoot {}
unsafe impl ArenaPod for ArenaTask {}

const _: () = {
    assert!(size_of::<WorkspaceRoot>() == 24);
    assert!(align_of::<WorkspaceRoot>() == 8);
    assert!(offset_of!(WorkspaceRoot, workspace_id) == 0);
    assert!(offset_of!(WorkspaceRoot, tasks) == 8);
    assert!(offset_of!(WorkspaceRoot, name) == 16);

    assert!(size_of::<ArenaTask>() == 64);
    assert!(align_of::<ArenaTask>() == 8);
    assert!(offset_of!(ArenaTask, id) == 0);
    assert!(offset_of!(ArenaTask, created_at_ms) == 8);
    assert!(offset_of!(ArenaTask, updated_at_ms) == 16);
    assert!(offset_of!(ArenaTask, priority) == 24);
    assert!(offset_of!(ArenaTask, status) == 28);
    assert!(offset_of!(ArenaTask, assignee_id) == 32);
    assert!(offset_of!(ArenaTask, flags) == 36);
    assert!(offset_of!(ArenaTask, title) == 40);
    assert!(offset_of!(ArenaTask, body) == 48);
    assert!(offset_of!(ArenaTask, tag) == 56);
};

/// Creates and populates a complete Studio Workspace inside a file-backed [`MmapOffsetArena`].
pub fn populate_workspace_arena(
    path: &Path,
    workspace_id: u64,
    name: &str,
    tasks: &[OwnedTask],
    extra_headroom_bytes: usize,
) -> Result<MmapOffsetArena, ArenaError> {
    let mut needed = ARENA_DATA_START
        + size_of::<WorkspaceRoot>()
        + name.len()
        + 64
        + tasks.len() * (size_of::<ArenaTask>() + 8);
    for t in tasks {
        needed += t.title.len() + t.body.len() + t.tag.len();
    }
    needed = needed
        .checked_add(extra_headroom_bytes)
        .ok_or(ArenaError::Overflow)?;

    let mut arena = MmapOffsetArena::create(path, needed)?;
    let name_str = arena.alloc_str(name)?;

    let mut pod_tasks = Vec::with_capacity(tasks.len());
    for t in tasks {
        let title = arena.alloc_str(&t.title)?;
        let body = arena.alloc_str(&t.body)?;
        let tag = arena.alloc_str(&t.tag)?;
        pod_tasks.push(ArenaTask {
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
    let tasks_slice = arena.alloc_slice_copy(&pod_tasks)?;
    let root_ptr = arena.alloc_val(WorkspaceRoot {
        workspace_id,
        tasks: tasks_slice,
        name: name_str,
    })?;
    arena.commit_root_buffered(root_ptr)?;
    Ok(arena)
}

#[inline]
pub fn workspace_root(arena: &MmapOffsetArena) -> Result<&WorkspaceRoot, ArenaError> {
    arena.get(arena.root_ptr::<WorkspaceRoot>())
}

#[inline]
pub fn workspace_tasks(arena: &MmapOffsetArena) -> Result<&[ArenaTask], ArenaError> {
    let root = *workspace_root(arena)?;
    arena.slice(root.tasks)
}

#[inline]
pub fn workspace_tasks_mut(arena: &mut MmapOffsetArena) -> Result<&mut [ArenaTask], ArenaError> {
    let tasks_slice = workspace_root(arena)?.tasks;
    arena.slice_mut(tasks_slice)
}

/// Mutates a task's status, priority, and timestamp directly in-place in `MAP_SHARED` memory.
/// Returns the byte offset of the mutated [`ArenaTask`] for targeted page `msync`.
#[inline]
pub fn mutate_task_in_place(
    arena: &mut MmapOffsetArena,
    index: usize,
    new_status: u32,
    new_priority: u32,
    updated_at_ms: i64,
) -> Result<usize, ArenaError> {
    let tasks_slice = workspace_root(arena)?.tasks;
    let tasks = arena.slice_mut(tasks_slice)?;
    let task = tasks.get_mut(index).ok_or(ArenaError::OutOfBounds {
        offset: index,
        len: 1,
        limit: tasks_slice.len as usize,
    })?;
    task.status = new_status;
    task.priority = new_priority;
    task.updated_at_ms = updated_at_ms;
    Ok(tasks_slice.offset as usize + index * size_of::<ArenaTask>())
}

/// Runs the apples-to-apples random-read workload directly over `MAP_SHARED` memory using
/// O(1) [`TrustedUtf8Str`] reads.
pub fn random_read_workspace_arena(
    arena: &MmapOffsetArena,
    indices: &[usize],
) -> Result<u64, ArenaError> {
    let tasks = workspace_tasks(arena)?;
    let mut acc = 0u64;
    for &idx in indices {
        let t = &tasks[idx];
        let title = arena.str_at(t.title)?;
        let tag = arena.str_at(t.tag)?;
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
    Ok(acc)
}
