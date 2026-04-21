use codesight_language_support::scm_engine::{ScmExtractor, parse_scm_header};

fn main() {
    for (name, src) in &[
        (
            "dart",
            include_str!("../crates/codesight-language-support/src/languages/dart/queries/tags.scm"),
        ),
        (
            "xml",
            include_str!("../crates/codesight-language-support/src/languages/xml/queries/tags.scm"),
        ),
        (
            "nix",
            include_str!("../crates/codesight-language-support/src/languages/nix/queries/tags.scm"),
        ),
    ] {
        let meta = parse_scm_header(src);
        print!("{name}: grammar={} ... ", meta.grammar);
        match ScmExtractor::new(&meta, src) {
            Ok(_) => println!("OK"),
            Err(e) => println!("FAILED: {e}"),
        }
    }
}
