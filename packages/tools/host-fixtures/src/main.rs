//! `nve-host-fixtures <out_dir>`: writes real `NVE2` frames built with the production
//! `NodeStore` + standard tables, so host suites (Android JNI on a desktop JVM, C++) exercise
//! `libnve_runtime` end to end instead of hand-encoded wire bytes.
//!
//! Files: `showcase_1.nve2` (keyframe), `showcase_2.nve2` (delta: text edit, image swap, node
//! removal), `showcase_3.nve2` (a new image source recycling the freed logo id, the canvas draw
//! list moved to an ephemeral resource), `showcase_4.nve2` (that ephemeral draw list rewritten
//! in place: same id, new bytes), `hot_v1.nve2` / `hot_v2.nve2` (hot-reload generations), and
//! `showcase.ids` (`name=value` lines: raw node ids and resource ids).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use nve_abi::{ColorRgba, EventMask};
use nve_canvas::{Canvas, DrawList};
use nve_components::tables::{
    Accessibility, AlignItems, AriaRole, DebugId, FlexDirection, FontStyle, Image, ImageFit, ImageSrc,
    Interaction, Layout, Scroll, ScrollAxes, TextAlign, TextContent, TextInput, Typography, Visual,
};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba, Transaction};

struct Source {
    store: NodeStore,
    enc: WireEncoder,
}

impl Source {
    fn new() -> Self {
        Self { store: NodeStore::new(), enc: WireEncoder::default() }
    }

