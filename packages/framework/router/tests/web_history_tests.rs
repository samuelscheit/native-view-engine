//! Comprehensive tests for `nve_router::web` (`HistoryBackend`, `MemoryHistoryBackend`,
//! `WebHistoryAdapter`, `normalize_base_path`, `popstate` loop-prevention, guard redirects,
//! anchor scroll dispatch, and SPA link-click interception).

use nve_router::prelude::*;

define_routes! {
    pub enum DocsRoute {
        #[route("/", title = "Home")]
        Home,
        #[route("/login", title = "Sign In")]
        Login,
        #[route("/admin", title = "Admin")]
        Admin,
        #[route("/docs/*slug", title = "Docs")]
        Docs {
            slug: String,
            #[query]
            version: Option<String>,
            #[fragment]
            heading: Option<String>,
        },
        #[route("/guide/*slug", title = "Guide")]
        Guide { slug: String },
        #[route("/search", title = "Search")]
        Search {
            #[query]
            q: String,
            #[query("p")]
            page: Option<u32>,
        },
    }
}

#[test]
fn base_path_normalization_and_href_for_consistency() {
    for (raw, expected) in [
        ("", ""),
        ("/", ""),
        ("///", ""),
        ("   ", ""),
        ("/native-view-engine", "/native-view-engine"),
        ("/native-view-engine/", "/native-view-engine"),
        ("native-view-engine", "/native-view-engine"),
        ("native-view-engine/", "/native-view-engine"),
        ("///org//native-view-engine///", "/org/native-view-engine"),
    ] {
        assert_eq!(normalize_base_path(raw), expected, "raw={raw:?}");
    }

    for root_base in ["", "/"] {
        let adapter: WebHistoryAdapter<DocsRoute, _> =
            WebHistoryAdapter::new(root_base, MemoryHistoryBackend::new("/"));
        assert_eq!(adapter.base_path(), "");
        assert_eq!(adapter.href_for(&DocsRoute::Home), "/");
        assert_eq!(
            adapter.href_for(&DocsRoute::Docs {
                slug: "getting started/intro".into(),
                version: Some("v2".into()),
                heading: Some("quick start".into()),
            }),
            "/docs/getting%20started/intro?version=v2#quick%20start"
        );
    }

    for sub_base in ["/native-view-engine", "/native-view-engine/", "native-view-engine"] {
        let adapter: WebHistoryAdapter<DocsRoute, _> =
            WebHistoryAdapter::new(sub_base, MemoryHistoryBackend::new("/native-view-engine/"));
        assert_eq!(adapter.base_path(), "/native-view-engine");
        assert_eq!(adapter.href_for(&DocsRoute::Home), "/native-view-engine/");
        assert_eq!(
            adapter.href_for(&DocsRoute::Docs {
                slug: "arch/abi".into(),
                version: Some("0.7".into()),
                heading: Some("command-buffer".into()),
            }),
            "/native-view-engine/docs/arch/abi?version=0.7#command-buffer"
        );
        assert_eq!(
            adapter.href_for(&DocsRoute::Search {
                q: "zero copy".into(),
                page: Some(2),
            }),
            "/native-view-engine/search?q=zero%20copy&p=2"
        );
    }
}

