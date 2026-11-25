//! First-class `<NativeView>` Dioxus component, registry context, and binary prop/handle
//! forwarding into `NveMutationWriter`.

use std::sync::Arc;

use dioxus_core::{
    provide_context, try_consume_context, use_hook, Attribute, AttributeValue, Element, VNode,
};
use dioxus_core_macro::{component, rsx, Props};
use dioxus_signals::{ReadableExt, Signal, WritableExt};

pub use nve_interop::native_view::{
    BlurMaterial, BlurSurface, BlurSurfaceProps, DecodeNativeCommand, DecodeNativeProps,
    DecodedWebElementProps, EncodeNativeCommand, EncodeNativeProps, NativeCommand,
    NativeCommandDecoder, NativeCommandEncoder, NativeProp, NativePropDecoder, NativePropEncoder,
    NativePropValue, NativeProps, NativeValueRef, NativeViewCodecError, NativeViewHandle,
    NativeViewHandleBinder, NativeViewRegistry, NativeViewSpec, NoCommand, QueuedNativeCommand,
    TypedNativeViewCommand, TypedNativeViewProps, VideoPlayer, VideoPlayerCommand,
    VideoPlayerProps, WebElement, WebElementProps, WebView, WebViewCommand, WebViewProps,
    NATIVE_CMD_MAGIC, NATIVE_PROPS_MAGIC, NATIVE_VIEW_KIND_PROP_KEY, NATIVE_VIEW_PROPS_PROP_KEY,
};

/// Internal template tag recognized by [`crate::NveMutationWriter`] to emit a deferred
/// `ComponentKind::Custom` node whose `custom_name` is resolved from [`NATIVE_VIEW_KIND_ATTR`].
pub(crate) const NATIVE_VIEW_TAG: &str = "nve.native_view";
/// Internal attribute carrying the resolved native kind string (e.g. `"nve.blur_surface"` or
/// `"web.video"`) so `NveMutationWriter` emits `CreateNode { kind: Custom, custom_name }` directly.
pub(crate) const NATIVE_VIEW_KIND_ATTR: &str = "__nve_native_kind";
/// Internal attribute carrying a type-erased [`NativeViewHandleBinder`] for mount/unmount binding.
pub(crate) const NATIVE_VIEW_HANDLE_ATTR: &str = "__nve_native_handle";

/// Zero-serialization typed prop payload forwarded via `AttributeValue::Any` straight into
/// `NveMutationWriter` without `NVNP` binary encoding on in-process Rust hosts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NativeTypedPropsAttr(pub TypedNativeViewProps);

/// Type-erased handle binder forwarded via `AttributeValue::Any` so `NveMutationWriter` binds
/// `(surface_id, node_id)` automatically on mount and unbinds on unmount.
#[derive(Debug, Clone)]
pub(crate) struct NativeHandleAttr(pub Option<NativeViewHandleBinder>);

impl PartialEq for NativeHandleAttr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => a.ptr_eq(b),
            _ => false,
        }
    }
}

/// Private element namespace used exclusively by [`NativeView`] so `dioxus_elements.rs` stays
/// untouched and clean.
mod dioxus_elements {
    #![allow(non_upper_case_globals, non_snake_case, non_camel_case_types, unused_imports)]

    pub use crate::dioxus_elements::GlobalAttributes;
    use crate::dioxus_elements::AttributeDescription;

    pub mod elements {
        pub mod nativeview {
            pub const TAG_NAME: &str = crate::native_view::NATIVE_VIEW_TAG;
            pub const NAME_SPACE: Option<&str> = None;
        }

        pub mod completions {
            pub mod CompleteWithBraces {}
        }
    }

    pub use elements::*;
}

#[derive(Clone, Copy)]
struct NativeViewRegistryContext {
    registry: Signal<NativeViewRegistry>,
}

/// Provides a [`NativeViewRegistry`] to the current Dioxus subtree.
pub fn use_provide_native_view_registry(
    init: impl FnOnce() -> NativeViewRegistry,
) -> Signal<NativeViewRegistry> {
    let ctx = use_hook(|| {
        provide_context(NativeViewRegistryContext {
            registry: Signal::new(init()),
        })
    });
    ctx.registry
}

/// Reads the active [`NativeViewRegistry`] from Dioxus context (or an empty registry if none is
/// provided, causing `<NativeView>` components to render their graceful `fallback` slot).
pub fn use_native_view_registry() -> NativeViewRegistry {
    match try_consume_context::<NativeViewRegistryContext>() {
        Some(ctx) => ctx.registry.read().clone(),
        None => NativeViewRegistry::new(),
    }
}

