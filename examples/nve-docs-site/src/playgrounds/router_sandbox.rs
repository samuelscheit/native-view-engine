//! Interactive `"router-sandbox"` demo powered by real `nve-router` stack, tabs, modal,
//! and guard transitions with live route path and history depth display.

use nve_dioxus::prelude::*;
use nve_router::{GuardDecision, QueryParams, Routable, RouterState, guard_fn};
use nve_ui::{Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Switch, use_theme};

pub const ROUTER_SANDBOX_SOURCE: &str = r#"use nve_router::{GuardDecision, Routable, RouterState, guard_fn};

#[derive(Debug, Clone, PartialEq, Eq)]
enum AppRoute { Home, Feed, Item(u32), Admin, Login, QuickModal }

let mut router = RouterState::new(AppRoute::Home);
router.add_guard(guard_fn(|ctx| {
    if ctx.to == AppRoute::Admin && !is_authed {
        GuardDecision::Redirect(AppRoute::Login)
    } else {
        GuardDecision::Allow
    }
}));
router.push(AppRoute::Item(42))?;
router.open_modal(AppRoute::QuickModal)?;"#;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SandboxRoute {
    Home,
    Feed,
    Item(u32),
    Admin,
    Settings,
    Login,
    ComposeModal,
}

impl Routable for SandboxRoute {
    fn parse_path(path: &str, _query: &QueryParams) -> Option<Self> {
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return Some(Self::Home);
        }
        match trimmed {
            "feed" => Some(Self::Feed),
            "admin" => Some(Self::Admin),
            "settings" => Some(Self::Settings),
            "login" => Some(Self::Login),
            "compose" => Some(Self::ComposeModal),
            other => other
                .strip_prefix("items/")
                .and_then(|id| id.parse::<u32>().ok())
                .map(Self::Item),
        }
    }

    fn to_path(&self) -> String {
        match self {
            Self::Home => "/".to_string(),
            Self::Feed => "/feed".to_string(),
            Self::Item(id) => format!("/items/{id}"),
            Self::Admin => "/admin".to_string(),
            Self::Settings => "/settings".to_string(),
            Self::Login => "/login".to_string(),
            Self::ComposeModal => "/compose".to_string(),
        }
    }

    fn title(&self) -> Option<&'static str> {
        Some(match self {
            Self::Home => "Home",
            Self::Feed => "Activity Feed",
            Self::Item(_) => "Item Detail",
            Self::Admin => "Admin Console",
            Self::Settings => "Settings",
            Self::Login => "Sign In (Guard Redirect)",
            Self::ComposeModal => "Quick Compose Modal",
        })
    }
}

fn build_router_with_guard(guard_enabled: bool) -> RouterState<SandboxRoute> {
    let mut router = RouterState::new(SandboxRoute::Home);
    if guard_enabled {
        router.add_guard(guard_fn(|ctx| {
            if *ctx.to == SandboxRoute::Admin {
                GuardDecision::Redirect(SandboxRoute::Login)
            } else {
                GuardDecision::Allow
            }
        }));
    }
    router
}

