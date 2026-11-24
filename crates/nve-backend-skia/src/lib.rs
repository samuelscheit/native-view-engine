//! Cross-platform Skia RenderBackend targeting macOS, Windows, Linux, iOS, and Android.
//!
//! Consumes the exact same `nve-abi` binary `CommandBuffer` as the React Native Fabric backend,
//! computes Flexbox layout, compiles a retained `DisplayList`, and (with `--features skia`)
//! paints directly onto a `skia_safe::Canvas`.

use std::time::Instant;

use nve_abi::{ColorRgba, ComponentKind, NativeEvent, NodeId, SurfaceId, ROOT_NODE_ID};
use nve_backend_headless::HeadlessBackend;
use nve_engine::{BackendApplyStats, BackendError, LayoutRect, RenderBackend, Viewport};

#[derive(Debug, Clone, PartialEq)]
pub enum PaintOp {
    SaveOpacity(f32),
    Restore,
    FillRoundedRect {
        rect: LayoutRect,
        radius: f32,
        color: ColorRgba,
    },
    StrokeRoundedRect {
        rect: LayoutRect,
        radius: f32,
        width: f32,
        color: ColorRgba,
    },
    DrawText {
        rect: LayoutRect,
        text: String,
        font_size: f32,
        color: ColorRgba,
    },
    CustomSkiaCanvas {
        node_id: NodeId,
        rect: LayoutRect,
    },
}

#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    pub ops: Vec<PaintOp>,
}

pub struct SkiaBackend {
    layout_tree: HeadlessBackend,
    display_list: DisplayList,
}

impl Default for SkiaBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SkiaBackend {
    pub fn new() -> Self {
        Self {
            layout_tree: HeadlessBackend::new(),
            display_list: DisplayList::default(),
        }
    }

    pub fn display_list(&self) -> &DisplayList {
        &self.display_list
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        self.layout_tree.hit_test(x, y, true)
    }

    fn rebuild_display_list(&mut self) {
        self.display_list.ops.clear();
        self.emit_node(ROOT_NODE_ID);
    }

    fn emit_node(&mut self, id: NodeId) {
        let Some(node) = self.layout_tree.node(id).cloned() else {
            return;
        };
        let pushed_opacity = node.style.opacity < 0.999;
        if pushed_opacity {
            self.display_list
                .ops
                .push(PaintOp::SaveOpacity(node.style.opacity));
        }

        if node.style.background.a > 0 {
            self.display_list.ops.push(PaintOp::FillRoundedRect {
                rect: node.computed_rect,
                radius: node.style.border_radius,
                color: node.style.background,
            });
        }
        if node.style.border_width > 0.0 && node.style.border_color.a > 0 {
            self.display_list.ops.push(PaintOp::StrokeRoundedRect {
                rect: node.computed_rect,
                radius: node.style.border_radius,
                width: node.style.border_width,
                color: node.style.border_color,
            });
        }
        if let Some(text) = node.text {
            self.display_list.ops.push(PaintOp::DrawText {
                rect: node.computed_rect,
                text,
                font_size: node.style.font_size,
                color: node.style.text_color,
            });
        }
        if node.kind == ComponentKind::SkiaCanvas {
            self.display_list.ops.push(PaintOp::CustomSkiaCanvas {
                node_id: id,
                rect: node.computed_rect,
            });
        }

        for child in node.children {
            self.emit_node(child);
        }

        if pushed_opacity {
            self.display_list.ops.push(PaintOp::Restore);
        }
    }

    #[cfg(feature = "skia")]
    pub fn paint_to_canvas(&self, canvas: &skia_safe::Canvas) {
        use skia_safe::{Color, Font, Paint, PaintStyle, RRect, Rect};
        let to_color = |c: ColorRgba| Color::from_argb(c.a, c.r, c.g, c.b);
        for op in &self.display_list.ops {
            match op {
                PaintOp::SaveOpacity(alpha) => {
                    let mut p = Paint::default();
                    p.set_alpha_f(*alpha);
                    canvas.save_layer(&skia_safe::canvas::SaveLayerRec::default().paint(&p));
                }
                PaintOp::Restore => {
                    canvas.restore();
                }
                PaintOp::FillRoundedRect {
                    rect,
                    radius,
                    color,
                } => {
                    let mut p = Paint::default();
                    p.set_anti_alias(true);
                    p.set_color(to_color(*color));
                    let r = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
                    canvas.draw_rrect(RRect::new_rect_xy(r, *radius, *radius), &p);
                }
                PaintOp::StrokeRoundedRect {
                    rect,
                    radius,
                    width,
                    color,
                } => {
                    let mut p = Paint::default();
                    p.set_anti_alias(true);
                    p.set_style(PaintStyle::Stroke);
                    p.set_stroke_width(*width);
                    p.set_color(to_color(*color));
                    let r = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
                    canvas.draw_rrect(RRect::new_rect_xy(r, *radius, *radius), &p);
                }
                PaintOp::DrawText {
                    rect,
                    text,
                    font_size,
                    color,
                } => {
                    let mut p = Paint::default();
                    p.set_anti_alias(true);
                    p.set_color(to_color(*color));
                    let font = Font::default().with_size(*font_size).unwrap_or_default();
                    canvas.draw_str(text, (rect.x, rect.y + *font_size), &font, &p);
                }
                PaintOp::CustomSkiaCanvas { .. } => {}
            }
        }
    }
}

impl RenderBackend for SkiaBackend {
    fn name(&self) -> &'static str {
        if cfg!(feature = "skia") {
            "skia-gpu"
        } else {
            "skia-displaylist"
        }
    }

    fn set_viewport(&mut self, surface_id: SurfaceId, viewport: Viewport) {
        self.layout_tree.set_viewport(surface_id, viewport);
    }

    fn submit_frame(&mut self, command_bytes: &[u8]) -> Result<BackendApplyStats, BackendError> {
        let mut stats = self.layout_tree.submit_frame(command_bytes)?;
        let t_mount = Instant::now();
        self.rebuild_display_list();
        stats.mount_us = t_mount.elapsed().as_micros() as u64;
        Ok(stats)
    }

    fn drain_events(&mut self) -> Vec<NativeEvent> {
        self.layout_tree.drain_events()
    }

    fn layout_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        self.layout_tree.layout_of(node_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::{CommandBufferBuilder, Dimension, LayoutProps, StyleProps};

    #[test]
    fn emits_retained_display_list_from_command_buffer() {
        let mut skia = SkiaBackend::new();
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(200.0),
                height: Dimension::Points(80.0),
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                background: ColorRgba::rgb(30, 41, 59),
                border_radius: 12.0,
                ..Default::default()
            },
        );
        b.create_node(3, ComponentKind::Text, None);
        b.set_text(3, "Skia Ready");
        b.set_children(2, &[3]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);

        skia.submit_frame(b.finish()).unwrap();
        assert_eq!(skia.display_list().ops.len(), 2);
    }
}
