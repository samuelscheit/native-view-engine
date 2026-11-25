use crate::NodeId;

use super::types::{ComponentKind, LayoutProps, RawPropValue, RawPropValueRef, StyleProps};

/// Zero-allocation iterator over little-endian `NodeId` values in a `SetChildren` payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildIdIter<'a> {
    pub(super) bytes: &'a [u8],
}

impl<'a> ChildIdIter<'a> {
    #[inline]
    pub fn len(&self) -> usize {
        self.bytes.len() / 4
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    #[inline]
    pub fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<NodeId> {
        let offset = index.checked_mul(4)?;
        let chunk = self.bytes.get(offset..offset + 4)?;
        Some(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
    }

    #[inline]
    pub fn iter(&self) -> Self {
        *self
    }

    #[inline]
    pub fn to_vec(&self) -> Vec<NodeId> {
        let mut out = Vec::with_capacity(self.len());
        self.copy_into(&mut out);
        out
    }

    #[inline]
    pub fn copy_into(&self, dst: &mut Vec<NodeId>) {
        dst.clear();
        dst.reserve(self.len());
        for chunk in self.bytes.chunks_exact(4) {
            dst.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
    }
}

impl<'a> Iterator for ChildIdIter<'a> {
    type Item = NodeId;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.bytes.len() < 4 {
            return None;
        }
        let (head, rest) = self.bytes.split_at(4);
        self.bytes = rest;
        Some(u32::from_le_bytes([head[0], head[1], head[2], head[3]]))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.bytes.len() / 4;
        (n, Some(n))
    }
}

impl<'a> DoubleEndedIterator for ChildIdIter<'a> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        let len = self.bytes.len();
        if len < 4 {
            return None;
        }
        let (rest, tail) = self.bytes.split_at(len - 4);
        self.bytes = rest;
        Some(u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]))
    }
}

impl<'a> ExactSizeIterator for ChildIdIter<'a> {}
impl<'a> std::iter::FusedIterator for ChildIdIter<'a> {}

/// Zero-copy borrowed command decoded directly from a `CommandBuffer` byte slice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CommandRef<'a> {
    CreateNode {
        node_id: NodeId,
        kind: ComponentKind,
        custom_name: Option<&'a str>,
    },
    RemoveNode {
        node_id: NodeId,
    },
    SetChildren {
        parent_id: NodeId,
        children: ChildIdIter<'a>,
    },
    SetText {
        node_id: NodeId,
        text: &'a str,
    },
    SetLayout {
        node_id: NodeId,
        layout: LayoutProps,
    },
    SetStyle {
        node_id: NodeId,
        style: StyleProps,
    },
    SetRawProp {
        node_id: NodeId,
        key: &'a str,
        value: RawPropValueRef<'a>,
    },
    SetEventMask {
        node_id: NodeId,
        mask: u64,
    },
    DispatchCommand {
        node_id: NodeId,
        command_name: &'a str,
        payload: &'a [u8],
    },
    Commit {
        sync_mount: bool,
    },
}

impl<'a> CommandRef<'a> {
    #[inline]
    pub fn to_owned(self) -> Command {
        match self {
            Self::CreateNode {
                node_id,
                kind,
                custom_name,
            } => Command::CreateNode {
                node_id,
                kind,
                custom_name: custom_name.map(ToOwned::to_owned),
            },
            Self::RemoveNode { node_id } => Command::RemoveNode { node_id },
            Self::SetChildren {
                parent_id,
                children,
            } => Command::SetChildren {
                parent_id,
                children: children.to_vec(),
            },
            Self::SetText { node_id, text } => Command::SetText {
                node_id,
                text: text.to_owned(),
            },
            Self::SetLayout { node_id, layout } => Command::SetLayout { node_id, layout },
            Self::SetStyle { node_id, style } => Command::SetStyle { node_id, style },
            Self::SetRawProp {
                node_id,
                key,
                value,
            } => Command::SetRawProp {
                node_id,
                key: key.to_owned(),
                value: value.to_owned(),
            },
            Self::SetEventMask { node_id, mask } => Command::SetEventMask { node_id, mask },
            Self::DispatchCommand {
                node_id,
                command_name,
                payload,
            } => Command::DispatchCommand {
                node_id,
                command_name: command_name.to_owned(),
                payload: payload.to_vec(),
            },
            Self::Commit { sync_mount } => Command::Commit { sync_mount },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    CreateNode {
        node_id: NodeId,
        kind: ComponentKind,
        custom_name: Option<String>,
    },
    RemoveNode {
        node_id: NodeId,
    },
    SetChildren {
        parent_id: NodeId,
        children: Vec<NodeId>,
    },
    SetText {
        node_id: NodeId,
        text: String,
    },
    SetLayout {
        node_id: NodeId,
        layout: LayoutProps,
    },
    SetStyle {
        node_id: NodeId,
        style: StyleProps,
    },
    SetRawProp {
        node_id: NodeId,
        key: String,
        value: RawPropValue,
    },
    SetEventMask {
        node_id: NodeId,
        mask: u64,
    },
    DispatchCommand {
        node_id: NodeId,
        command_name: String,
        payload: Vec<u8>,
    },
    Commit {
        sync_mount: bool,
    },
}
