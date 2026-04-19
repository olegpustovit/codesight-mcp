use codesight_extractors::extractors::engine::scm::{ScmExtractor, parse_scm_header};

fn main() {
    for (name, src) in &[
        ("dart", include_str!("../runtime/queries/dart/tags.scm")),
        ("xml", include_str!("../runtime/queries/xml/tags.scm")),
        ("nix", include_str!("../runtime/queries/nix/tags.scm")),
    ] {
        let meta = parse_scm_header(src);
        print!("{name}: grammar={} ... ", meta.grammar);
        match ScmExtractor::new(&meta, src) {
            Ok(_) => println!("OK"),
            Err(e) => println!("FAILED: {e}"),
        }
    }
}
