//! Composable `Procedure` contracts, type-level `EffectKind` markers (`Query` / `Action`),
//! compile-time wire-name validation, and optional zero-proc-macro `procedures!` sugar.

use nve_hash::xxh64;

use crate::status::DomainStatus;
use crate::tag::TagSink;
use crate::transport::{MaybeSend, MaybeSync};

/// Domain-separated XXH64 seed for procedure wire names.
pub const PROC_ID_SEED: u64 = 0x4E56_455F_5052_4F43;

/// Stable 64-bit hash of a procedure's explicit wire name (`"issues.v1.Issues/List"`).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcId(u64);

impl ProcId {
    /// Computes the compile-time procedure identifier from an explicit wire name.
    #[inline]
    pub const fn of(name: &str) -> Self {
        Self(xxh64(name.as_bytes(), PROC_ID_SEED))
    }

    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Validates that a procedure wire name is non-empty, has no leading/trailing/empty/dot
/// (`.` / `..`) path segments, and uses only URI-safe characters (`[A-Za-z0-9._/-]`).
pub const fn is_valid_procedure_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes[0] == b'/' || bytes[bytes.len() - 1] == b'/' {
        return false;
    }
    let mut i = 0usize;
    let mut seg_start = 0usize;
    let mut prev_slash = false;
    while i < bytes.len() {
        let b = bytes[i];
        let ok = b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/');
        if !ok {
            return false;
        }
        if b == b'/' {
            if prev_slash || is_dot_segment(bytes, seg_start, i) {
                return false;
            }
            prev_slash = true;
            seg_start = i + 1;
        } else {
            prev_slash = false;
        }
        i += 1;
    }
    !is_dot_segment(bytes, seg_start, bytes.len())
}

const fn is_dot_segment(bytes: &[u8], start: usize, end: usize) -> bool {
    let len = end - start;
    (len == 1 && bytes[start] == b'.')
        || (len == 2 && bytes[start] == b'.' && bytes[start + 1] == b'.')
}

/// Compile-time assertion enforcing [`is_valid_procedure_name`].
pub const fn assert_valid_procedure_name(name: &str) {
    assert!(
        is_valid_procedure_name(name),
        "Procedure::NAME must be non-empty, without leading/trailing/double slashes, and contain only [A-Za-z0-9._/-]"
    );
}

/// Runtime effect classification matching [`EffectKind`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Effect {
    Query = 1,
    Action = 2,
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Query {}
    impl Sealed for super::Action {}
}

/// Type-level marker distinguishing side-effect-free queries from state-mutating actions so
/// query-only APIs (`preload::<P>()`, `use_query::<P>()`) reject actions during `cargo check`.
pub trait EffectKind: sealed::Sealed + Copy + 'static {
    const EFFECT: Effect;
    const IS_QUERY: bool;
}

/// Marker type for read-only, cacheable/preloadable procedures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Query;

impl EffectKind for Query {
    const EFFECT: Effect = Effect::Query;
    const IS_QUERY: bool = true;
}

/// Marker type for state-mutating procedures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Action;

impl EffectKind for Action {
    const EFFECT: Effect = Effect::Action;
    const IS_QUERY: bool = false;
}

/// Composable Layer-0 RPC procedure contract.
///
/// Implemented manually, via [`crate::procedures!`], or emitted by any future user or framework macro
/// (`#[nve::service]`, `#[nve::query]`, etc.).
pub trait Procedure: 'static {
    /// Explicit stable wire path segment (e.g. `"issues.v1.Issues/List"`). Never derive from
    /// `module_path!()` so refactoring modules cannot break installed mobile binaries.
    const NAME: &'static str;

    /// Deterministic hash of [`Self::NAME`] used for zero-allocation sorted-slice dispatch.
    /// Do not override; `call_with_codec` and `RpcRegistryBuilder` verify `ID == ProcId::of(NAME)`
    /// at compile time.
    const ID: ProcId = ProcId::of(Self::NAME);

    /// Compile-time [`Query`] or [`Action`] marker.
    type Effect: EffectKind;

    /// Request argument payload.
    type Args: MaybeSend + MaybeSync + 'static;

    /// Successful response payload.
    type Output: MaybeSend + MaybeSync + 'static;

    /// Typed domain error payload (returned with HTTP 4xx + `Nve-Error-Kind: domain`).
    type Error: DomainStatus + MaybeSend + MaybeSync + 'static;

    /// Optional stack-allocated cache tags this query depends on.
    #[inline]
    fn tags(_args: &Self::Args, _sink: &mut TagSink) {}

    /// Optional stack-allocated cache tags statically invalidated when an action succeeds.
    #[inline]
    fn invalidates(_args: &Self::Args, _sink: &mut TagSink) {}
}

