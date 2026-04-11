//! Tree-sitter–backed import capture (primary path when parse succeeds without errors).

mod go;
mod js_ts;
mod python;
mod rust;

pub use go::extract_go_imports_ts;
pub use js_ts::extract_js_imports_ts;
pub use python::extract_python_imports_ts;
pub use rust::extract_rust_imports_ts;