/// Hook creating a persistent typed [`NativeViewHandle`] for imperative binary commands.
pub fn use_native_view_handle<P: NativeViewSpec>() -> NativeViewHandle<P> {
    use_hook(NativeViewHandle::<P>::new)
}

/// Context provider component exposing a host [`NativeViewRegistry`] to descendant `<NativeView>`s.
#[component]
pub fn NativeViewProvider(registry: NativeViewRegistry, children: Element) -> Element {
    let mut sig = use_provide_native_view_registry(|| registry.clone());
    if *sig.peek() != registry {
        sig.set(registry);
    }
    children
}

/// Properties accepted by [`NativeView`].
#[derive(Props)]
pub struct NativeViewProps<P: NativeViewSpec> {
    pub props: P::Props,
    #[props(into, default)]
    pub style: Option<String>,
    #[props(into, default)]
    pub test_id: Option<String>,
    #[props(default)]
    pub handle: Option<NativeViewHandle<P>>,
    #[props(into, default)]
    pub fallback: Option<Element>,
}

impl<P: NativeViewSpec> Clone for NativeViewProps<P> {
    fn clone(&self) -> Self {
        Self {
            props: self.props.clone(),
            style: self.style.clone(),
            test_id: self.test_id.clone(),
            handle: self.handle.clone(),
            fallback: self.fallback.clone(),
        }
    }
}

impl<P: NativeViewSpec> PartialEq for NativeViewProps<P> {
    fn eq(&self, other: &Self) -> bool {
        self.props == other.props
            && self.style == other.style
            && self.test_id == other.test_id
            && self.handle == other.handle
            && self.fallback == other.fallback
    }
}

/// First-class native platform view escape hatch (`<NativeView<P>>`).
///
/// - Checks the active [`NativeViewRegistry`] for `P::resolved_kind(&props)` (supporting exact
///   kinds like `"nve.blur_surface"` and prefix wildcards like `"web.*"`).
/// - When supported: emits a standard `nve-abi` `ComponentKind::Custom` node with binary `NVNP`
///   props (`RawPropValue::Bytes`, binary FFI) and binds `handle` on mount / unbinds on unmount.
/// - When unsupported: gracefully renders `fallback` (or an empty node) and leaves `handle` unbound.
#[allow(non_snake_case)]
pub fn NativeView<P: NativeViewSpec>(nv_props: NativeViewProps<P>) -> Element {
    let NativeViewProps {
        props,
        style,
        test_id,
        handle,
        fallback,
    } = nv_props;
    let registry = use_native_view_registry();
    let kind = P::resolved_kind(&props);

    if !registry.supports(kind.as_ref()) {
        if let Some(ref h) = handle {
            h.unbind();
        }
        return fallback.unwrap_or_else(VNode::empty);
    }

    let native_props = match props.to_native_props() {
        Ok(p) => p,
        Err(_) => {
            if let Some(ref h) = handle {
                h.unbind();
            }
            return fallback.unwrap_or_else(VNode::empty);
        }
    };
    let kind_owned = kind.into_owned();
    let typed_props = TypedNativeViewProps::with_typed(
        kind_owned.as_str(),
        native_props,
        Arc::new(props),
    );

    let mut attrs = Vec::with_capacity(5);
    // Must be first so `NveMutationWriter` emits `CreateNode { Custom, custom_name: Some(kind) }`
    // before any prop/layout/style op for this node.
    attrs.push(Attribute::new(
        NATIVE_VIEW_KIND_ATTR,
        AttributeValue::Text(kind_owned),
        None,
        false,
    ));
    attrs.push(Attribute::new(
        NATIVE_VIEW_PROPS_PROP_KEY,
        AttributeValue::any_value(NativeTypedPropsAttr(typed_props)),
        None,
        false,
    ));
    if let Some(ref h) = handle {
        attrs.push(Attribute::new(
            NATIVE_VIEW_HANDLE_ATTR,
            AttributeValue::any_value(NativeHandleAttr(Some(h.binder()))),
            None,
            false,
        ));
    }
    if let Some(ref tid) = test_id {
        attrs.push(Attribute::new(
            "testID",
            AttributeValue::Text(tid.clone()),
            None,
            false,
        ));
    }
    if let Some(ref st) = style {
        attrs.push(Attribute::new(
            "style",
            AttributeValue::Text(st.clone()),
            None,
            false,
        ));
    }

    rsx! {
        nativeview {
            ..attrs,
        }
    }
}
