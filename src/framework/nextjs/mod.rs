//! Next.js framework context provider.
//!
//! Detects Next.js via `next.config.{js,mjs,ts}`, maps App Router paths to
//! URL routes, extracts API route HTTP methods, and tags special files
//! (layouts, middleware, error boundaries).

mod middleware;
mod provider;
mod render;
mod routing;
mod snapshot;

pub use provider::NextjsProvider;
pub use render::render_file_context;
pub use routing::{next_api_methods_from_content, next_route_from_path};
pub use snapshot::{NextjsSnapshot, NextjsStats};
