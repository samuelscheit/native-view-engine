//! Criterion micro-benchmarks for `nve-tv-web`: DOM rule evaluation and `WebTvController` spatial
//! D-Pad key interception.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_backend_web::{WebBackend, WebKeyEventInfo};
    use nve_tv::{NodeId, SpatialFocusNode, SpatialRect, WebTvPlatform};
    use nve_tv_web::rules::{absent_pad_slots, ActivationFacts, FocusableFacts, TrapFacts};
    use nve_tv_web::{SelectActivation, WebTvConfig, WebTvController};

    fn bench_dom_decision_rules(c: &mut Criterion) {
        let focus_samples: [FocusableFacts<'_>; 4] = [
            FocusableFacts {
                tag: "BUTTON",
                ..Default::default()
            },
            FocusableFacts {
                tag: "A",
                has_href: true,
                ..Default::default()
            },
            FocusableFacts {
                tag: "DIV",
                tabindex: Some(" 0 "),
                ..Default::default()
            },
            FocusableFacts {
                tag: "INPUT",
                input_type: "hidden",
                disabled: false,
                ..Default::default()
            },
        ];
        let trap_sample = TrapFacts {
            open_dialog: true,
            modal: None,
            aria_modal: Some("true"),
            focus_trap: None,
            inert_or_hidden: false,
            rendered: true,
        };
        let act_sample = ActivationFacts {
            tag: "BUTTON",
            tabindex: Some("0"),
            disabled: false,
            inert: false,
        };

        c.bench_function("tv_web_dom_rules_64_candidates", |b| {
            b.iter(|| {
                let mut score = 0usize;
                for i in 0..64usize {
                    let f = black_box(focus_samples[i & 3]);
                    if f.is_focusable() {
                        score += 1;
                    }
                    if black_box(trap_sample).is_active() {
                        score += 1;
                    }
                    if black_box(act_sample).is_activatable() {
                        score += 1;
                    }
                    let act = SelectActivation::for_target(
                        black_box("Enter"),
                        black_box("BUTTON"),
                        black_box(""),
                        true,
                    );
                    score += usize::from(act == SelectActivation::Native);
                }
                score += absent_pad_slots(black_box(0x00FF_00FF_u64), 32).count();
                black_box(score)
            });
        });
    }

    fn bench_controller_spatial_dispatch(c: &mut Criterion) {
        let mut backend = WebBackend::new();
        let tv = WebTvController::install(
            &mut backend,
            WebTvConfig::for_platform(WebTvPlatform::Standard),
        );
        tv.with_engine(|engine| {
            let mut id = 1u32;
            for row in 0..10u32 {
                for col in 0..20u32 {
                    let rect = SpatialRect::new(
                        col as f32 * 140.0,
                        row as f32 * 100.0,
                        120.0,
                        80.0,
                    );
                    engine.register(SpatialFocusNode::new(NodeId::new(id, 0), rect));
                    id += 1;
                }
            }
        })
        .expect("engine is free");

        let right = WebKeyEventInfo::new("ArrowRight", "ArrowRight", 39);
        let left = WebKeyEventInfo::new("ArrowLeft", "ArrowLeft", 37);
        let down = WebKeyEventInfo::new("ArrowDown", "ArrowDown", 40);
        let up = WebKeyEventInfo::new("ArrowUp", "ArrowUp", 38);

        c.bench_function("tv_web_controller_4_step_nav_200_nodes", |b| {
            b.iter(|| {
                let d1 = backend.intercept_key(black_box(&right));
                let d2 = backend.intercept_key(black_box(&down));
                let d3 = backend.intercept_key(black_box(&left));
                let d4 = backend.intercept_key(black_box(&up));
                black_box((d1, d2, d3, d4))
            });
        });
    }

    criterion_group!(
        benches,
        bench_dom_decision_rules,
        bench_controller_spatial_dispatch
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
