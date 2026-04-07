//! Next.js framework context provider.
//!
//! Detects Next.js via `next.config.{js,mjs,ts}`, maps App Router paths to
//! URL routes, extracts API route HTTP methods, and tags special files
//! (layouts, middleware, error boundaries).

mod middleware;
mod provider;
mod routing;

pub use provider::{NextjsProvider, NextjsStats};
pub use routing::{next_api_methods_from_content, next_route_from_path};
