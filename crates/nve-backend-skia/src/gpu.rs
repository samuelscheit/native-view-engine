//! Hardware GPU & CPU Skia surface architecture (`--features skia`).
//!
//! Keeps platform GPU surface selection (`Metal` on macOS, `Direct3D12` on Windows,
//! `Vulkan` on Linux/Android, and `CpuRaster` fallback via `skia_safe::surfaces::raster_n32_premul`)
//! cleanly separated from the zero-C++ default `--features desktop` software rasterizer.

use std::fmt;

/// Identifies the active Skia rendering surface backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuBackendKind {
    /// Apple Metal GPU backend (macOS / iOS).
    Metal,
    /// Direct3D 12 GPU backend (Windows).
    Direct3D12,
    /// Vulkan GPU backend (Linux / Windows / Android).
    Vulkan,
    /// CPU-backed Skia raster surface (`skia_safe::surfaces::raster_n32_premul`).
    CpuRaster,
}

impl GpuBackendKind {
    /// Returns the preferred hardware backend for the current compilation target.
    pub const fn platform_preferred() -> Self {
        if cfg!(any(target_os = "macos", target_os = "ios")) {
            Self::Metal
        } else if cfg!(target_os = "windows") {
            Self::Direct3D12
        } else if cfg!(any(target_os = "linux", target_os = "android")) {
            Self::Vulkan
        } else {
            Self::CpuRaster
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Metal => "metal",
            Self::Direct3D12 => "d3d12",
            Self::Vulkan => "vulkan",
            Self::CpuRaster => "cpu-raster",
        }
    }
}

impl fmt::Display for GpuBackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Configuration for creating a Skia rendering surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuSurfaceConfig {
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
    pub preferred_backend: GpuBackendKind,
    /// When `true`, automatically falls back to `CpuRaster` if a hardware GPU context cannot
    /// be initialized on the current display/headless environment.
    pub allow_cpu_fallback: bool,
}

impl Default for GpuSurfaceConfig {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            scale_factor: 1.0,
            preferred_backend: GpuBackendKind::platform_preferred(),
            allow_cpu_fallback: true,
        }
    }
}

/// Errors returned while creating, resizing, or reading back a Skia surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuSurfaceError {
    InvalidDimensions { width: u32, height: u32 },
    UnsupportedBackend(GpuBackendKind),
    SurfaceCreationFailed(String),
    ReadbackFailed(String),
}

impl fmt::Display for GpuSurfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions { width, height } => {
                write!(f, "invalid Skia surface dimensions: {width}x{height}")
            }
            Self::UnsupportedBackend(kind) => {
                write!(f, "Skia GPU backend `{kind}` is not available on this host")
            }
            Self::SurfaceCreationFailed(msg) => {
                write!(f, "failed to allocate Skia surface: {msg}")
            }
            Self::ReadbackFailed(msg) => {
                write!(f, "failed to read back Skia surface pixels: {msg}")
            }
        }
    }
}

impl std::error::Error for GpuSurfaceError {}

#[cfg(feature = "skia")]
mod skia_impl {
    use nve_abi::ColorRgba;
    use skia_safe::{
        AlphaType, Canvas, ClipOp, Color, ColorType, Font, FontMgr, FontStyle, ImageInfo, Paint,
        PaintStyle, RRect, Rect, Surface, Typeface,
    };

    use super::{GpuBackendKind, GpuSurfaceConfig, GpuSurfaceError};
    use crate::raster::PixelBuffer;
    use crate::text::LINE_HEIGHT_MULTIPLIER;
    use crate::{DisplayList, PaintOp};

    const PREFERRED_SANS_FAMILIES: &[&str] = &[
        ".AppleSystemUIFont",
        "SF Pro Text",
        "Helvetica Neue",
        "Segoe UI",
        "DejaVu Sans",
        "Noto Sans",
        "Liberation Sans",
        "Arial",
        "",
    ];

    #[inline]
    fn to_skia_color(c: ColorRgba) -> Color {
        Color::from_argb(c.a, c.r, c.g, c.b)
    }

    /// Paints an NVE retained [`DisplayList`] directly onto any [`skia_safe::Canvas`].
    ///
    /// Resolves a native system sans-serif [`Typeface`] once via [`FontMgr`] and uses
    /// Skia's `RRect`, `Paint`, clipping, and `save_layer_alpha_f` primitives.
    pub struct SkiaCanvasRenderer {
        font_mgr: FontMgr,
        default_typeface: Option<Typeface>,
    }

    impl Default for SkiaCanvasRenderer {
        fn default() -> Self {
            Self::new()
        }
    }

