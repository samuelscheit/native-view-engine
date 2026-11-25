//! Unit tests for `define_routes!`, `QueryParams`, `RouterState`, `TabRouterState`,
//! `NavigationGuard`, and `DeepLinkConfig`.

use nve_router::prelude::*;
use nve_router::{percent_decode, percent_encode, PercentDecodeError};

define_routes! {
    pub enum AppRoute {
        #[route("/", title = "Home")]
        Home,
        #[route("/settings/profile", title = "Profile Settings")]
        Profile,
        #[route("/login", title = "Sign In")]
        Login,
        #[route("/admin", title = "Admin")]
        Admin,
        #[route("/issues/new", title = "New Issue")]
        NewIssue,
        #[route("/issues/:id", title = "Issue Detail")]
        Issue {
            id: u64,
            #[query]
            tab: Option<String>,
            #[query("p")]
            page: Option<u32>,
            #[fragment]
            section: Option<String>,
        },
        #[route("/docs/*slug", title = "Documentation")]
        Docs { slug: String },
        #[route("/search", title = "Search")]
        Search {
            #[query]
            q: String,
            #[query]
            limit: Option<usize>,
        },
        #[route("/modal/confirm/:action", title = "Confirm")]
        ConfirmModal { action: String },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AppTab {
    Home,
    Issues,
    Docs,
}

#[test]
fn define_routes_static_typed_wildcard_query_fragment_roundtrip() {
    assert_eq!(AppRoute::parse_uri("/"), Some(AppRoute::Home));
    assert_eq!(AppRoute::Home.to_path(), "/");
    assert_eq!(AppRoute::Home.title(), Some("Home"));
    assert_eq!(AppRoute::parse_uri("/settings/profile/"), Some(AppRoute::Profile));
    assert_eq!(AppRoute::Profile.to_path(), "/settings/profile");
    assert_eq!(AppRoute::parse_uri("/issues/new"), Some(AppRoute::NewIssue));

    let issue_uri = "/issues/42?tab=comments%20%26%20notes&p=3#top%20anchor";
    let parsed = AppRoute::parse_uri(issue_uri).expect("valid issue uri");
    assert_eq!(
        parsed,
        AppRoute::Issue {
            id: 42,
            tab: Some("comments & notes".into()),
            page: Some(3),
            section: Some("top anchor".into()),
        }
    );
    let encoded = parsed.to_path();
    assert_eq!(encoded, "/issues/42?tab=comments%20%26%20notes&p=3#top%20anchor");
    assert_eq!(AppRoute::parse_uri(&encoded), Some(parsed));

    let docs = AppRoute::parse_uri("/docs/guides/getting%20started/v2").expect("docs wildcard");
    assert_eq!(
        docs,
        AppRoute::Docs {
            slug: "guides/getting started/v2".into(),
        }
    );
    assert_eq!(docs.to_path(), "/docs/guides/getting%20started/v2");
    assert_eq!(AppRoute::parse_uri(&docs.to_path()), Some(docs));

    let search = AppRoute::parse_uri("/search?q=hello%20world&limit=25").expect("search route");
    assert_eq!(
        search,
        AppRoute::Search {
            q: "hello world".into(),
            limit: Some(25),
        }
    );
    assert_eq!(search.to_path(), "/search?q=hello%20world&limit=25");

    // Reject invalid segments, bad typed params, missing required query, bad percent-escapes.
    for invalid in [
        "",
        "issues/42",
        "//issues/42",
        "/issues//42",
        "/issues/not-a-number",
        "/issues/42?p=bad-int",
        "/issues/42?tab=%ZZ",
        "/docs",
        "/docs/good/%2",
        "/search",
        "/search?q=ok&limit=nope",
    ] {
        assert_eq!(AppRoute::parse_uri(invalid), None, "expected None for {invalid:?}");
    }
}

#[test]
fn query_params_zero_alloc_fast_paths_and_strict_decoding() {
    assert!(matches!(
        percent_encode("alpha-09._~"),
        std::borrow::Cow::Borrowed("alpha-09._~")
    ));
    assert_eq!(percent_encode("a b/c?d#e"), "a%20b%2Fc%3Fd%23e");
    assert!(matches!(
        percent_decode("plain_value").unwrap(),
        std::borrow::Cow::Borrowed("plain_value")
    ));
    assert_eq!(percent_decode("caf%C3%A9%20100%25").unwrap(), "café 100%");
    assert_eq!(percent_decode("bad%2"), Err(PercentDecodeError::TruncatedEscape(3)));
    assert_eq!(percent_decode("bad%2G"), Err(PercentDecodeError::InvalidHex(5)));
    assert_eq!(percent_decode("%FF%FE"), Err(PercentDecodeError::InvalidUtf8));

    let (path, qp) = QueryParams::from_uri("/x?tag=a&tag=b&count=7#sec%201").unwrap();
    assert_eq!(path, "/x");
    assert_eq!(qp.get("tag"), Some("a"));
    assert_eq!(qp.get_all("tag").collect::<Vec<_>>(), vec!["a", "b"]);
    assert_eq!(qp.get_parse::<u32>("count"), Some(7));
    assert_eq!(qp.fragment(), Some("sec 1"));
    assert_eq!(qp.append_to_path("/x"), "/x?tag=a&tag=b&count=7#sec%201");
}

#[test]
fn router_stack_history_forward_truncation_and_modal_isolation() {
    let mut r = RouterState::new(AppRoute::Home);
    assert_eq!(r.current().entry_id, 1);
    assert_eq!(r.depth(), 1);
    assert!(!r.can_go_back());
    assert!(!r.can_go_forward());
    assert_eq!(r.pop(), Err(NavigationError::CannotPopRoot));

    let t1 = r.push(AppRoute::Profile).unwrap();
    assert_eq!(t1.entry_id, 2);
    assert_eq!(t1.stack_depth, 2);
    assert!(r.can_go_back());
    assert!(!r.can_go_forward());

    let t2 = r.push(AppRoute::Docs { slug: "intro".into() }).unwrap();
    assert_eq!(t2.entry_id, 3);
    assert_eq!(r.depth(), 3);

    let back = r.go(-1).unwrap();
    assert_eq!(back.route, AppRoute::Profile);
    assert_eq!(back.entry_id, 2);
    assert!(r.can_go_back() && r.can_go_forward());
    assert_eq!(
        r.go(2),
        Err(NavigationError::HistoryOutOfBounds { cursor: 1, len: 3, delta: 2 })
    );
    assert_eq!(
        r.go(-2),
        Err(NavigationError::HistoryOutOfBounds { cursor: 1, len: 3, delta: -2 })
    );
    assert_eq!(r.go(1).unwrap().entry_id, 3);
    assert!(!r.can_go_forward());

    // go(-1) then push truncates forward history.
    r.go(-1).unwrap();
    let t3 = r.push(AppRoute::NewIssue).unwrap();
    assert_eq!(t3.entry_id, 4);
    assert!(!r.can_go_forward());
    assert_eq!(r.history_len(), 3);

    // replace keeps depth, assigns new monotonic id.
    let rep = r.replace(AppRoute::Login).unwrap();
    assert_eq!(rep.entry_id, 5);
    assert_eq!(r.depth(), 3);
    assert_eq!(r.current_route(), &AppRoute::Login);

    // Modal overlay isolation.
    let m1 = r.open_modal(AppRoute::ConfirmModal { action: "delete".into() }).unwrap();
    assert!(m1.is_modal);
    assert_eq!(m1.entry_id, 6);
    assert_eq!(r.current_route(), &AppRoute::Login);
    assert_eq!(r.depth(), 3);

    let m2 = r.open_modal(AppRoute::ConfirmModal { action: "confirm2".into() }).unwrap();
    assert_eq!(m2.entry_id, 7);
    assert_eq!(r.modals().len(), 2);

    let dm1 = r.dismiss_modal().unwrap();
    assert!(dm1.is_modal);
    assert_eq!(dm1.route, AppRoute::ConfirmModal { action: "delete".into() });
    assert_eq!(r.dismiss_all_modals(), 1);
    assert!(!r.has_modal());
    assert_eq!(r.dismiss_modal(), Err(NavigationError::NoActiveModal));
    assert_eq!(r.current_route(), &AppRoute::Login);
    assert_eq!(r.depth(), 3);

    r.pop().unwrap();
    assert_eq!(r.current_route(), &AppRoute::Profile);
    assert!(!r.can_go_forward());
    r.pop_to_root().unwrap();
    assert_eq!(r.current_route(), &AppRoute::Home);
    assert_eq!(r.depth(), 1);
}

#[test]
fn tab_router_preserves_independent_stacks_retap_pops_to_root_and_shares_modals() {
    let mut tabs = TabRouterState::with_specs(
        vec![
            TabSpec::new(AppTab::Home, "home", AppRoute::Home),
            TabSpec::new(AppTab::Issues, "issues", AppRoute::NewIssue),
            TabSpec::new(AppTab::Docs, "docs", AppRoute::Docs { slug: "start".into() }),
        ],
        AppTab::Home,
    )
    .unwrap();

    tabs.push(AppRoute::Profile).unwrap();
    tabs.push(AppRoute::Login).unwrap();
    assert_eq!(tabs.depth(), 3);

    let sw = tabs.select_tab(AppTab::Issues).unwrap();
    assert_eq!(sw.action, NavigationAction::SwitchTab);
    assert_eq!(sw.active_tab_index, 1);
    assert_eq!(sw.active_tab_id, "issues");
    assert_eq!(tabs.current_route(), &AppRoute::NewIssue);

    tabs.push(AppRoute::Issue {
        id: 99,
        tab: None,
        page: None,
        section: None,
    })
    .unwrap();
    assert_eq!(tabs.depth(), 2);

    // Returning to Home preserves its depth=3 stack.
    tabs.select_tab(AppTab::Home).unwrap();
    assert_eq!(tabs.depth(), 3);
    assert_eq!(tabs.current_route(), &AppRoute::Login);

    // Re-tapping active tab pops Home to root without touching Issues stack.
    let retap = tabs.select_tab(AppTab::Home).unwrap();
    assert_eq!(retap.action, NavigationAction::PopToRoot);
    assert_eq!(tabs.current_route(), &AppRoute::Home);
    assert_eq!(tabs.depth(), 1);
    assert_eq!(tabs.tab_stack(AppTab::Issues).unwrap().depth(), 2);

    // Shared modal survives tab switch without mutating stacks.
    tabs.open_modal(AppRoute::ConfirmModal { action: "logout".into() }).unwrap();
    tabs.select_tab(AppTab::Issues).unwrap();
    assert!(tabs.has_modal());
    assert_eq!(tabs.depth(), 2);
    tabs.dismiss_modal().unwrap();
    assert!(!tabs.has_modal());
    assert_eq!(tabs.depth(), 2);
}

#[test]
fn navigation_guards_allow_redirect_chain_cycle_cap_and_block() {
    let mut r = RouterState::new(AppRoute::Home).with_max_redirect_hops(4);
    r.add_guard_fn(|ctx| match ctx.to {
        AppRoute::Admin => GuardDecision::Block,
        AppRoute::Profile => GuardDecision::Redirect(AppRoute::Login),
        AppRoute::Login => GuardDecision::Redirect(AppRoute::Docs {
            slug: "welcome".into(),
        }),
        _ => GuardDecision::Allow,
    });

    assert_eq!(r.push(AppRoute::Admin), Err(NavigationError::Blocked));
    assert_eq!(r.current_route(), &AppRoute::Home);
    assert_eq!(r.depth(), 1);

    let ok = r.push(AppRoute::NewIssue).unwrap();
    assert_eq!(ok.redirect_hops, 0);

    let redir = r.push(AppRoute::Profile).unwrap();
    assert_eq!(redir.redirect_hops, 2);
    assert_eq!(redir.route, AppRoute::Docs { slug: "welcome".into() });
    assert_eq!(r.depth(), 3);

    let mut cyclic = RouterState::new(AppRoute::Home);
    cyclic.add_guard_fn(|ctx| match ctx.to {
        AppRoute::Admin => GuardDecision::Redirect(AppRoute::Login),
        AppRoute::Login => GuardDecision::Redirect(AppRoute::Admin),
        _ => GuardDecision::Allow,
    });
    assert!(matches!(
        cyclic.push(AppRoute::Admin),
        Err(NavigationError::RedirectCycle { hops: 2 })
    ));
    assert_eq!(cyclic.depth(), 1);

    let mut capped = RouterState::new(AppRoute::Home).with_max_redirect_hops(3);
    capped.add_guard_fn(|ctx| match ctx.to {
        AppRoute::Issue { id, .. } => GuardDecision::Redirect(AppRoute::Issue {
            id: id + 1,
            tab: None,
            page: None,
            section: None,
        }),
        _ => GuardDecision::Allow,
    });
    assert!(matches!(
        capped.push(AppRoute::Issue {
            id: 1,
            tab: None,
            page: None,
            section: None,
        }),
        Err(NavigationError::RedirectCycle { hops: 4 })
    ));
    assert_eq!(capped.depth(), 1);
}

#[test]
fn deeplink_custom_scheme_universal_link_prefix_strip_and_rejections() {
    let cfg = DeepLinkConfig::new()
        .with_scheme("nve")
        .with_universal_host("app.example.com")
        .with_base_prefix("/app");

    let m1 = cfg
        .match_uri::<AppRoute>("nve://app/issues/42?tab=comments#top")
        .unwrap();
    assert_eq!(m1.origin, DeepLinkOrigin::CustomScheme("nve".into()));
    assert_eq!(m1.normalized_path, "/issues/42");
    assert_eq!(
        m1.route,
        AppRoute::Issue {
            id: 42,
            tab: Some("comments".into()),
            page: None,
            section: Some("top".into()),
        }
    );

    let m2 = cfg
        .match_uri::<AppRoute>("HTTPS://APP.EXAMPLE.COM/issues/42/?p=2")
        .unwrap();
    assert_eq!(
        m2.origin,
        DeepLinkOrigin::UniversalLink {
            scheme: "https".into(),
            host: "app.example.com".into(),
        }
    );
    assert_eq!(
        m2.route,
        AppRoute::Issue {
            id: 42,
            tab: None,
            page: Some(2),
            section: None,
        }
    );

    let m3 = cfg.match_uri::<AppRoute>("/app/settings/profile/").unwrap();
    assert_eq!(m3.origin, DeepLinkOrigin::WebPath);
    assert_eq!(m3.route, AppRoute::Profile);

    assert!(matches!(
        cfg.match_uri::<AppRoute>("/application/settings/profile"),
        Err(DeepLinkError::UnmatchedRoute(p)) if p == "/application/settings/profile"
    ));

    let strict_cfg = cfg.clone().require_base_prefix(true);
    assert_eq!(
        strict_cfg.match_uri::<AppRoute>("/settings/profile"),
        Err(DeepLinkError::PrefixMismatch)
    );

    assert_eq!(cfg.match_uri::<AppRoute>(""), Err(DeepLinkError::EmptyUri));
    assert!(matches!(
        cfg.match_uri::<AppRoute>("https://app.example.com/bad path"),
        Err(DeepLinkError::InvalidUri(_))
    ));
    assert!(matches!(
        cfg.match_uri::<AppRoute>("javascript:alert(1)"),
        Err(DeepLinkError::UnsupportedScheme(s)) if s == "javascript"
    ));
    assert!(matches!(
        cfg.match_uri::<AppRoute>("http://app.example.com/issues/1"),
        Err(DeepLinkError::UnsupportedScheme(s)) if s == "http"
    ));
    assert!(matches!(
        cfg.match_uri::<AppRoute>("https://evil.example.com/issues/1"),
        Err(DeepLinkError::UnsupportedHost(h)) if h == "evil.example.com"
    ));
    assert!(matches!(
        cfg.match_uri::<AppRoute>("https://evil@app.example.com/issues/1"),
        Err(DeepLinkError::InvalidUri(_))
    ));
    assert!(matches!(
        cfg.match_uri::<AppRoute>("nve://app/issues/42?tab=%ZZ"),
        Err(DeepLinkError::InvalidPercentEncoding(_))
    ));
}
