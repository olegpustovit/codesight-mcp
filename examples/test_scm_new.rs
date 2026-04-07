use codesight_mcp::extractors::engine::scm::{ScmExtractor, parse_scm_header};

fn main() {
    for (name, src) in &[
        ("dart", include_str!("../runtime/queries/dart/symbols.scm")),
        ("xml", include_str!("../runtime/queries/xml/symbols.scm")),
        ("nix", include_str!("../runtime/queries/nix/symbols.scm")),
    ] {
        let meta = parse_scm_header(src);
        print!("{name}: grammar={} ... ", meta.grammar);
        match ScmExtractor::new(&meta, src) {
            Ok(_) => println!("OK"),
            Err(e) => println!("FAILED: {e}"),
        }
    }
}
