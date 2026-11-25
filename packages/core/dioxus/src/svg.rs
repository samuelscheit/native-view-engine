//! Zero-serialization Dioxus `rsx!` attribute wrapper ([`SvgPictureAttr`]) carrying an
//! immutable, content-hashed [`SvgPicture`] directly through `AttributeValue::Any` into
//! `NveMutationWriter` without stringifying or re-parsing SVG markup on native targets.

use std::ops::Deref;
use std::rc::Rc;

use dioxus_core::{AttributeValue, IntoAttributeValue};
use nve_components::svg::SvgPicture;

/// Attribute wire name recognized by [`crate::NveMutationWriter`] for direct typed [`SvgPicture`].
pub const NVE_SVG_PICTURE_ATTR: &str = "svg_picture";

/// Ergonomic `IntoAttributeValue` wrapper carrying a zero-serialization [`SvgPicture`] directly
/// into `<svg picture: SvgPictureAttr::from(pic), ...>` or any element's `svg_picture` attribute.
#[derive(Debug, Clone, PartialEq)]
pub struct SvgPictureAttr(pub SvgPicture);

impl SvgPictureAttr {
    #[inline]
    pub fn new(picture: SvgPicture) -> Self {
        Self(picture)
    }

    #[inline]
    pub fn into_inner(self) -> SvgPicture {
        self.0
    }
}

impl Deref for SvgPictureAttr {
    type Target = SvgPicture;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<SvgPicture> for SvgPictureAttr {
    #[inline]
    fn from(picture: SvgPicture) -> Self {
        Self(picture)
    }
}

impl IntoAttributeValue for SvgPictureAttr {
    #[inline]
    fn into_value(self) -> AttributeValue {
        AttributeValue::Any(Rc::new(self.0))
    }
}
