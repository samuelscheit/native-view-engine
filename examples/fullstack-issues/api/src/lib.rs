//! Shared pure-Rust RPC contracts for `examples/fullstack-issues`.
//!
//! Compiles with zero server or UI dependencies on native, `wasm32-unknown-unknown`, and
//! `wasm32-wasip2`. Both structs (integer-keyed CBOR maps) and enums (open `Unknown(u32)`
//! catch-alls) tolerate mobile App Store version skew without turning unknown server variants
//! into decode failures.

use minicbor::decode::Error as DecodeError;
use minicbor::encode::{Error as EncodeError, Write as EncodeWrite};
use minicbor::{Decode, Decoder, Encode, Encoder};
use nve_rpc::{
    procedures, CacheTag, DomainStatus, FieldErrors, FormDecode, FormPairs, RpcStatus, Tag64,
};

/// Strongly typed issue identifier encoded transparently as a CBOR `u64`.
#[repr(transparent)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, minicbor::Encode, minicbor::Decode,
)]
#[cbor(transparent)]
pub struct IssueId(#[n(0)] pub u64);

/// Strongly typed comment identifier encoded transparently as a CBOR `u64`.
#[repr(transparent)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, minicbor::Encode, minicbor::Decode,
)]
#[cbor(transparent)]
pub struct CommentId(#[n(0)] pub u64);

/// Open issue workflow state tolerating future server variants (`Unknown(u32)`) on older mobile
/// clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueState {
    Open,
    InProgress,
    Closed,
    Unknown(u32),
}

impl IssueState {
    #[inline]
    pub const fn to_wire(self) -> u32 {
        match self {
            Self::Open => 0,
            Self::InProgress => 1,
            Self::Closed => 2,
            Self::Unknown(code) => code,
        }
    }

    #[inline]
    pub const fn from_wire(code: u32) -> Self {
        match code {
            0 => Self::Open,
            1 => Self::InProgress,
            2 => Self::Closed,
            other => Self::Unknown(other),
        }
    }
}

impl<C> Encode<C> for IssueState {
    fn encode<W: EncodeWrite>(
        &self,
        e: &mut Encoder<W>,
        _ctx: &mut C,
    ) -> Result<(), EncodeError<W::Error>> {
        if let Self::Unknown(code) = *self
            && code <= 2
        {
            return Err(EncodeError::message(
                "IssueState::Unknown must carry an unrecognized code (>= 3)",
            ));
        }
        e.u32(self.to_wire())?;
        Ok(())
    }
}

impl<'b, C> Decode<'b, C> for IssueState {
    fn decode(d: &mut Decoder<'b>, _ctx: &mut C) -> Result<Self, DecodeError> {
        Ok(Self::from_wire(d.u32()?))
    }
}

/// Issue record encoded as an integer-keyed CBOR map for forward/backward mobile compatibility.
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct Issue {
    #[n(0)]
    pub id: IssueId,
    #[n(1)]
    pub title: String,
    #[n(2)]
    pub state: IssueState,
    #[n(3)]
    pub priority: Option<u8>,
    #[n(4)]
    pub comment_count: u32,
}

/// Issue comment record.
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct Comment {
    #[n(0)]
    pub id: CommentId,
    #[n(1)]
    pub issue_id: IssueId,
    #[n(2)]
    pub author: String,
    #[n(3)]
    pub body: String,
}

/// Filter arguments for listing issues.
#[derive(Debug, Clone, Default, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct IssueFilter {
    #[n(0)]
    pub state: Option<IssueState>,
    #[n(1)]
    pub query: Option<String>,
}

/// Input for renaming an existing issue (supports both CBOR RPC and 0-JS HTML `<form>`).
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct RenameIssueInput {
    #[n(0)]
    pub id: IssueId,
    #[n(1)]
    pub title: String,
}

impl RenameIssueInput {
    /// Single source of truth for field validation shared by RPC handlers and [`FormDecode`].
    pub fn validate_into(&self, errs: &mut FieldErrors) {
        let trimmed = self.title.trim();
        if trimmed.is_empty() {
            errs.push("title", "title must not be empty");
        } else if trimmed.len() > 200 {
            errs.push("title", "title must be <= 200 bytes");
        }
    }

    pub fn validate(&self) -> Result<(), FieldErrors> {
        let mut errs = FieldErrors::new();
        self.validate_into(&mut errs);
        errs.into_result(())
    }
}

impl FormDecode for RenameIssueInput {
    fn from_form(form: &FormPairs) -> Result<Self, FieldErrors> {
        let mut errs = FieldErrors::new();
        let id = form
            .get("id")
            .and_then(|raw| raw.parse::<u64>().ok())
            .map(IssueId)
            .unwrap_or_else(|| {
                errs.push("id", "valid numeric issue id required");
                IssueId(0)
            });
        let input = Self {
            id,
            title: form.get("title").unwrap_or("").trim().to_owned(),
        };
        input.validate_into(&mut errs);
        errs.into_result(input)
    }
}

/// Input for appending a comment to an issue (supports both CBOR RPC and 0-JS HTML `<form>`).
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct AddCommentInput {
    #[n(0)]
    pub issue_id: IssueId,
    #[n(1)]
    pub body: String,
}