#[component]
pub fn RouterSandboxDemo() -> Element {
    let theme = use_theme();
    let mut guard_enabled = use_signal(|| true);
    let mut router = use_signal(|| build_router_with_guard(true));
    let mut next_item_id = use_signal(|| 42u32);
    let mut last_event = use_signal(|| "Initialized stack at /".to_string());

    let snap = router.read();
    let current_path = snap.current_route().to_path();
    let current_title = snap.current_route().title().unwrap_or("Route");
    let depth = snap.depth();
    let can_back = snap.can_go_back();
    let active_modal = snap.active_modal().map(|m| m.route.to_path());
    let has_modal = active_modal.is_some();
    let stack_trail: Vec<String> = snap.entries().iter().map(|e| e.route.to_path()).collect();
    drop(snap);

    rsx! {
        view {
            test_id: "pg-router-root",
            style: "width: 100%; flex-direction: column; gap: 12px;",

            // Live URL bar + stack depth telemetry
            view {
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px; padding: 10px; border-radius: 8px; background-color: {theme.palette.surface_elevated}; border-width: 1px; border-color: {theme.palette.border};",
                view {
                    style: "flex-direction: row; align-items: center; gap: 8px;",
                    Badge {
                        label: "ROUTE".to_string(),
                        variant: BadgeVariant::Accent,
                    }
                    text {
                        test_id: "pg-router-path",
                        style: "font-size: 14px; font-weight: 700; color: {theme.palette.text_primary};",
                        "{current_path}"
                    }
                    text {
                        style: "font-size: 12px; color: {theme.palette.text_secondary};",
                        "({current_title})"
                    }
                }
                view {
                    style: "flex-direction: row; align-items: center; gap: 6px;",
                    Badge {
                        test_id: Some("pg-router-depth".to_string()),
                        label: format!("Depth: {depth}"),
                        variant: BadgeVariant::Neutral,
                    }
                    if let Some(ref modal_path) = active_modal {
                        Badge {
                            test_id: Some("pg-router-modal-badge".to_string()),
                            label: format!("Modal: {modal_path}"),
                            variant: BadgeVariant::Warning,
                        }
                    }
                }
            }

            // Tab switches + Stack actions + Guard toggle
            view {
                style: "width: 100%; flex-direction: row; flex-wrap: wrap; gap: 8px; align-items: center;",
                Button {
                    test_id: Some("pg-router-tab-home".to_string()),
                    variant: if current_path == "/" { ButtonVariant::Primary } else { ButtonVariant::Secondary },
                    size: ButtonSize::Sm,
                    on_press: move |_| {
                        let _ = router.write().replace(SandboxRoute::Home);
                        last_event.set("Tab switched -> /".to_string());
                    },
                    "Tab: Home"
                }
                Button {
                    test_id: Some("pg-router-tab-feed".to_string()),
                    variant: if current_path == "/feed" { ButtonVariant::Primary } else { ButtonVariant::Secondary },
                    size: ButtonSize::Sm,
                    on_press: move |_| {
                        let _ = router.write().push(SandboxRoute::Feed);
                        last_event.set("Pushed /feed".to_string());
                    },
                    "Tab: Feed"
                }
                Button {
                    test_id: Some("pg-router-push-item".to_string()),
                    variant: ButtonVariant::Outline,
                    size: ButtonSize::Sm,
                    on_press: move |_| {
                        let id = next_item_id();
                        next_item_id.set(id + 1);
                        let _ = router.write().push(SandboxRoute::Item(id));
                        last_event.set(format!("Pushed /items/{id}"));
                    },
                    "Push /items/{next_item_id}"
                }
                Button {
                    test_id: Some("pg-router-push-admin".to_string()),
                    variant: ButtonVariant::Outline,
                    size: ButtonSize::Sm,
                    on_press: move |_| {
                        match router.write().push(SandboxRoute::Admin) {
                            Ok(tr) if tr.route == SandboxRoute::Login => {
                                last_event.set("Guard redirected /admin -> /login".to_string());
                            }
                            Ok(tr) => {
                                last_event.set(format!("Allowed -> {}", tr.route.to_path()));
                            }
                            Err(err) => {
                                last_event.set(format!("Blocked: {err:?}"));
                            }
                        }
                    },
                    "Push /admin"
                }
                Button {
                    test_id: Some("pg-router-pop".to_string()),
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Sm,
                    disabled: !can_back,
                    on_press: move |_| {
                        if let Ok(tr) = router.write().pop() {
                            last_event.set(format!("Popped -> {}", tr.route.to_path()));
                        }
                    },
                    "Pop"
                }
                Button {
                    test_id: Some("pg-router-modal".to_string()),
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Sm,
                    on_press: move |_| {
                        if has_modal {
                            let _ = router.write().dismiss_modal();
                            last_event.set("Dismissed modal overlay".to_string());
                        } else {
                            let _ = router.write().open_modal(SandboxRoute::ComposeModal);
                            last_event.set("Opened modal /compose".to_string());
                        }
                    },
                    if has_modal { "Dismiss Modal" } else { "Open Modal" }
                }
            }

            // Auth guard toggle + stack history readout
            view {
                style: "width: 100%; flex-direction: row; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 10px;",
                Switch {
                    test_id: Some("pg-router-guard-toggle".to_string()),
                    checked: guard_enabled(),
                    label: Some("Auth Guard (redirect /admin -> /login)".to_string()),
                    on_change: move |enabled| {
                        guard_enabled.set(enabled);
                        let cur = router.read().current_route().clone();
                        let mut rebuilt = build_router_with_guard(enabled);
                        if cur != SandboxRoute::Home {
                            let _ = rebuilt.replace(cur);
                        }
                        router.set(rebuilt);
                        last_event.set(if enabled {
                            "Auth guard enabled".to_string()
                        } else {
                            "Auth guard disabled — /admin unlocked".to_string()
                        });
                    },
                }
                text {
                    test_id: "pg-router-status",
                    style: "font-size: 12px; color: {theme.palette.text_secondary};",
                    "{last_event}"
                }
            }

            // History stack visualization
            view {
                style: "width: 100%; flex-direction: row; flex-wrap: wrap; gap: 6px; align-items: center;",
                text {
                    style: "font-size: 11px; font-weight: 600; color: {theme.palette.text_muted};",
                    "Stack:"
                }
                for (i, seg) in stack_trail.into_iter().enumerate() {
                    Badge {
                        key: "stk-{i}",
                        label: seg,
                        variant: BadgeVariant::Neutral,
                    }
                }
            }
        }
    }
}
