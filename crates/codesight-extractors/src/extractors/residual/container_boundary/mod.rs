//! Residual extractors for host/container boundaries.
//!
//! **A7 status (2026-04-10):** `ejs` is now grammar-backed — the extractor is a thin
//! facade around `runtime/queries/ejs/` (tree-sitter-embedded-template) plus two
//! documented host post-passes. `vue`, `razor`, and `blade` remain on Rust shims
//! **only** because their upstream grammars are unavailable at our `tree-sitter
//! 0.26.8` baseline:
//!
//! | Host  | Blocker                                                        |
//! |-------|----------------------------------------------------------------|
//! | vue   | `tree-sitter-vue` 0.0.3 links against `tree-sitter 0.20.10`, type-incompatible with our 0.26.8; no newer stable crate is published. |
//! | razor | `tree-sitter-razor` 0.1.0 is on the same pre-0.22 era; no 0.25+ release exists. |
//! | blade | No `tree-sitter-blade` crate on crates.io at all.              |
//!
//! Each facade below keeps its own exit criteria. Promoting any of these to a
//! grammar-backed pack is a one-file swap once a compatible grammar lands: add the
//! dep, register the grammar in `catalogs/grammar.rs`, create
//! `runtime/queries/{host}/{tags,injections}.scm`, and delete this module.

pub(crate) mod blade;
pub(crate) mod ejs;
pub(crate) mod razor;
pub(crate) mod vue;