#[test]
fn initial_route_parses_subpaths_trailing_slashes_queries_percent_encoding_and_anchors() {
    for initial_href in [
        "/native-view-engine",
        "/native-view-engine/",
        "https://user.github.io/native-view-engine",
        "https://user.github.io/native-view-engine/",
    ] {
        let adapter: WebHistoryAdapter<DocsRoute, _> =
            WebHistoryAdapter::new("/native-view-engine/", MemoryHistoryBackend::new(initial_href));
        assert_eq!(adapter.initial_route(), Some(DocsRoute::Home), "href={initial_href}");
        assert_eq!(adapter.current_anchor(), None);
    }

    let nested_href =
        "/native-view-engine/docs/getting%20started/install/?version=1.2#step%20one";
    let adapter: WebHistoryAdapter<DocsRoute, _> =
        WebHistoryAdapter::new("/native-view-engine", MemoryHistoryBackend::new(nested_href));
    let expected = DocsRoute::Docs {
        slug: "getting started/install".into(),
        version: Some("1.2".into()),
        heading: Some("step one".into()),
    };
    assert_eq!(adapter.initial_route(), Some(expected.clone()));
    assert_eq!(
        adapter.initial_route_with_anchor(),
        Some((expected, Some("step one".into())))
    );
    assert_eq!(adapter.current_anchor(), Some("step one".into()));

    // Route without `#[fragment]` still surfaces anchor via `initial_route_with_anchor`.
    let guide_adapter: WebHistoryAdapter<DocsRoute, _> = WebHistoryAdapter::new(
        "/native-view-engine",
        MemoryHistoryBackend::new("/native-view-engine/guide/ffi-bridge#zero-copy"),
    );
    assert_eq!(
        guide_adapter.initial_route_with_anchor(),
        Some((DocsRoute::Guide { slug: "ffi-bridge".into() }, Some("zero-copy".into())))
    );

    // Query params containing "://" (encoded or unencoded) must not trigger scheme stripping.
    for href in [
        "/native-view-engine/search?q=https%3A%2F%2Fexample.com%2Fa&p=4",
        "/native-view-engine/search?q=https://example.com/a&p=4",
    ] {
        let a: WebHistoryAdapter<DocsRoute, _> =
            WebHistoryAdapter::new("/native-view-engine", MemoryHistoryBackend::new(href));
        assert_eq!(
            a.initial_route(),
            Some(DocsRoute::Search { q: "https://example.com/a".into(), page: Some(4) }),
            "href={href}"
        );
    }

    // Outside base_path, sibling prefix, double slashes, and invalid percent-escapes reject.
    for rejected in [
        "/",
        "/docs/intro",
        "/native-view-engine-2/docs/intro",
        "/native-view-engine//docs/intro",
        "/native-view-engine/docs/bad%ZZ",
        "/native-view-engine/search?q=ok&p=not-a-number",
    ] {
        let bad: WebHistoryAdapter<DocsRoute, _> =
            WebHistoryAdapter::new("/native-view-engine", MemoryHistoryBackend::new(rejected));
        assert_eq!(bad.initial_route(), None, "rejected={rejected:?}");
    }
}

#[test]
fn sync_push_and_replace_deduplicate_and_dispatch_scrolls() {
    let mut adapter: WebHistoryAdapter<DocsRoute, _> = WebHistoryAdapter::new(
        "/native-view-engine/",
        MemoryHistoryBackend::new("/native-view-engine/"),
    );

    // Pushing current URL is a no-op.
    assert!(!adapter.sync_push(&DocsRoute::Home));
    assert_eq!(adapter.backend().push_count(), 0);
    assert!(adapter.backend().scroll_events().is_empty());

    // Pushing new route appends and scrolls to top.
    let guide = DocsRoute::Guide { slug: "architecture".into() };
    assert!(adapter.sync_push(&guide));
    assert_eq!(adapter.backend().push_count(), 1);
    assert_eq!(adapter.backend().current_href(), "/native-view-engine/guide/architecture");
    assert_eq!(adapter.backend().scroll_events(), &[ScrollTarget::Top]);

    // Repeat push deduplicates.
    assert!(!adapter.sync_push(&guide));
    assert_eq!(adapter.backend().push_count(), 1);
    assert_eq!(adapter.backend().len(), 2);

    // Route with `#heading` scrolls to decoded anchor.
    let docs_anchor = DocsRoute::Docs {
        slug: "router".into(),
        version: None,
        heading: Some("history api".into()),
    };
    assert!(adapter.sync_push(&docs_anchor));
    assert_eq!(adapter.backend().current_href(), "/native-view-engine/docs/router#history%20api");
    assert_eq!(adapter.backend().last_scroll(), Some(&ScrollTarget::Anchor("history api".into())));

    // `sync_push_with_anchor` overrides existing fragment cleanly without `#a#b`.
    assert!(adapter.sync_push_with_anchor(&docs_anchor, Some("#deep-link")));
    assert_eq!(adapter.backend().current_href(), "/native-view-engine/docs/router#deep-link");
    assert_eq!(adapter.backend().last_scroll(), Some(&ScrollTarget::Anchor("deep-link".into())));

    // `sync_replace` updates in place and deduplicates repeats.
    let search = DocsRoute::Search { q: "layout".into(), page: Some(1) };
    let len_before = adapter.backend().len();
    assert!(adapter.sync_replace(&search));
    assert_eq!(adapter.backend().len(), len_before);
    assert_eq!(adapter.backend().replace_count(), 1);
    assert_eq!(adapter.backend().current_href(), "/native-view-engine/search?q=layout&p=1");
    assert_eq!(adapter.backend().last_scroll(), Some(&ScrollTarget::Top));

    assert!(!adapter.sync_replace(&search));
    assert_eq!(adapter.backend().replace_count(), 1);
}