/// Zero-proc-macro declarative helper for defining [`Procedure`] zero-sized marker types with
/// optional `tags` / `invalidates` clauses and compile-time wire-name validation.
///
/// # Example
/// ```rust
/// use nve_rpc::{procedures, DomainStatus, Procedure, Tag64};
///
/// #[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
/// #[cbor(map)]
/// pub struct EchoReq { #[n(0)] pub text: String }
///
/// #[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
/// #[cbor(map)]
/// pub struct EchoRes { #[n(0)] pub text: String }
///
/// #[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
/// pub enum EchoErr { #[n(0)] TooLong }
/// impl DomainStatus for EchoErr {}
///
/// procedures! {
///     pub query Echo = "demo.v1.Echo/Say" (EchoReq) -> EchoRes, EchoErr,
///         tags |_req, sink| { sink.push(Tag64::of("demo.echo")); };
/// }
///
/// assert_eq!(Echo::NAME, "demo.v1.Echo/Say");
/// ```
#[macro_export]
macro_rules! procedures {
    () => {};
    (
        $(#[$meta:meta])*
        $vis:vis query $ident:ident = $name:literal ($args:ty) -> $output:ty, $error:ty,
            tags |$tag_arg:pat_param, $tag_sink:ident| $tag_body:block;
        $($rest:tt)*
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        $vis struct $ident;

        impl $crate::Procedure for $ident {
            const NAME: &'static str = {
                $crate::assert_valid_procedure_name($name);
                $name
            };
            type Effect = $crate::Query;
            type Args = $args;
            type Output = $output;
            type Error = $error;

            #[inline]
            fn tags($tag_arg: &Self::Args, $tag_sink: &mut $crate::TagSink) $tag_body
        }

        $crate::procedures! { $($rest)* }
    };
    (
        $(#[$meta:meta])*
        $vis:vis query $ident:ident = $name:literal ($args:ty) -> $output:ty, $error:ty;
        $($rest:tt)*
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        $vis struct $ident;

        impl $crate::Procedure for $ident {
            const NAME: &'static str = {
                $crate::assert_valid_procedure_name($name);
                $name
            };
            type Effect = $crate::Query;
            type Args = $args;
            type Output = $output;
            type Error = $error;
        }

        $crate::procedures! { $($rest)* }
    };
    (
        $(#[$meta:meta])*
        $vis:vis action $ident:ident = $name:literal ($args:ty) -> $output:ty, $error:ty,
            invalidates |$inv_arg:pat_param, $inv_sink:ident| $inv_body:block;
        $($rest:tt)*
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        $vis struct $ident;

        impl $crate::Procedure for $ident {
            const NAME: &'static str = {
                $crate::assert_valid_procedure_name($name);
                $name
            };
            type Effect = $crate::Action;
            type Args = $args;
            type Output = $output;
            type Error = $error;

            #[inline]
            fn invalidates($inv_arg: &Self::Args, $inv_sink: &mut $crate::TagSink) $inv_body
        }

        $crate::procedures! { $($rest)* }
    };
    (
        $(#[$meta:meta])*
        $vis:vis action $ident:ident = $name:literal ($args:ty) -> $output:ty, $error:ty;
        $($rest:tt)*
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        $vis struct $ident;

        impl $crate::Procedure for $ident {
            const NAME: &'static str = {
                $crate::assert_valid_procedure_name($name);
                $name
            };
            type Effect = $crate::Action;
            type Args = $args;
            type Output = $output;
            type Error = $error;
        }

        $crate::procedures! { $($rest)* }
    };
}
