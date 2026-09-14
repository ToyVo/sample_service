#[cfg(feature = "server")]
pub mod backend;
#[cfg(feature = "server")]
pub use backend::*;
#[cfg(feature = "server")]
pub mod generated;
#[cfg(feature = "server")]
pub use generated::*;

pub mod frontend;
pub use frontend::*;

/// Define a components module that contains all shared components for our app.
pub mod components;
/// Define a views module that contains the UI for all Layouts and Routes for our app.
pub mod views;

#[cfg(feature = "server")]
pub mod models;
#[cfg(feature = "server")]
pub mod schema;
