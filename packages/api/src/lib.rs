//! This crate contains all shared fullstack server functions.
#[cfg(feature = "server")]
pub mod generated;
#[cfg(feature = "server")]
pub use generated::*;
#[cfg(feature = "server")]
pub mod models;
pub mod routes;
pub use routes::*;
#[cfg(feature = "server")]
pub mod schemas;
#[cfg(feature = "server")]
pub mod state;
#[cfg(feature = "server")]
pub use state::*;
#[cfg(feature = "server")]
pub mod store;