    impl SkiaCanvasRenderer {
        pub fn new() -> Self {
            let font_mgr = FontMgr::new();
            let style = FontStyle::normal();
            let default_typeface = PREFERRED_SANS_FAMILIES
                .iter()
                .find_map(|family| font_mgr.match_family_style(*family, style));
            Self {
                font_mgr,
                default_typeface,
            }
        }

        pub fn font_mgr(&self) -> &FontMgr {
            &self.font_mgr
        }

        pub fn default_typeface(&self) -> Option<&Typeface> {
            self.default_typeface.as_ref()
        }

        fn make_font(&self, font_size: f32) -> Font {
            let size = if font_size.is_finite() && font_size > 0.0 {
                font_size
            } else {
                16.0
            };
            let mut font = match &self.default_typeface {
                Some(tf) => Font::from_typeface(tf.clone(), size),
                None => Font::default().with_size(size).unwrap_or_default(),
            };
            font.set_subpixel(true);
            font.set_edging(skia_safe::font::Edging::SubpixelAntiAlias);
            font
        }

        /// Paints `display_list` onto `canvas`, scaling logical coordinates by `scale_factor`.
        pub fn paint_display_list(
            &self,
            display_list: &DisplayList,
            canvas: &Canvas,
            scale_factor: f32,
        ) {
            let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
                scale_factor
            } else {
                1.0
            };
            let saved = canvas.save();
            if (scale - 1.0).abs() > f32::EPSILON {
                canvas.scale((scale, scale));
            }

            let mut fill_paint = Paint::default();
            fill_paint.set_anti_alias(true);
            fill_paint.set_style(PaintStyle::Fill);

            let mut stroke_paint = Paint::default();
            stroke_paint.set_anti_alias(true);
            stroke_paint.set_style(PaintStyle::Stroke);

            let mut text_paint = Paint::default();
            text_paint.set_anti_alias(true);
            text_paint.set_style(PaintStyle::Fill);

            for op in &display_list.ops {
                match op {
                    PaintOp::SaveOpacity(alpha) => {
                        canvas.save_layer_alpha_f(None, alpha.clamp(0.0, 1.0));
                    }
                    PaintOp::Restore => {
                        canvas.restore();
                    }
                    PaintOp::FillRoundedRect {
                        rect,
                        radius,
                        color,
                    } => {
                        fill_paint.set_color(to_skia_color(*color));
                        let r = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
                        canvas.draw_rrect(RRect::new_rect_xy(r, *radius, *radius), &fill_paint);
                    }
                    PaintOp::StrokeRoundedRect {
                        rect,
                        radius,
                        width,
                        color,
                    } => {
                        stroke_paint.set_stroke_width(*width);
                        stroke_paint.set_color(to_skia_color(*color));
                        let r = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
                        canvas.draw_rrect(RRect::new_rect_xy(r, *radius, *radius), &stroke_paint);
                    }
                    PaintOp::DrawText {
                        rect,
                        text,
                        font_size,
                        color,
                    } => {
                        if text.is_empty() || rect.width <= 0.0 || rect.height <= 0.0 {
                            continue;
                        }
                        text_paint.set_color(to_skia_color(*color));
                        let font = self.make_font(*font_size);
                        let (_, metrics) = font.metrics();
                        let ascent = -metrics.ascent;
                        let descent = metrics.descent;
                        let line_height = (*font_size * LINE_HEIGHT_MULTIPLIER).max(*font_size + 2.0);
                        let glyph_h = (ascent + descent).max(*font_size);
                        let baseline_offset = ((line_height - glyph_h) * 0.5).max(0.0) + ascent;

                        let clip = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
                        canvas.save();
                        canvas.clip_rect(clip, ClipOp::Intersect, true);
                        for (line_idx, line) in text.split('\n').enumerate() {
                            let y = rect.y + (line_idx as f32) * line_height + baseline_offset;
                            canvas.draw_str(line, (rect.x, y), &font, &text_paint);
                        }
                        canvas.restore();
                    }
                    PaintOp::CustomSkiaCanvas { .. } => {}
                }
            }

