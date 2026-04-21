//! Per-language implementations.
//!
//! Every language owns exactly one folder here containing its struct,
//! `impl Language`, grammar loader + SCM includes, and (when applicable)
//! framework subpackages.

pub mod al;
pub mod ansible;
pub mod asm;
pub mod bash;
pub mod blade;
pub mod builtins;
pub mod c;
pub mod cpp;
pub mod csharp;
pub mod css;
pub mod dart;
pub mod ejs;
pub mod elixir;
pub mod erlang;
pub mod fortran;
pub mod gleam;
pub mod go;
pub mod java;
pub mod javascript;
pub mod json;
pub mod jsx;
pub mod kotlin;
pub mod less;
pub mod lua;
pub mod luau;
pub mod markdown;
pub mod nix;
pub mod openapi;
pub mod perl;
pub mod php;
pub mod python;
pub mod razor;
pub mod ruby;
pub mod rust;
pub mod sass;
pub mod scss;
pub mod sql;
pub mod styl;
pub mod swift;
pub mod toml;
pub mod tree_sitter_query;
pub mod tsx;
pub mod typescript;
pub mod verse;
pub mod vue;
pub mod xml;
pub mod yaml;

pub use builtins::register_builtins;