#[test]
fn popstate_sync_preserves_forward_history_and_prevents_push_loops() {
    let mut adapter: WebHistoryAdapter<DocsRoute, _> = WebHistoryAdapter::new(
        "/native-view-engine",
        MemoryHistoryBackend::new("/native-view-engine/"),
    );
    let mut router = RouterState::new(DocsRoute::Home);

    let intro = DocsRoute::Guide { slug: "intro".into() };
    let api = DocsRoute::Docs {
        slug: "api".into(),
        version: Some("1".into()),
        heading: Some("methods".into()),
    };

    assert!(adapter.sync_transition(&router.push(intro.clone()).unwrap()));
    assert!(adapter.sync_transition(&router.push(api.clone()).unwrap()));
    assert_eq!((adapter.backend().push_count(), adapter.backend().replace_count()), (2, 0));

    // Back -> `intro`: preserves forward history, zero duplicate push/replace.
    adapter.backend_mut().clear_scroll_events();
    assert_eq!(adapter.backend_mut().go_back(), Some("/native-view-engine/guide/intro"));
    assert_eq!(adapter.handle_popstate(&mut router), Some(intro.clone()));
    assert_eq!((router.cursor(), router.can_go_back(), router.can_go_forward()), (1, true, true));
    assert_eq!((adapter.backend().push_count(), adapter.backend().replace_count()), (2, 0));

    // Even a redundant `sync_push` from a reactive effect stays loop-free.
    assert!(!adapter.sync_push(router.current_route()));
    assert_eq!(adapter.backend().push_count(), 2);

    // Back -> `Home`, then Forward -> `intro`, then Forward -> `api` (with `#methods` scroll).
    assert_eq!(adapter.backend_mut().go_back(), Some("/native-view-engine/"));
    assert_eq!(adapter.handle_popstate(&mut router), Some(DocsRoute::Home));
    assert_eq!((router.cursor(), router.can_go_back(), router.can_go_forward()), (0, false, true));

    adapter.backend_mut().go_forward();
    assert_eq!(adapter.handle_popstate(&mut router), Some(intro.clone()));
    assert_eq!(router.cursor(), 1);

    adapter.backend_mut().clear_scroll_events();
    adapter.backend_mut().go_forward();
    assert_eq!(adapter.handle_popstate(&mut router), Some(api));
    assert_eq!(router.cursor(), 2);
    assert_eq!(adapter.backend().scroll_events(), &[ScrollTarget::Anchor("methods".into())]);
    assert_eq!((adapter.backend().push_count(), adapter.backend().replace_count()), (2, 0));

    // Same-route browser `#hash` popstate scrolls to anchor without mutating history.
    let mut same_router = RouterState::new(intro.clone());
    let mut same_adapter: WebHistoryAdapter<DocsRoute, _> = WebHistoryAdapter::new(
        "/native-view-engine",
        MemoryHistoryBackend::new("/native-view-engine/guide/intro"),
    );
    same_adapter
        .backend_mut()
        .navigate_browser("/native-view-engine/guide/intro#install-steps");
    assert_eq!(same_adapter.handle_popstate(&mut same_router), Some(intro));
    assert_eq!((same_adapter.backend().push_count(), same_adapter.backend().replace_count()), (0, 0));
    assert_eq!(
        same_adapter.backend().scroll_events(),
        &[ScrollTarget::Anchor("install-steps".into())]
    );
}