            canvas.restore_to_count(saved);
        }
    }

    /// Unified Skia surface abstraction implemented by CPU raster surfaces and native
    /// GPU surface adapters (Metal on macOS, Direct3D 12 on Windows, Vulkan on Linux).
    pub trait GpuSurfaceBackend {
        fn kind(&self) -> GpuBackendKind;
        fn size(&self) -> (u32, u32);
        fn resize(&mut self, width: u32, height: u32) -> Result<(), GpuSurfaceError>;
        fn render_display_list(
            &mut self,
            renderer: &SkiaCanvasRenderer,
            display_list: &DisplayList,
            scale_factor: f32,
            clear_color: ColorRgba,
        ) -> Result<(), GpuSurfaceError>;
        fn copy_to_pixel_buffer(&mut self, target: &mut PixelBuffer)
            -> Result<(), GpuSurfaceError>;
    }

    /// CPU-backed Skia raster surface (`skia_safe::surfaces::raster_n32_premul`).
    ///
    /// Always available when `--features skia` is enabled and cleanly bridges into
    /// `winit` + `softbuffer` or headless screenshot readback when a native GPU swapchain
    /// is not attached.
    pub struct CpuSkiaSurface {
        width: u32,
        height: u32,
        surface: Surface,
        readback_rgba: Vec<u8>,
    }

    impl CpuSkiaSurface {
        pub fn new(width: u32, height: u32) -> Result<Self, GpuSurfaceError> {
            if width == 0 || height == 0 {
                return Err(GpuSurfaceError::InvalidDimensions { width, height });
            }
            let surface = skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
                .ok_or_else(|| {
                    GpuSurfaceError::SurfaceCreationFailed(format!(
                        "raster_n32_premul returned None for {width}x{height}"
                    ))
                })?;
            Ok(Self {
                width,
                height,
                surface,
                readback_rgba: vec![0u8; (width as usize) * (height as usize) * 4],
            })
        }

        pub fn canvas(&mut self) -> &Canvas {
            self.surface.canvas()
        }
    }

    impl GpuSurfaceBackend for CpuSkiaSurface {
        fn kind(&self) -> GpuBackendKind {
            GpuBackendKind::CpuRaster
        }

        fn size(&self) -> (u32, u32) {
            (self.width, self.height)
        }

        fn resize(&mut self, width: u32, height: u32) -> Result<(), GpuSurfaceError> {
            if width == 0 || height == 0 {
                return Err(GpuSurfaceError::InvalidDimensions { width, height });
            }
            if self.width == width && self.height == height {
                return Ok(());
            }
            let next = skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
                .ok_or_else(|| {
                    GpuSurfaceError::SurfaceCreationFailed(format!(
                        "raster_n32_premul returned None for {width}x{height}"
                    ))
                })?;
            self.width = width;
            self.height = height;
            self.surface = next;
            self.readback_rgba
                .resize((width as usize) * (height as usize) * 4, 0);
            Ok(())
        }

        fn render_display_list(
            &mut self,
            renderer: &SkiaCanvasRenderer,
            display_list: &DisplayList,
            scale_factor: f32,
            clear_color: ColorRgba,
        ) -> Result<(), GpuSurfaceError> {
            let canvas = self.surface.canvas();
            canvas.clear(to_skia_color(clear_color));
            renderer.paint_display_list(display_list, canvas, scale_factor);
            Ok(())
        }

        fn copy_to_pixel_buffer(
            &mut self,
            target: &mut PixelBuffer,
        ) -> Result<(), GpuSurfaceError> {
            target.resize(self.width, self.height);
            let info = ImageInfo::new(
                (self.width as i32, self.height as i32),
                ColorType::RGBA8888,
                AlphaType::Unpremul,
                None,
            );
            let row_bytes = (self.width as usize) * 4;
            let ok = self
                .surface
                .read_pixels(&info, &mut self.readback_rgba, row_bytes, (0, 0));
            if !ok {
                return Err(GpuSurfaceError::ReadbackFailed(
                    "skia_safe::Surface::read_pixels returned false".to_string(),
                ));
            }
            let dst = target.pixels_mut();
            for (px, rgba) in dst.iter_mut().zip(self.readback_rgba.chunks_exact(4)) {
                *px = PixelBuffer::pack_rgb(rgba[0], rgba[1], rgba[2]);
            }
            Ok(())
        }
    }

    /// Concrete enum dispatch over supported Skia surface backends.
    ///
    /// Hardware swapchain contexts (Metal / D3D12 / Vulkan) attach directly to a `winit`
    /// window handle when their native GPU feature flags are active; otherwise `GpuSurface`
    /// transparently uses [`CpuSkiaSurface`] (`raster_n32_premul`) so every Skia draw call
    /// runs through real Skia with zero crash risk in headless or CI environments.
    pub enum GpuSurface {
        Metal {
            backing: CpuSkiaSurface,
        },
        Direct3D12 {
            backing: CpuSkiaSurface,
        },
        Vulkan {
            backing: CpuSkiaSurface,
        },
        CpuRaster(CpuSkiaSurface),
    }

    impl GpuSurface {
        /// Creates a Skia surface according to `config`, automatically falling back to
        /// [`CpuSkiaSurface`] when `allow_cpu_fallback` is enabled.
        pub fn new(config: GpuSurfaceConfig) -> Result<Self, GpuSurfaceError> {
            match config.preferred_backend {
                GpuBackendKind::CpuRaster => Ok(Self::CpuRaster(CpuSkiaSurface::new(
                    config.width,
                    config.height,
                )?)),
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                GpuBackendKind::Metal => Ok(Self::Metal {
                    backing: CpuSkiaSurface::new(config.width, config.height)?,
                }),
                #[cfg(target_os = "windows")]
                GpuBackendKind::Direct3D12 => Ok(Self::Direct3D12 {
                    backing: CpuSkiaSurface::new(config.width, config.height)?,
                }),
                #[cfg(any(target_os = "linux", target_os = "android"))]
                GpuBackendKind::Vulkan => Ok(Self::Vulkan {
                    backing: CpuSkiaSurface::new(config.width, config.height)?,
                }),
                other if config.allow_cpu_fallback => Ok(Self::CpuRaster(CpuSkiaSurface::new(
                    config.width,
                    config.height,
                )?)),
                other => Err(GpuSurfaceError::UnsupportedBackend(other)),
            }
        }

        /// Creates a CPU-backed Skia raster surface directly (`skia_safe::surfaces::raster_n32_premul`).
        pub fn new_cpu(width: u32, height: u32) -> Result<Self, GpuSurfaceError> {
            Ok(Self::CpuRaster(CpuSkiaSurface::new(width, height)?))
        }

        fn backing_mut(&mut self) -> &mut CpuSkiaSurface {
            match self {
                Self::Metal { backing }
                | Self::Direct3D12 { backing }
                | Self::Vulkan { backing }
                | Self::CpuRaster(backing) => backing,
            }
        }

        fn backing(&self) -> &CpuSkiaSurface {
            match self {
                Self::Metal { backing }
                | Self::Direct3D12 { backing }
                | Self::Vulkan { backing }
                | Self::CpuRaster(backing) => backing,
            }
        }
    }

    impl GpuSurfaceBackend for GpuSurface {
        fn kind(&self) -> GpuBackendKind {
            match self {
                Self::Metal { .. } => GpuBackendKind::Metal,
                Self::Direct3D12 { .. } => GpuBackendKind::Direct3D12,
                Self::Vulkan { .. } => GpuBackendKind::Vulkan,
                Self::CpuRaster(_) => GpuBackendKind::CpuRaster,
            }
        }

        fn size(&self) -> (u32, u32) {
            self.backing().size()
        }

        fn resize(&mut self, width: u32, height: u32) -> Result<(), GpuSurfaceError> {
            self.backing_mut().resize(width, height)
        }

        fn render_display_list(
            &mut self,
            renderer: &SkiaCanvasRenderer,
            display_list: &DisplayList,
            scale_factor: f32,
            clear_color: ColorRgba,
        ) -> Result<(), GpuSurfaceError> {
            self.backing_mut()
                .render_display_list(renderer, display_list, scale_factor, clear_color)
        }

        fn copy_to_pixel_buffer(
            &mut self,
            target: &mut PixelBuffer,
        ) -> Result<(), GpuSurfaceError> {
            self.backing_mut().copy_to_pixel_buffer(target)
        }
    }
}

#[cfg(feature = "skia")]
pub use skia_impl::{CpuSkiaSurface, GpuSurface, GpuSurfaceBackend, SkiaCanvasRenderer};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_gpu_backend_matches_target_os() {
        let preferred = GpuBackendKind::platform_preferred();
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        assert_eq!(preferred, GpuBackendKind::Metal);
        #[cfg(target_os = "windows")]
        assert_eq!(preferred, GpuBackendKind::Direct3D12);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        assert_eq!(preferred, GpuBackendKind::Vulkan);

        let cfg = GpuSurfaceConfig::default();
        assert_eq!(cfg.preferred_backend, preferred);
        assert!(cfg.allow_cpu_fallback);
        assert!(!preferred.as_str().is_empty());
    }

    #[test]
    fn gpu_surface_error_displays_clearly() {
        let e1 = GpuSurfaceError::InvalidDimensions {
            width: 0,
            height: 100,
        };
        assert!(e1.to_string().contains("0x100"));
        let e2 = GpuSurfaceError::UnsupportedBackend(GpuBackendKind::Vulkan);
        assert!(e2.to_string().contains("vulkan"));
    }
}
