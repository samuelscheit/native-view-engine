//! Registry of all 5 interactive Dioxus playgrounds embedded in `docs/content/`.

pub mod canvas_draw1;
pub mod crdt_simulator;
pub mod router_sandbox;
pub mod ui_playground;
pub mod virtual_list;

use std::sync::{Arc, OnceLock};

use nve_dioxus::prelude::*;
use nve_docs::{PlaygroundMeta, PlaygroundRegistry};

use canvas_draw1::{CANVAS_DRAW1_SOURCE, CanvasDraw1Demo};
use crdt_simulator::{CRDT_SIMULATOR_SOURCE, CrdtSimulatorDemo};
use router_sandbox::{ROUTER_SANDBOX_SOURCE, RouterSandboxDemo};
use ui_playground::{UI_PLAYGROUND_SOURCE, UiPlaygroundDemo};
use virtual_list::{VIRTUAL_LIST_SOURCE, VirtualList100kDemo};

/// Build a fresh [`PlaygroundRegistry`] containing all 5 interactive docs demos.
pub fn build_playground_registry() -> PlaygroundRegistry {
    let mut reg = PlaygroundRegistry::new();

    reg.register(
        PlaygroundMeta::new(
            "ui-playground",
            "nve-ui Interactive Component Kitchen Sink",
            UI_PLAYGROUND_SOURCE,
        )
        .with_description(
            "Live Button variants, Switch, Checkbox, Input, Badge, Tabs, and Toast notifications.",
        )
        .with_default_height(280),
        || rsx! { UiPlaygroundDemo {} },
    );

    reg.register(
        PlaygroundMeta::new(
            "virtual-list-100k",
            "100,000-Row VirtualList Benchmark Sandbox",
            VIRTUAL_LIST_SOURCE,
        )
        .with_description(
            "O(1) windowed virtualization across 100,000 rows with instant filters and jump controls.",
        )
        .with_default_height(300),
        || rsx! { VirtualList100kDemo {} },
    );

    reg.register(
        PlaygroundMeta::new(
            "router-sandbox",
            "nve-router Stack, Tabs, Modal & Guard Sandbox",
            ROUTER_SANDBOX_SOURCE,
        )
        .with_description(
            "Push/pop routes, toggle auth redirect guards, switch tabs, and inspect live history state.",
        )
        .with_default_height(260),
        || rsx! { RouterSandboxDemo {} },
    );

    reg.register(
        PlaygroundMeta::new(
            "canvas-draw1",
            "NVE_DRAW1 Zero-JSON Vector Canvas",
            CANVAS_DRAW1_SOURCE,
        )
        .with_description(
            "Switch scenes, palettes, and stroke widths compiled directly to binary NVE_DRAW1 draw ops.",
        )
        .with_default_height(260),
        || rsx! { CanvasDraw1Demo {} },
    );

    reg.register(
        PlaygroundMeta::new(
            "crdt-simulator",
            "Two-Peer Local-First Binary NVC1 CRDT Simulator",
            CRDT_SIMULATOR_SOURCE,
        )
        .with_description(
            "Simulate offline partitions and binary NVC1 delta sync across LwwRegister, PnCounter, and OrSet.",
        )
        .with_default_height(290),
        || rsx! { CrdtSimulatorDemo {} },
    );

    reg
}

/// Returns the shared cached `Arc<PlaygroundRegistry>` for all 5 official playgrounds.
pub fn nve_docs_playground_registry() -> Arc<PlaygroundRegistry> {
    static REGISTRY: OnceLock<Arc<PlaygroundRegistry>> = OnceLock::new();
    Arc::clone(REGISTRY.get_or_init(|| Arc::new(build_playground_registry())))
}