#[test]
fn guard_redirect_and_block_update_browser_history_via_replace_state() {
    let redirect_admin = |ctx: &NavigationContext<'_, DocsRoute>| {
        if *ctx.to == DocsRoute::Admin {
            GuardDecision::Redirect(DocsRoute::Login)
        } else {
            GuardDecision::Allow
        }
    };

    // 1. Guard redirect on `handle_popstate`: `/admin` -> `/login` via `replace_state`.
    let mut adapter: WebHistoryAdapter<DocsRoute, _> =
        WebHistoryAdapter::new("/app", MemoryHistoryBackend::new("/app/"));
    let mut router = RouterState::new(DocsRoute::Home);
    router.add_guard_fn(redirect_admin);
    adapter.backend_mut().navigate_browser("/app/admin");
    assert_eq!(adapter.handle_popstate(&mut router), Some(DocsRoute::Login));
    assert_eq!(router.current_route(), &DocsRoute::Login);
    assert_eq!(adapter.backend().current_href(), "/app/login");
    assert_eq!((adapter.backend().push_count(), adapter.backend().replace_count()), (0, 1));

    // 2. Guard redirect on `apply_initial_route`.
    let mut init_adapter: WebHistoryAdapter<DocsRoute, _> =
        WebHistoryAdapter::new("/app", MemoryHistoryBackend::new("/app/admin"));
    let mut init_router = RouterState::new(DocsRoute::Home);
    init_router.add_guard_fn(redirect_admin);
    assert_eq!(init_adapter.apply_initial_route(&mut init_router), Some(DocsRoute::Login));
    assert_eq!(init_adapter.backend().current_href(), "/app/login");
    assert_eq!((init_adapter.backend().push_count(), init_adapter.backend().replace_count()), (0, 1));

    // 3. Guard block on `handle_popstate` restores browser URL via `replace_state`.
    let mut block_adapter: WebHistoryAdapter<DocsRoute, _> =
        WebHistoryAdapter::new("/app", MemoryHistoryBackend::new("/app/"));
    let mut block_router = RouterState::new(DocsRoute::Home);
    block_router.add_guard_fn(|ctx| {
        if *ctx.to == DocsRoute::Admin { GuardDecision::Block } else { GuardDecision::Allow }
    });
    block_adapter.backend_mut().navigate_browser("/app/admin");
    assert_eq!(block_adapter.handle_popstate(&mut block_router), None);
    assert_eq!(block_router.current_route(), &DocsRoute::Home);
    assert_eq!(block_adapter.backend().current_href(), "/app/");
    assert_eq!((block_adapter.backend().push_count(), block_adapter.backend().replace_count()), (0, 1));
}

