//! Zero-proc-macro declarative [`define_routes!`] macro implementation.

/// Zero-proc-macro declarative route definition macro generating a typed `enum` and its
/// [`crate::Routable`] implementation.
///
/// Supports:
/// - Static segments: `#[route("/settings/profile", title = "Profile")]`
/// - Typed path parameters via `FromStr`: `#[route("/issues/:id")] Issue { id: u64 }`
/// - Catch-all wildcards: `#[route("/docs/*slug")] Docs { slug: String }`
/// - Required and optional query parameters (`#[query]` / `#[query("custom_key")]`)
/// - Optional `#fragment` capture (`#[fragment] hash: Option<String>`)
///
/// # Example
/// ```rust
/// use nve_router::{define_routes, QueryParams, Routable};
///
/// define_routes! {
///     pub enum AppRoute {
///         #[route("/", title = "Home")]
///         Home,
///         #[route("/issues/:id", title = "Issue")]
///         Issue {
///             id: u64,
///             #[query]
///             tab: Option<String>,
///         },
///         #[route("/docs/*slug", title = "Docs")]
///         Docs { slug: String },
///     }
/// }
///
/// let parsed = AppRoute::parse_uri("/issues/42?tab=comments").unwrap();
/// assert_eq!(parsed, AppRoute::Issue { id: 42, tab: Some("comments".into()) });
/// assert_eq!(parsed.to_path(), "/issues/42?tab=comments");
/// ```
#[macro_export]
macro_rules! define_routes {
    (
        $(#[$enum_meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[doc = $var_doc:expr])*
                #[route($pat:literal $(, title = $title:literal)?)]
                $(#[$var_meta:meta])*
                $variant:ident $({ $($fields:tt)* })?
            ),* $(,)?
        }
    ) => {
        $crate::__nve_define_routes_emit_enum! {
            meta [ $(#[$enum_meta])* ]
            vis [ $vis ]
            name [ $name ]
            variants []
            rest [
                $(
                    variant {
                        meta [ $(#[doc = $var_doc])* $(#[$var_meta])* ]
                        ident [ $variant ]
                        fields [ $($($fields)*)? ]
                    }
                )*
            ]
        }

        impl $crate::Routable for $name {
            fn parse_path(path: &str, query: &$crate::QueryParams) -> ::core::option::Option<Self> {
                $(
                    if let ::core::option::Option::Some(matched) =
                        $crate::route::__private::match_route_pattern($pat, path)
                    {
                        if let ::core::option::Option::Some(route) =
                            $crate::__nve_define_routes_parse_variant!(
                                $name,
                                $variant,
                                matched,
                                query,
                                [ $($($fields)*)? ]
                            )
                        {
                            return ::core::option::Option::Some(route);
                        }
                    }
                )*
                ::core::option::Option::None
            }

            fn to_path(&self) -> ::std::string::String {
                $crate::__nve_define_routes_emit_to_path! {
                    self_val = self,
                    name = $name,
                    arms [],
                    rest [
                        $(
                            arm {
                                variant [ $variant ]
                                pat [ $pat ]
                                fields [ $($($fields)*)? ]
                            }
                        )*
                    ]
                }
            }

            fn title(&self) -> ::core::option::Option<&'static str> {
                $crate::__nve_define_routes_emit_title! {
                    self_val = self,
                    name = $name,
                    arms [],
                    rest [
                        $(
                            arm {
                                variant [ $variant ]
                                title [ $($title)? ]
                                fields [ $($($fields)*)? ]
                            }
                        )*
                    ]
                }
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __nve_define_routes_emit_enum {
    (
        meta [ $(#[$enum_meta:meta])* ]
        vis [ $vis:vis ]
        name [ $name:ident ]
        variants [ $($done:tt)* ]
        rest []
    ) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        $(#[$enum_meta])*
        $vis enum $name {
            $($done)*
        }
    };
    (
        meta [ $($enum_meta:tt)* ]
        vis [ $vis:vis ]
        name [ $name:ident ]
        variants [ $($done:tt)* ]
        rest [
            variant { meta [ $(#[$var_meta:meta])* ] ident [ $variant:ident ] fields [] }
            $($tail:tt)*
        ]
    ) => {
        $crate::__nve_define_routes_emit_enum! {
            meta [ $($enum_meta)* ]
            vis [ $vis ]
            name [ $name ]
            variants [ $($done)* $(#[$var_meta])* $variant, ]
            rest [ $($tail)* ]
        }
    };
    (
        meta [ $($enum_meta:tt)* ]
        vis [ $vis:vis ]
        name [ $name:ident ]
        variants [ $($done:tt)* ]
        rest [
            variant { meta [ $(#[$var_meta:meta])* ] ident [ $variant:ident ] fields [ $($raw_fields:tt)+ ] }
            $($tail:tt)*
        ]
    ) => {
        $crate::__nve_define_routes_strip_fields! {
            acc []
            rest [ $($raw_fields)+ ]
            cont {
                meta [ $($enum_meta)* ]
                vis [ $vis ]
                name [ $name ]
                variants [ $($done)* ]
                cur_meta [ $(#[$var_meta])* ]
                cur_ident [ $variant ]
                tail [ $($tail)* ]
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __nve_define_routes_strip_fields {
    (
        acc [ $($acc:tt)* ]
        rest []
        cont {
            meta [ $($enum_meta:tt)* ]
            vis [ $vis:vis ]
            name [ $name:ident ]
            variants [ $($done:tt)* ]
            cur_meta [ $(#[$var_meta:meta])* ]
            cur_ident [ $variant:ident ]
            tail [ $($tail:tt)* ]
        }
    ) => {
        $crate::__nve_define_routes_emit_enum! {
            meta [ $($enum_meta)* ]
            vis [ $vis ]
            name [ $name ]
            variants [ $($done)* $(#[$var_meta])* $variant { $($acc)* }, ]
            rest [ $($tail)* ]
        }
    };
    (acc [ $($acc:tt)* ] rest [ #[query($qkey:literal)] $f:ident : $ty:ty $(, $($rest:tt)*)? ] cont $cont:tt) => {
        $crate::__nve_define_routes_strip_fields! { acc [ $($acc)* $f : $ty, ] rest [ $($($rest)*)? ] cont $cont }
    };
    (acc [ $($acc:tt)* ] rest [ #[query] $f:ident : $ty:ty $(, $($rest:tt)*)? ] cont $cont:tt) => {
        $crate::__nve_define_routes_strip_fields! { acc [ $($acc)* $f : $ty, ] rest [ $($($rest)*)? ] cont $cont }
    };
    (acc [ $($acc:tt)* ] rest [ #[fragment] $f:ident : $ty:ty $(, $($rest:tt)*)? ] cont $cont:tt) => {
        $crate::__nve_define_routes_strip_fields! { acc [ $($acc)* $f : $ty, ] rest [ $($($rest)*)? ] cont $cont }
    };
    (acc [ $($acc:tt)* ] rest [ $f:ident : $ty:ty $(, $($rest:tt)*)? ] cont $cont:tt) => {
        $crate::__nve_define_routes_strip_fields! { acc [ $($acc)* $f : $ty, ] rest [ $($($rest)*)? ] cont $cont }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __nve_define_routes_parse_variant {
    ($name:ident, $variant:ident, $matched:ident, $query:ident, []) => {{
        let _ = (&$matched, &$query);
        ::core::option::Option::Some($name::$variant)
    }};
    ($name:ident, $variant:ident, $matched:ident, $query:ident, [ $($fields:tt)+ ]) => {{
        (|| -> ::core::option::Option<$name> {
            let _ = (&$matched, &$query);
            $crate::__nve_define_routes_parse_fields!(
                matched = $matched,
                query = $query,
                name = $name,
                variant = $variant,
                bound [],
                rest [ $($fields)+ ]
            )
        })()
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __nve_define_routes_parse_fields {
    (
        matched = $matched:ident, query = $query:ident, name = $name:ident, variant = $variant:ident,
        bound [ $($f:ident),* ], rest []
    ) => {
        ::core::option::Option::Some($name::$variant { $($f),* })
    };
    (
        matched = $matched:ident, query = $query:ident, name = $name:ident, variant = $variant:ident,
        bound [ $($b:ident),* ],
        rest [ #[query($qkey:literal)] $f:ident : Option<$inner:ty> $(, $($rest:tt)*)? ]
    ) => {{
        let $f: ::core::option::Option<$inner> = match $query.get_parse_result::<$inner>($qkey) {
            ::core::option::Option::None => ::core::option::Option::None,
            ::core::option::Option::Some(::core::result::Result::Ok(v)) => ::core::option::Option::Some(v),
            ::core::option::Option::Some(::core::result::Result::Err(_)) => return ::core::option::Option::None,
        };
        $crate::__nve_define_routes_parse_fields!(
            matched = $matched, query = $query, name = $name, variant = $variant,
            bound [ $($b,)* $f ], rest [ $($($rest)*)? ]
        )
    }};
    (
        matched = $matched:ident, query = $query:ident, name = $name:ident, variant = $variant:ident,
        bound [ $($b:ident),* ],
        rest [ #[query($qkey:literal)] $f:ident : $ty:ty $(, $($rest:tt)*)? ]
    ) => {{
        let $f: $ty = $query.get_parse::<$ty>($qkey)?;
        $crate::__nve_define_routes_parse_fields!(
            matched = $matched, query = $query, name = $name, variant = $variant,
            bound [ $($b,)* $f ], rest [ $($($rest)*)? ]
        )
    }};
    (
        matched = $matched:ident, query = $query:ident, name = $name:ident, variant = $variant:ident,
        bound [ $($b:ident),* ],
        rest [ #[query] $f:ident : Option<$inner:ty> $(, $($rest:tt)*)? ]
    ) => {{
        let $f: ::core::option::Option<$inner> =
            match $query.get_parse_result::<$inner>(::core::stringify!($f)) {
                ::core::option::Option::None => ::core::option::Option::None,
                ::core::option::Option::Some(::core::result::Result::Ok(v)) => ::core::option::Option::Some(v),
                ::core::option::Option::Some(::core::result::Result::Err(_)) => return ::core::option::Option::None,
            };
        $crate::__nve_define_routes_parse_fields!(
            matched = $matched, query = $query, name = $name, variant = $variant,
            bound [ $($b,)* $f ], rest [ $($($rest)*)? ]
        )
    }};
    (
        matched = $matched:ident, query = $query:ident, name = $name:ident, variant = $variant:ident,
        bound [ $($b:ident),* ],
        rest [ #[query] $f:ident : $ty:ty $(, $($rest:tt)*)? ]
    ) => {{
        let $f: $ty = $query.get_parse::<$ty>(::core::stringify!($f))?;
        $crate::__nve_define_routes_parse_fields!(
            matched = $matched, query = $query, name = $name, variant = $variant,
            bound [ $($b,)* $f ], rest [ $($($rest)*)? ]
        )
    }};
    (
        matched = $matched:ident, query = $query:ident, name = $name:ident, variant = $variant:ident,
        bound [ $($b:ident),* ],
        rest [ #[fragment] $f:ident : Option<$inner:ty> $(, $($rest:tt)*)? ]
    ) => {{
        let $f: ::core::option::Option<$inner> = match $query.fragment() {
            ::core::option::Option::None => ::core::option::Option::None,
            ::core::option::Option::Some(raw) => match raw.parse::<$inner>() {
                ::core::result::Result::Ok(v) => ::core::option::Option::Some(v),
                ::core::result::Result::Err(_) => return ::core::option::Option::None,
            },
        };
        $crate::__nve_define_routes_parse_fields!(
            matched = $matched, query = $query, name = $name, variant = $variant,
            bound [ $($b,)* $f ], rest [ $($($rest)*)? ]
        )
    }};
    (
        matched = $matched:ident, query = $query:ident, name = $name:ident, variant = $variant:ident,
        bound [ $($b:ident),* ],
        rest [ $f:ident : $ty:ty $(, $($rest:tt)*)? ]
    ) => {{
        let $f: $ty = $matched.get(::core::stringify!($f))?.parse::<$ty>().ok()?;
        $crate::__nve_define_routes_parse_fields!(
            matched = $matched, query = $query, name = $name, variant = $variant,
            bound [ $($b,)* $f ], rest [ $($($rest)*)? ]
        )
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __nve_define_routes_emit_to_path {
    (self_val = $self_val:ident, name = $name:ident, arms [ $($arms:tt)* ], rest []) => {
        match $self_val {
            $($arms)*
        }
    };
    (
        self_val = $self_val:ident, name = $name:ident, arms [ $($arms:tt)* ],
        rest [ arm { variant [ $variant:ident ] pat [ $pat:literal ] fields [] } $($tail:tt)* ]
    ) => {
        $crate::__nve_define_routes_emit_to_path! {
            self_val = $self_val,
            name = $name,
            arms [
                $($arms)*
                $name::$variant => $crate::route::__private::format_route_pattern(
                    $pat,
                    &[],
                    &$crate::QueryParams::new(),
                ),
            ],
            rest [ $($tail)* ]
        }
    };
    (
        self_val = $self_val:ident, name = $name:ident, arms [ $($arms:tt)* ],
        rest [ arm { variant [ $variant:ident ] pat [ $pat:literal ] fields [ $($fields:tt)+ ] } $($tail:tt)* ]
    ) => {
        $crate::__nve_define_routes_format_fields! {
            p = __params, q = __query,
            names [], stmts [], rest [ $($fields)+ ],
            cont {
                self_val = $self_val,
                name = $name,
                variant = $variant,
                pat = $pat,
                arms [ $($arms)* ],
                tail [ $($tail)* ]
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __nve_define_routes_format_fields {
    (
        p = $p:ident, q = $q:ident,
        names [ $($f:ident),* ], stmts [ $($stmts:tt)* ], rest [],
        cont {
            self_val = $self_val:ident, name = $name:ident, variant = $variant:ident, pat = $pat:literal,
            arms [ $($arms:tt)* ], tail [ $($tail:tt)* ]
        }
    ) => {
        $crate::__nve_define_routes_emit_to_path! {
            self_val = $self_val,
            name = $name,
            arms [
                $($arms)*
                $name::$variant { $($f),* } => {
                    #[allow(unused_mut)]
                    let mut $p: ::std::vec::Vec<(&'static str, ::std::string::String)> =
                        ::std::vec::Vec::new();
                    #[allow(unused_mut)]
                    let mut $q = $crate::QueryParams::new();
                    $($stmts)*
                    $crate::route::__private::format_route_pattern($pat, &$p, &$q)
                },
            ],
            rest [ $($tail)* ]
        }
    };
    (
        p = $p:ident, q = $q:ident,
        names [ $($n:ident),* ], stmts [ $($stmts:tt)* ],
        rest [ #[query($qkey:literal)] $f:ident : Option<$inner:ty> $(, $($rest:tt)*)? ],
        cont $cont:tt
    ) => {
        $crate::__nve_define_routes_format_fields! {
            p = $p, q = $q,
            names [ $($n,)* $f ],
            stmts [ $($stmts)* if let ::core::option::Option::Some(v) = $f { $q.push($qkey, ::std::string::ToString::to_string(v)); } ],
            rest [ $($($rest)*)? ],
            cont $cont
        }
    };
    (
        p = $p:ident, q = $q:ident,
        names [ $($n:ident),* ], stmts [ $($stmts:tt)* ],
        rest [ #[query($qkey:literal)] $f:ident : $ty:ty $(, $($rest:tt)*)? ],
        cont $cont:tt
    ) => {
        $crate::__nve_define_routes_format_fields! {
            p = $p, q = $q,
            names [ $($n,)* $f ],
            stmts [ $($stmts)* $q.push($qkey, ::std::string::ToString::to_string($f)); ],
            rest [ $($($rest)*)? ],
            cont $cont
        }
    };
    (
        p = $p:ident, q = $q:ident,
        names [ $($n:ident),* ], stmts [ $($stmts:tt)* ],
        rest [ #[query] $f:ident : Option<$inner:ty> $(, $($rest:tt)*)? ],
        cont $cont:tt
    ) => {
        $crate::__nve_define_routes_format_fields! {
            p = $p, q = $q,
            names [ $($n,)* $f ],
            stmts [ $($stmts)* if let ::core::option::Option::Some(v) = $f { $q.push(::core::stringify!($f), ::std::string::ToString::to_string(v)); } ],
            rest [ $($($rest)*)? ],
            cont $cont
        }
    };
    (
        p = $p:ident, q = $q:ident,
        names [ $($n:ident),* ], stmts [ $($stmts:tt)* ],
        rest [ #[query] $f:ident : $ty:ty $(, $($rest:tt)*)? ],
        cont $cont:tt
    ) => {
        $crate::__nve_define_routes_format_fields! {
            p = $p, q = $q,
            names [ $($n,)* $f ],
            stmts [ $($stmts)* $q.push(::core::stringify!($f), ::std::string::ToString::to_string($f)); ],
            rest [ $($($rest)*)? ],
            cont $cont
        }
    };
    (
        p = $p:ident, q = $q:ident,
        names [ $($n:ident),* ], stmts [ $($stmts:tt)* ],
        rest [ #[fragment] $f:ident : Option<$inner:ty> $(, $($rest:tt)*)? ],
        cont $cont:tt
    ) => {
        $crate::__nve_define_routes_format_fields! {
            p = $p, q = $q,
            names [ $($n,)* $f ],
            stmts [ $($stmts)* if let ::core::option::Option::Some(v) = $f { $q.set_fragment(::core::option::Option::Some(::std::string::ToString::to_string(v))); } ],
            rest [ $($($rest)*)? ],
            cont $cont
        }
    };
    (
        p = $p:ident, q = $q:ident,
        names [ $($n:ident),* ], stmts [ $($stmts:tt)* ],
        rest [ $f:ident : $ty:ty $(, $($rest:tt)*)? ],
        cont $cont:tt
    ) => {
        $crate::__nve_define_routes_format_fields! {
            p = $p, q = $q,
            names [ $($n,)* $f ],
            stmts [ $($stmts)* $p.push((::core::stringify!($f), ::std::string::ToString::to_string($f))); ],
            rest [ $($($rest)*)? ],
            cont $cont
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __nve_define_routes_emit_title {
    (self_val = $self_val:ident, name = $name:ident, arms [ $($arms:tt)* ], rest []) => {
        match $self_val {
            $($arms)*
        }
    };
    (
        self_val = $self_val:ident, name = $name:ident, arms [ $($arms:tt)* ],
        rest [ arm { variant [ $variant:ident ] title [ $title:literal ] fields [] } $($tail:tt)* ]
    ) => {
        $crate::__nve_define_routes_emit_title! {
            self_val = $self_val, name = $name,
            arms [ $($arms)* $name::$variant => ::core::option::Option::Some($title), ],
            rest [ $($tail)* ]
        }
    };
    (
        self_val = $self_val:ident, name = $name:ident, arms [ $($arms:tt)* ],
        rest [ arm { variant [ $variant:ident ] title [] fields [] } $($tail:tt)* ]
    ) => {
        $crate::__nve_define_routes_emit_title! {
            self_val = $self_val, name = $name,
            arms [ $($arms)* $name::$variant => ::core::option::Option::None, ],
            rest [ $($tail)* ]
        }
    };
    (
        self_val = $self_val:ident, name = $name:ident, arms [ $($arms:tt)* ],
        rest [ arm { variant [ $variant:ident ] title [ $title:literal ] fields [ $($fields:tt)+ ] } $($tail:tt)* ]
    ) => {
        $crate::__nve_define_routes_emit_title! {
            self_val = $self_val, name = $name,
            arms [ $($arms)* $name::$variant { .. } => ::core::option::Option::Some($title), ],
            rest [ $($tail)* ]
        }
    };
    (
        self_val = $self_val:ident, name = $name:ident, arms [ $($arms:tt)* ],
        rest [ arm { variant [ $variant:ident ] title [] fields [ $($fields:tt)+ ] } $($tail:tt)* ]
    ) => {
        $crate::__nve_define_routes_emit_title! {
            self_val = $self_val, name = $name,
            arms [ $($arms)* $name::$variant { .. } => ::core::option::Option::None, ],
            rest [ $($tail)* ]
        }
    };
}