    fn frame(&mut self, f: impl FnOnce(&mut Transaction<'_>)) -> Vec<u8> {
        let mut changes = ChangeSet::default();
        let mut tx = self.store.tx();
        f(&mut tx);
        tx.commit_into(&mut changes);
        let mut out = Vec::new();
        self.enc.encode(&self.store, &changes, &mut out);
        out
    }
}

/// Node handles of the showcase scene.
struct Showcase {
    column: NodeId,
    title: NodeId,
    input: NodeId,
    image: NodeId,
    scroll: NodeId,
    rows: [NodeId; 3],
    canvas: NodeId,
    button: NodeId,
}

fn showcase_keyframe(tx: &mut Transaction<'_>) -> Showcase {
    let column = tx.create();
    tx.edit::<Layout>(column)
        .width(Length::points(360.0))
        .flex_direction(FlexDirection::Column)
        .align_items(AlignItems::FlexStart);
    tx.edit::<Visual>(column)
        .background(Rgba::rgb(15, 23, 42))
        .border_color(Rgba::rgb(56, 189, 248))
        .border_width(2.0)
        .border_radius(12.0)
        .opacity(0.9);
    // Inherited by every text descendant: color, italic, centered (the title overrides the color).
    tx.edit::<Typography>(column)
        .set(Typography::SET_COLOR | Typography::SET_FONT_STYLE | Typography::SET_TEXT_ALIGN)
        .color(Rgba::rgb(1, 2, 3))
        .font_style(FontStyle::Italic)
        .text_align(TextAlign::Center);

    let title = tx.create();
    tx.edit::<TextContent>(title).text("Hello NVE");
    tx.edit::<Typography>(title)
        .set(Typography::SET_COLOR | Typography::SET_FONT_SIZE | Typography::SET_FONT_WEIGHT)
        .color(Rgba::rgb(248, 250, 252))
        .font_size(24.0)
        .font_weight(700);
    tx.edit::<Accessibility>(title).role(AriaRole::Heading).level(1);

    let input = tx.create();
    tx.edit::<Layout>(input).width(Length::points(200.0)).height(Length::points(40.0));
    tx.edit::<TextInput>(input).value("draft").placeholder("Type here").secure(true);
    tx.edit::<Accessibility>(input).label("Password");

    let image = tx.create();
    tx.edit::<Layout>(image).width(Length::points(64.0)).height(Length::points(64.0));
    tx.edit::<Image>(image)
        .src(Some(ImageSrc::Url("asset://logo.png".into())))
        .fit(ImageFit::Contain)
        .alt("Logo");

    let scroll = tx.create();
    tx.edit::<Layout>(scroll).width(Length::points(300.0)).height(Length::points(80.0));
    tx.edit::<Scroll>(scroll).axes(ScrollAxes::Vertical);
    let rows = [tx.create(), tx.create(), tx.create()];
    for (i, &row) in rows.iter().enumerate() {
        tx.edit::<Layout>(row).width(Length::points(300.0)).height(Length::points(50.0)).flex_shrink(0.0);
        tx.edit::<TextContent>(row).text(&format!("Row {i}"));
    }
    tx.set_children(scroll, &rows);

    let canvas = tx.create();
    tx.edit::<Layout>(canvas).width(Length::points(120.0)).height(Length::points(40.0));
    let mut draw = DrawList::builder();
    draw.fill_rect(0.0, 0.0, 120.0, 40.0, ColorRgba::rgba(30, 41, 59, 255));
    tx.edit::<Canvas>(canvas).draw_list(Some(draw.finish())).generation(1);

    let button = tx.create();
    tx.edit::<Layout>(button).width(Length::points(120.0)).height(Length::points(44.0));
    tx.edit::<Interaction>(button).event_mask(EventMask::PRESS as u32);
    tx.edit::<Accessibility>(button).role(AriaRole::Button).label("Submit");
    tx.edit::<DebugId>(button).test_id("submit-btn");
    tx.edit::<TextContent>(button).text("Submit");

    tx.set_children(column, &[title, input, image, scroll, canvas, button]);
    tx.set_children(NodeId::ROOT, &[column]);
    Showcase { column, title, input, image, scroll, rows, canvas, button }
}

fn hot_frame(tx: &mut Transaction<'_>, label: &str) {
    let text = tx.create();
    tx.edit::<TextContent>(text).text(label);
    tx.set_children(NodeId::ROOT, &[text]);
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(dir.join(name), bytes).map_err(|e| format!("write {name}: {e}"))
}

fn run(dir: PathBuf) -> Result<(), String> {
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let mut src = Source::new();
    let mut ids = None;
    let frame1 = src.frame(|tx| ids = Some(showcase_keyframe(tx)));
    let s = ids.ok_or("showcase keyframe produced no ids")?;
    write(&dir, "showcase_1.nve2", &frame1)?;
    let res = |id: NodeId, src: &Source| src.store.try_row::<Image>(id).map_or(0, |r| r.src().0);
    let logo = res(s.image, &src);
    let canvas_res = src.store.try_row::<Canvas>(s.canvas).map_or(0, |r| r.draw_list().0);

    let frame2 = src.frame(|tx| {
        tx.edit::<TextContent>(s.title).text("Hello again");
        tx.edit::<Image>(s.image).src(Some(ImageSrc::EncodedBytes(vec![0x89, b'P', b'N', b'G'].into())));
        tx.remove(s.rows[2]);
        tx.set_children(s.scroll, &s.rows[..2]);
    });
    write(&dir, "showcase_2.nve2", &frame2)?;
    let hero = res(s.image, &src);

    let draw = |w: f32| {
        let mut d = DrawList::builder();
        d.fill_rect(0.0, 0.0, w, 40.0, ColorRgba::rgba(30, 41, 59, 255));
        d.finish()
    };
    let canvas_res_of = |src: &Source| src.store.try_row::<Canvas>(s.canvas).map_or(0, |r| r.draw_list().0);
    let frame3 = src.frame(|tx| {
        tx.edit::<Image>(s.image).src(Some(ImageSrc::Url("asset://recycled.png".into())));
        tx.edit::<Canvas>(s.canvas).draw_list_ephemeral(draw(60.0)).generation(2);
    });
    write(&dir, "showcase_3.nve2", &frame3)?;
    let recycled = res(s.image, &src);
    let canvas_eph = canvas_res_of(&src);
    let frame4 = src.frame(|tx| {
        tx.edit::<Canvas>(s.canvas).draw_list_ephemeral(draw(90.0)).generation(3);
    });
    write(&dir, "showcase_4.nve2", &frame4)?;
    if canvas_res_of(&src) != canvas_eph {
        return Err("the ephemeral draw list was not rewritten in place".into());
    }

    let mut out = String::new();
    for (name, id) in [
        ("column", s.column),
        ("title", s.title),
        ("input", s.input),
        ("image", s.image),
        ("scroll", s.scroll),
        ("row0", s.rows[0]),
        ("row1", s.rows[1]),
        ("row2", s.rows[2]),
        ("canvas", s.canvas),
        ("button", s.button),
    ] {
        let _ = writeln!(out, "{name}={}", id.raw());
    }
    let _ = writeln!(
        out,
        "logo_res={logo}\nhero_res={hero}\ncanvas_res={canvas_res}\nrecycled_res={recycled}\ncanvas_eph_res={canvas_eph}"
    );
    write(&dir, "showcase.ids", out.as_bytes())?;

    for (name, label) in [("hot_v1.nve2", "generation one"), ("hot_v2.nve2", "generation two")] {
        let mut hot = Source::new();
        write(&dir, name, &hot.frame(|tx| hot_frame(tx, label)))?;
    }
    Ok(())
}

fn main() {
    let Some(dir) = std::env::args_os().nth(1) else {
        eprintln!("usage: nve-host-fixtures <out_dir>");
        std::process::exit(2);
    };
    if let Err(e) = run(PathBuf::from(dir)) {
        eprintln!("nve-host-fixtures: {e}");
        std::process::exit(1);
    }
}