#[test]
fn should_intercept_link_click_exhaustive_matrix() {
    let backend = MemoryHistoryBackend::new("/native-view-engine/docs/intro?version=1")
        .with_origin("https://docs.example.com");
    let adapter: WebHistoryAdapter<DocsRoute, _> =
        WebHistoryAdapter::new("/native-view-engine/", backend);
    let valid = "/native-view-engine/docs/router";
    let plain_click = |href: &str| {
        adapter.should_intercept_link_click(href, false, false, false, false, 0, None)
    };

    // Modifier keys ignored.
    for (c, m, s, a) in [
        (true, false, false, false),
        (false, true, false, false),
        (false, false, true, false),
        (false, false, false, true),
    ] {
        assert_eq!(adapter.should_intercept_link_click(valid, c, m, s, a, 0, None), None);
    }

    // Non-primary mouse buttons ignored (middle=1, right=2, aux=3, negative).
    for button in [-1, 1, 2, 3, 4] {
        assert_eq!(
            adapter.should_intercept_link_click(valid, false, false, false, false, button, None),
            None
        );
    }

    // Target attribute handling.
    for allowed_target in [Some(""), Some("_self"), Some("_SELF")] {
        assert_eq!(
            adapter.should_intercept_link_click(valid, false, false, false, false, 0, allowed_target),
            Some("/docs/router".into())
        );
    }
    for blocked_target in ["_blank", "_parent", "_top", "docs-frame"] {
        assert_eq!(
            adapter.should_intercept_link_click(valid, false, false, false, false, 0, Some(blocked_target)),
            None
        );
    }

    // Non-interceptable hrefs (hash-only, same-page hash, schemes, foreign origins, outside base).
    for ignored in [
        "",
        "   ",
        "#",
        "#section-heading",
        "/native-view-engine/docs/intro?version=1#same-page",
        "mailto:team@example.com",
        "MAILTO:team@example.com",
        "tel:+1-555-0100",
        "javascript:void(0)",
        "data:text/html,hi",
        "blob:https://docs.example.com/1234",
        "ftp://docs.example.com/native-view-engine/docs/router",
        "https://external.example.org/native-view-engine/docs/router",
        "http://docs.example.com/native-view-engine/docs/router",
        "https://docs.example.com:8443/native-view-engine/docs/router",
        "//external.example.org/native-view-engine/docs/router",
        "/other-app/docs/router",
        "/native-view-engine-v2/docs/router",
        "/",
        "../../outside",
        "/native-view-engine/docs/\nnewline",
        "/native-view-engine/docs//double-slash",
    ] {
        assert_eq!(plain_click(ignored), None, "expected None for {ignored:?}");
    }

    // Valid internal links: root, nested, trailing slash, query-only, relative, same-origin.
    for (href, expected) in [
        ("/native-view-engine", "/"),
        ("/native-view-engine/", "/"),
        ("/native-view-engine/docs/router", "/docs/router"),
        ("/native-view-engine/docs/router/", "/docs/router"),
        ("/native-view-engine/docs/router?version=2#api", "/docs/router?version=2#api"),
        ("?version=2", "/docs/intro?version=2"),
        ("?version=2#changelog", "/docs/intro?version=2#changelog"),
        ("router", "/docs/router"),
        ("./router?version=3", "/docs/router?version=3"),
        ("../guide/setup#deps", "/guide/setup#deps"),
        ("https://docs.example.com/native-view-engine/docs/router?version=2", "/docs/router?version=2"),
        ("HTTPS://DOCS.EXAMPLE.COM:443/native-view-engine/docs/router", "/docs/router"),
        ("//docs.example.com/native-view-engine/guide/perf", "/guide/perf"),
    ] {
        assert_eq!(plain_click(href), Some(expected.to_owned()), "href={href:?}");
    }

    // Without a configured backend origin, absolute & protocol-relative URLs are ignored.
    let no_origin: WebHistoryAdapter<DocsRoute, _> =
        WebHistoryAdapter::new("", MemoryHistoryBackend::new("/docs/intro"));
    assert_eq!(
        no_origin.should_intercept_link_click(
            "https://docs.example.com/docs/router", false, false, false, false, 0, None
        ),
        None
    );
    assert_eq!(
        no_origin.should_intercept_link_click(
            "//docs.example.com/docs/router", false, false, false, false, 0, None
        ),
        None
    );
    assert_eq!(
        no_origin.should_intercept_link_click("/docs/router", false, false, false, false, 0, None),
        Some("/docs/router".into())
    );
}
