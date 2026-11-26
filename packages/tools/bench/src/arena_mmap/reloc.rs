//! Workload A Candidate 1: Studio Workspace task schema adapter directly over production
//! [`nve_engine::reloc`] (`NVERELOC` 64-byte header + XXH64 checksum + `RelocSlice<T>`).

use std::mem::{align_of, offset_of, size_of};
use std::path::Path;

use nve_engine::mmap_arena::{write_file_bytes, ArenaPod};
pub use nve_engine::reloc::{
    compute_reloc_checksum, MmapReloc, RelocBuf, RelocError, RelocHeader, RelocSlice, RelocStr,
    RelocView, RelocWriter, VerifyMode, RELOC_KIND_WORKSPACE, RELOC_MAGIC, RELOC_VERSION,
};

use super::{fold_task_fields, OwnedTask};

/// Fixed-size, 8-byte-aligned, zero-padding POD representation of a Studio Workspace task (64 B).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskPod {
    pub id: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub priority: u32,
    pub status: u32,
    pub assignee_id: u32,
    pub flags: u32,
    pub title: RelocStr,
    pub body: RelocStr,
    pub tag: RelocStr,
}

/// 16-byte root record stored inside `NVERELOC` (`RELOC_KIND_WORKSPACE`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskWorkspaceRoot {
    pub tasks: RelocSlice<TaskPod>,
    pub reserved: u64,
}

// SAFETY: `TaskPod` is `#[repr(C)]` (64 bytes) with compile-time layout assertions and zero padding.
unsafe impl ArenaPod for TaskPod {}
// SAFETY: `TaskWorkspaceRoot` is `#[repr(C)]` (16 bytes) with compile-time layout assertions and zero padding.
unsafe impl ArenaPod for TaskWorkspaceRoot {}

const _: () = {
    assert!(size_of::<TaskPod>() == 64);
    assert!(align_of::<TaskPod>() == 8);
    assert!(offset_of!(TaskPod, id) == 0);
    assert!(offset_of!(TaskPod, created_at_ms) == 8);
    assert!(offset_of!(TaskPod, updated_at_ms) == 16);
    assert!(offset_of!(TaskPod, priority) == 24);
    assert!(offset_of!(TaskPod, status) == 28);
    assert!(offset_of!(TaskPod, assignee_id) == 32);
    assert!(offset_of!(TaskPod, flags) == 36);
    assert!(offset_of!(TaskPod, title) == 40);
    assert!(offset_of!(TaskPod, body) == 48);
    assert!(offset_of!(TaskPod, tag) == 56);

    assert!(size_of::<TaskWorkspaceRoot>() == 16);
    assert!(align_of::<TaskWorkspaceRoot>() == 8);
    assert!(offset_of!(TaskWorkspaceRoot, tasks) == 0);
    assert!(offset_of!(TaskWorkspaceRoot, reserved) == 8);
};

