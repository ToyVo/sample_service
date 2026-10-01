//! This crate contains all shared UI for the workspace.
//!
//! Styled primitives from `dioxus-components` live in [`components`] (added
//! via `dx components add ... --module-path ../ui/src/components` from a
//! binary crate). All app views (`Home`, `Blog`, `Docs`, `Console`, …) live
//! here so `web`, `desktop`, and `mobile` share one implementation.
// `asset!` in const position trips a nightly `volatile_composites` false
// positive that cannot be addressed in user code.
#![allow(clippy::volatile_composites)]

use dioxus::prelude::*;

/// Theme variables required by every styled component in [`components`].
///
/// `with_preload(true)` inlines a `<link rel="preload">` into the base html
/// at compile time (see Dioxus #3970), so the theme applies before the wasm
/// mounts and avoids a flash of unstyled content.
pub const THEME_CSS: Asset = asset!(
    "/assets/dx-components-theme.css",
    CssAssetOptions::new().with_preload(true)
);

/// Styled `dioxus-components` (button, card, accordion, …).
///
/// Vendored via `dx components add`; third-party styling code below the
/// workspace pedantic/nursery bar, so lints are relaxed for this subtree only.
#[allow(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    clippy::restriction,
    clippy::empty_enums,
    clippy::volatile_composites
)]
pub mod components;

mod navbar;
pub use navbar::Navbar;

mod docs;
pub use docs::{Docs, Endpoint, ParamInfo, parse_endpoints};

mod api_explorer;
pub use api_explorer::ApiExplorer;

mod console;
pub use console::Console;

mod not_found;
pub use not_found::NotFound;
