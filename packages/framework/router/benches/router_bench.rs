//! Criterion benchmarks for `nve-router`: typed route parsing/formatting, deep-link matching,
//! `NVN1` packet codec, `split_uri`, `RawQueryIter`, and RFC 3986 percent encoding/decoding.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_router::prelude::*;
    use nve_router::{
        decode_nav_packet_ref, encode_nav_packet_into, percent_decode, percent_encode,
        percent_encode_into, split_uri, NavPacketRef, RawQueryIter,
    };

    define_routes! {
        enum BenchRoute {
            #[route("/", title = "Home")]
            Home,
            #[route("/settings/profile", title = "Profile")]
            Profile,
            #[route("/issues/:id", title = "Issue")]
            Issue {
                id: u64,
                #[query]
                tab: Option<String>,
                #[query("p")]
                page: Option<u32>,
                #[fragment]
                section: Option<String>,
            },
            #[route("/docs/*slug", title = "Docs")]
            Docs { slug: String },
        }
    }

    fn bench_router(c: &mut Criterion) {
        let mut g = c.benchmark_group("router");

        let issue_uri = "/issues/42?tab=comments%20%26%20notes&p=3#top%20anchor";
        let docs_uri = "/docs/guides/getting%20started/v2";
        let issue_route = BenchRoute::parse_uri(issue_uri).expect("valid bench issue uri");
        let deep_link = DeepLinkConfig::new()
            .with_scheme("nve")
            .with_universal_host("app.example.com")
            .with_base_prefix("/app");

        g.bench_function("route_parse_static", |b| {
            b.iter(|| black_box(BenchRoute::parse_uri(black_box("/settings/profile"))));
        });

        g.bench_function("route_parse_typed_query_fragment", |b| {
            b.iter(|| black_box(BenchRoute::parse_uri(black_box(issue_uri))));
        });

        g.bench_function("route_parse_wildcard", |b| {
            b.iter(|| black_box(BenchRoute::parse_uri(black_box(docs_uri))));
        });

        g.bench_function("route_to_path_typed_query_fragment", |b| {
            b.iter(|| black_box(black_box(&issue_route).to_path().len()));
        });

        g.bench_function("deeplink_match_universal", |b| {
            b.iter(|| {
                let m = deep_link
                    .match_uri::<BenchRoute>(black_box("https://app.example.com/app/issues/42?p=2"))
                    .expect("valid universal link");
                black_box(m.normalized_path.len())
            });
        });

        let pkt = NavPacketRef {
            action: NavigationAction::Push,
            can_go_back: true,
            can_go_forward: false,
            is_modal: false,
            stack_depth: 3,
            active_tab_index: 1,
            uri: issue_uri,
            tab_id: "issues",
        };
        let mut wire = Vec::with_capacity(128);
        encode_nav_packet_into(pkt, &mut wire).expect("encode bench NVN1");

        g.bench_function("nvn1_encode_nav_packet_into_warm", |b| {
            b.iter(|| {
                wire.clear();
                encode_nav_packet_into(black_box(pkt), &mut wire).expect("encode NVN1");
                black_box(wire.len())
            });
        });

        g.bench_function("nvn1_decode_nav_packet_ref", |b| {
            b.iter(|| {
                let decoded = decode_nav_packet_ref(black_box(&wire)).expect("decode NVN1");
                black_box(decoded.uri.len())
            });
        });

        g.bench_function("split_uri_and_raw_query_iter", |b| {
            b.iter(|| {
                let parts = split_uri(black_box(issue_uri));
                let mut count = 0usize;
                if let Some(q) = parts.raw_query {
                    for (k, v) in RawQueryIter::new(q) {
                        count += k.len() + v.len();
                    }
                }
                black_box((parts.path.len(), count))
            });
        });

        let mut enc_buf = String::with_capacity(128);
        g.bench_function("percent_encode_borrowed_fast_path", |b| {
            b.iter(|| black_box(percent_encode(black_box("alpha-09._~")).len()));
        });

        g.bench_function("percent_encode_into_escaped_warm", |b| {
            b.iter(|| {
                enc_buf.clear();
                percent_encode_into(black_box("café 100% /a?b#c"), &mut enc_buf);
                black_box(enc_buf.len())
            });
        });

        g.bench_function("percent_decode_borrowed_fast_path", |b| {
            b.iter(|| {
                let dec = percent_decode(black_box("plain_value_123")).expect("valid decode");
                black_box(dec.len())
            });
        });

        g.bench_function("percent_decode_escaped_utf8", |b| {
            b.iter(|| {
                let dec = percent_decode(black_box("caf%C3%A9%20100%25")).expect("valid decode");
                black_box(dec.len())
            });
        });

        g.finish();
    }

    criterion_group!(benches, bench_router);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
