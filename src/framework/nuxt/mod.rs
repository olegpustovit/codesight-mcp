//! Nuxt.js framework context provider.
//!
//! Detects Nuxt via `nuxt.config.{ts,js}`, maps page paths to routes,
//! parses server API routes with method suffixes, and discovers auto-import
//! symbols from `composables/` and `utils/` directories.

mod provider;
mod routing;

pub use provider::{NuxtProvider, NuxtStats};
pub use routing::{NuxtApiRoute, nuxt_api_route_from_path, nuxt_route_from_path};