/// Builds a 64-byte-aligned `NVERELOC` binary buffer from owned Studio Workspace tasks.
pub fn build_reloc_workspace(tasks: &[OwnedTask]) -> Result<RelocBuf, RelocError> {
    let str_bytes: usize = tasks
        .iter()
        .map(|t| t.title.len() + t.body.len() + t.tag.len())
        .sum();
    let cap = tasks.len() * size_of::<TaskPod>() + str_bytes + size_of::<TaskWorkspaceRoot>() + 64;
    let mut w = RelocWriter::with_capacity(RELOC_KIND_WORKSPACE, 1, 0, cap);

    let mut pods = Vec::with_capacity(tasks.len());
    for t in tasks {
        let title = w.write_str(&t.title)?;
        let body = w.write_str(&t.body)?;
        let tag = w.write_str(&t.tag)?;
        pods.push(TaskPod {
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

    let tasks_slice = w.write_slice(&pods)?;
    let root_ptr = w.write_val(&TaskWorkspaceRoot {
        tasks: tasks_slice,
        reserved: 0,
    })?;
    w.finish(root_ptr)
}

/// Builds and writes an `NVERELOC` workspace blob to `path`.
pub fn write_reloc_workspace(path: &Path, tasks: &[OwnedTask]) -> Result<usize, RelocError> {
    let bytes = build_reloc_workspace(tasks)?;
    write_file_bytes(path, &bytes)?;
    Ok(bytes.len())
}

/// Zero-copy borrowed view directly over an `NVERELOC` task workspace buffer.
#[derive(Clone, Copy)]
pub struct RelocWorkspaceView<'a> {
    view: RelocView<'a>,
    tasks: &'a [TaskPod],
    utf8_verified: bool,
}

impl<'a> RelocWorkspaceView<'a> {
    pub fn from_bytes(bytes: &'a [u8], mode: VerifyMode) -> Result<Self, RelocError> {
        let view = RelocView::from_bytes(bytes, mode)?;
        Self::from_view(view)
    }

    pub fn from_view(view: RelocView<'a>) -> Result<Self, RelocError> {
        view.require_content_kind(RELOC_KIND_WORKSPACE)?;
        let root = view.root::<TaskWorkspaceRoot>()?;
        let tasks = view.slice(root.tasks)?;
        let mut utf8_verified = false;
        if view.mode() == VerifyMode::Full {
            for pod in tasks {
                let _ = view.str_at(pod.title)?;
                let _ = view.str_at(pod.body)?;
                let _ = view.str_at(pod.tag)?;
            }
            utf8_verified = true;
        }
        Ok(Self {
            view,
            tasks,
            utf8_verified,
        })
    }

    /// Zero-check pointer cast directly onto an `NVERELOC` task workspace buffer.
    ///
    /// # Safety
    /// Caller guarantees `bytes` is a valid aligned `NVERELOC` buffer produced by
    /// [`build_reloc_workspace`].
    #[inline]
    pub unsafe fn from_bytes_unchecked(bytes: &'a [u8]) -> Self {
        // SAFETY: Caller guarantees `bytes` is a valid aligned `NVERELOC` workspace buffer produced by `build_reloc_workspace`.
        let (view, tasks) = unsafe {
            let view = RelocView::from_bytes_unchecked(bytes);
            let root = &*bytes
                .as_ptr()
                .add(view.header().root_offset as usize)
                .cast::<TaskWorkspaceRoot>();
            let tasks = std::slice::from_raw_parts(
                bytes
                    .as_ptr()
                    .add(root.tasks.offset as usize)
                    .cast::<TaskPod>(),
                root.tasks.len as usize,
            );
            (view, tasks)
        };
        Self {
            view,
            tasks,
            utf8_verified: true,
        }
    }

    #[inline]
    pub fn header(&self) -> &'a RelocHeader {
        self.view.header()
    }

    #[inline]
    pub fn tasks(&self) -> &'a [TaskPod] {
        self.tasks
    }

    #[inline]
    pub fn str_at(&self, s: RelocStr) -> Result<&'a str, RelocError> {
        if self.utf8_verified {
            // SAFETY: Validated during `VerifyMode::Full` or upheld by `from_bytes_unchecked`.
            unsafe { self.view.str_at_unchecked(s) }
        } else {
            self.view.str_at(s)
        }
    }

    pub fn random_read_checksum(&self, indices: &[usize]) -> Result<u64, RelocError> {
        let mut acc = 0u64;
        for &idx in indices {
            let pod = &self.tasks[idx];
            let title = self.str_at(pod.title)?;
            let tag = self.str_at(pod.tag)?;
            acc = fold_task_fields(
                acc,
                pod.id,
                pod.updated_at_ms,
                pod.priority,
                pod.status,
                title,
                tag,
            );
        }
        Ok(acc)
    }
}

/// Owned `mmap`-backed `NVERELOC` task workspace handle.
pub struct MmapRelocWorkspace {
    mmap: MmapReloc,
    mode: VerifyMode,
}

impl MmapRelocWorkspace {
    pub fn open(path: &Path, mode: VerifyMode) -> Result<Self, RelocError> {
        let effective = match mode {
            VerifyMode::Unchecked => VerifyMode::HeaderOnly,
            other => other,
        };
        let mmap = MmapReloc::open(path, effective)?;
        let _ = RelocWorkspaceView::from_view(mmap.view())?;
        Ok(Self {
            mmap,
            mode: effective,
        })
    }

    /// Opens and maps an `NVERELOC` file with zero-check pointer casting (`VerifyMode::Unchecked`).
    ///
    /// # Safety
    /// File contents must be a valid `NVERELOC` workspace blob produced by [`build_reloc_workspace`].
    pub unsafe fn open_unchecked(path: &Path) -> Result<Self, RelocError> {
        // SAFETY: Upheld by caller.
        let mmap = unsafe { MmapReloc::open_unchecked(path)? };
        Ok(Self {
            mmap,
            mode: VerifyMode::Unchecked,
        })
    }

    #[inline]
    pub fn view(&self) -> RelocWorkspaceView<'_> {
        match self.mode {
            VerifyMode::Unchecked | VerifyMode::Full => {
                // SAFETY: Validated on open or upheld by `open_unchecked`.
                unsafe { RelocWorkspaceView::from_bytes_unchecked(self.mmap.as_bytes()) }
            }
            VerifyMode::HeaderOnly => {
                // SAFETY: Header & root slice bounds were validated in `open`.
                let mut v =
                    unsafe { RelocWorkspaceView::from_bytes_unchecked(self.mmap.as_bytes()) };
                v.utf8_verified = false;
                v
            }
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.mmap.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.mmap.is_empty()
    }
}