impl AddCommentInput {
    /// Single source of truth for field validation shared by RPC handlers and [`FormDecode`].
    pub fn validate_into(&self, errs: &mut FieldErrors) {
        let trimmed = self.body.trim();
        if trimmed.is_empty() {
            errs.push("body", "comment body must not be empty");
        } else if trimmed.len() > 2000 {
            errs.push("body", "comment body must be <= 2000 bytes");
        }
    }

    pub fn validate(&self) -> Result<(), FieldErrors> {
        let mut errs = FieldErrors::new();
        self.validate_into(&mut errs);
        errs.into_result(())
    }
}

impl FormDecode for AddCommentInput {
    fn from_form(form: &FormPairs) -> Result<Self, FieldErrors> {
        let mut errs = FieldErrors::new();
        let issue_id = form
            .get("issue_id")
            .and_then(|raw| raw.parse::<u64>().ok())
            .map(IssueId)
            .unwrap_or_else(|| {
                errs.push("issue_id", "valid numeric issue_id required");
                IssueId(0)
            });
        let input = Self {
            issue_id,
            body: form.get("body").unwrap_or("").trim().to_owned(),
        };
        input.validate_into(&mut errs);
        errs.into_result(input)
    }
}

/// Open typed domain error union tolerating future server error variants (`Unknown(u32)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    NotFound(IssueId),
    Validation(FieldErrors),
    Unknown(u32),
}

impl<C> Encode<C> for ApiError {
    fn encode<W: EncodeWrite>(
        &self,
        e: &mut Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), EncodeError<W::Error>> {
        match self {
            Self::NotFound(id) => {
                e.array(2)?.u32(0)?;
                id.encode(e, ctx)?;
            }
            Self::Validation(errs) => {
                e.array(2)?.u32(1)?;
                errs.encode(e, ctx)?;
            }
            Self::Unknown(code) => {
                if *code <= 1 {
                    return Err(EncodeError::message(
                        "ApiError::Unknown must carry an unrecognized variant code (>= 2)",
                    ));
                }
                e.array(2)?.u32(*code)?.null()?;
            }
        }
        Ok(())
    }
}

impl<'b, C> Decode<'b, C> for ApiError {
    fn decode(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Self, DecodeError> {
        let len = d
            .array()?
            .ok_or_else(|| DecodeError::message("ApiError expects definite CBOR array"))?;
        if len == 0 {
            return Err(DecodeError::message("ApiError tag missing"));
        }
        let tag = d.u32()?;
        let out = match tag {
            0 => {
                if len < 2 {
                    return Err(DecodeError::message("ApiError::NotFound missing IssueId"));
                }
                let id = IssueId::decode(d, ctx)?;
                for _ in 2..len {
                    d.skip()?;
                }
                Self::NotFound(id)
            }
            1 => {
                if len < 2 {
                    return Err(DecodeError::message("ApiError::Validation missing FieldErrors"));
                }
                let errs = FieldErrors::decode(d, ctx)?;
                for _ in 2..len {
                    d.skip()?;
                }
                Self::Validation(errs)
            }
            other => {
                for _ in 1..len {
                    d.skip()?;
                }
                Self::Unknown(other)
            }
        };
        Ok(out)
    }
}

impl DomainStatus for ApiError {
    fn rpc_status(&self) -> RpcStatus {
        match self {
            Self::NotFound(_) => RpcStatus::NotFound,
            Self::Validation(_) | Self::Unknown(_) => RpcStatus::Unprocessable,
        }
    }

    fn field_errors(&self) -> Option<&FieldErrors> {
        match self {
            Self::Validation(errs) => Some(errs),
            Self::NotFound(_) | Self::Unknown(_) => None,
        }
    }
}

/// Strongly typed zero-allocation cache invalidation tags for `examples/fullstack-issues`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IssueTag {
    List,
    Issue(IssueId),
    Comments(IssueId),
}

impl CacheTag for IssueTag {
    #[inline]
    fn tag64(&self) -> Tag64 {
        match *self {
            Self::List => Tag64::of("issues.v1.list"),
            Self::Issue(id) => Tag64::keyed("issues.v1.issue", id.0),
            Self::Comments(id) => Tag64::keyed("issues.v1.comments", id.0),
        }
    }
}

procedures! {
    pub query ListIssues = "issues.v1.Issues/List" (IssueFilter) -> Vec<Issue>, ApiError,
        tags |_filter, sink| {
            sink.push(IssueTag::List);
        };

    pub query GetIssue = "issues.v1.Issues/Get" (IssueId) -> Issue, ApiError,
        tags |id, sink| {
            sink.push(IssueTag::Issue(*id));
        };

    pub query ListComments = "issues.v1.Comments/List" (IssueId) -> Vec<Comment>, ApiError,
        tags |id, sink| {
            sink.push(IssueTag::Comments(*id));
        };

    pub action RenameIssue = "issues.v1.Issues/Rename" (RenameIssueInput) -> Issue, ApiError,
        invalidates |input, sink| {
            sink.push(IssueTag::List);
            sink.push(IssueTag::Issue(input.id));
        };

    pub action AddComment = "issues.v1.Comments/Add" (AddCommentInput) -> Comment, ApiError,
        invalidates |input, sink| {
            sink.push(IssueTag::List);
            sink.push(IssueTag::Issue(input.issue_id));
            sink.push(IssueTag::Comments(input.issue_id));
        };
}
