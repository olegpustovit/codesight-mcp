//! Builtin language manifests + the `register_builtins` entry point.
//!
//! Each `static <LANG>_MANIFEST: LanguageManifest = …` captures the
//! identity metadata that the registry keys its lookup tables on. The
//! `grammar` field references the per-language `GRAMMAR` static defined in
//! that language's module, so all behavior lives next to the Rust code.

use std::sync::Arc;

use crate::manifest::{LanguageId, LanguageManifest};
use crate::registry::LanguageRegistry;

use super::{
    al, ansible, asm, bash, blade, c, cpp, csharp, css, dart, ejs, elixir, erlang, fortran, gleam,
    go, java, javascript, json, jsx, kotlin, less, lua, luau, markdown, nix, openapi, perl, php,
    python, razor, ruby, rust, sass, scss, sql, styl, swift, toml, tree_sitter_query, tsx,
    typescript, verse, vue, xml, yaml,
};
use crate::manifest::{
    BASH, CPP, CSHARP, CSS, DART, ELIXIR, GO, JAVA, JAVASCRIPT, JSON, KOTLIN, LUA, LUAU, PERL, PHP,
    PYTHON, RUBY, RUST, SCSS, SQL, SWIFT, TSX, TYPESCRIPT, XML, YAML,
};

macro_rules! manifest {
    (
        $name:ident,
        display = $display:expr,
        aliases = $aliases:expr,
        ext = $ext:expr,
        filenames = $fnames:expr,
        compound = $compound:expr,
        grammar = $grammar:expr,
        profiles = $profiles:expr,
        probe = $probe:expr $(,)?
    ) => {
        pub(crate) static $name: LanguageManifest = LanguageManifest {
            display_name: $display,
            aliases: $aliases,
            extensions: $ext,
            filename_overrides: $fnames,
            compound_extensions: $compound,
            grammar: $grammar,
            profiles: $profiles,
            content_probe: $probe,
        };
    };
}

manifest!(
    RUST_MANIFEST,
    display = "Rust",
    aliases = &["rs"],
    ext = &["rs"],
    filenames = &[],
    compound = &[],
    grammar = Some(&rust::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    TYPESCRIPT_MANIFEST,
    display = "TypeScript",
    aliases = &["ts"],
    ext = &["ts", "mts", "cts"],
    filenames = &[],
    compound = &["d.ts"],
    grammar = Some(&typescript::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    TSX_MANIFEST,
    display = "TSX",
    aliases = &[],
    ext = &["tsx"],
    filenames = &[],
    compound = &[],
    grammar = Some(&tsx::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    JAVASCRIPT_MANIFEST,
    display = "JavaScript",
    aliases = &["js"],
    ext = &["js", "mjs", "cjs"],
    filenames = &[],
    compound = &[],
    grammar = Some(&javascript::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    JSX_MANIFEST,
    display = "JSX",
    aliases = &[],
    ext = &["jsx"],
    filenames = &[],
    compound = &[],
    grammar = Some(&javascript::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    PYTHON_MANIFEST,
    display = "Python",
    aliases = &["py"],
    ext = &["py", "pyi"],
    filenames = &[],
    compound = &[],
    grammar = Some(&python::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    GO_MANIFEST,
    display = "Go",
    aliases = &["golang"],
    ext = &["go"],
    filenames = &[],
    compound = &[],
    grammar = Some(&go::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    JAVA_MANIFEST,
    display = "Java",
    aliases = &[],
    ext = &["java"],
    filenames = &[],
    compound = &[],
    grammar = Some(&java::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    RUBY_MANIFEST,
    display = "Ruby",
    aliases = &["rb"],
    ext = &["rb"],
    filenames = &[],
    compound = &[],
    grammar = Some(&ruby::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    PHP_MANIFEST,
    display = "PHP",
    aliases = &[],
    ext = &["php"],
    filenames = &[],
    compound = &[],
    grammar = Some(&php::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    CSHARP_MANIFEST,
    display = "C#",
    aliases = &["c_sharp", "cs"],
    ext = &["cs"],
    filenames = &[],
    compound = &[],
    grammar = Some(&csharp::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    CPP_MANIFEST,
    display = "C++",
    aliases = &[],
    ext = &["cc", "cpp", "cxx", "hpp", "hh", "hxx", "h"],
    filenames = &[],
    compound = &[],
    grammar = Some(&cpp::GRAMMAR),
    profiles = &[],
    probe = Some(cpp::cpp_header_content_probe)
);

manifest!(
    C_MANIFEST,
    display = "C",
    aliases = &[],
    ext = &["c"],
    filenames = &[],
    compound = &[],
    grammar = Some(&cpp::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    SWIFT_MANIFEST,
    display = "Swift",
    aliases = &[],
    ext = &["swift"],
    filenames = &[],
    compound = &[],
    grammar = Some(&swift::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    KOTLIN_MANIFEST,
    display = "Kotlin",
    aliases = &[],
    ext = &["kt", "kts"],
    filenames = &[],
    compound = &[],
    grammar = Some(&kotlin::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    DART_MANIFEST,
    display = "Dart",
    aliases = &[],
    ext = &["dart"],
    filenames = &[],
    compound = &[],
    grammar = Some(&dart::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    BASH_MANIFEST,
    display = "Bash",
    aliases = &["sh"],
    ext = &["sh", "bash"],
    filenames = &[],
    compound = &[],
    grammar = Some(&bash::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    SQL_MANIFEST,
    display = "SQL",
    aliases = &[],
    ext = &["sql"],
    filenames = &[],
    compound = &[],
    grammar = Some(&sql::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    YAML_MANIFEST,
    display = "YAML",
    aliases = &["yml"],
    ext = &["yaml", "yml"],
    filenames = &[],
    compound = &[],
    grammar = Some(&yaml::GRAMMAR),
    profiles = yaml::YAML_PROFILES,
    probe = None
);

manifest!(
    JSON_MANIFEST,
    display = "JSON",
    aliases = &[],
    ext = &["json"],
    filenames = &[],
    compound = &[],
    grammar = Some(&json::GRAMMAR),
    profiles = yaml::JSON_PROFILES,
    probe = None
);

manifest!(
    XML_MANIFEST,
    display = "XML",
    aliases = &[],
    ext = &["xml", "svg"],
    filenames = &[],
    compound = &[],
    grammar = Some(&xml::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    CSS_MANIFEST,
    display = "CSS",
    aliases = &[],
    ext = &["css"],
    filenames = &[],
    compound = &[],
    grammar = Some(&css::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    SCSS_MANIFEST,
    display = "SCSS",
    aliases = &[],
    ext = &["scss"],
    filenames = &[],
    compound = &[],
    grammar = Some(&scss::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    LESS_MANIFEST,
    display = "Less",
    aliases = &[],
    ext = &["less"],
    filenames = &[],
    compound = &[],
    grammar = None,
    profiles = &[],
    probe = None
);

manifest!(
    SASS_MANIFEST,
    display = "Sass",
    aliases = &[],
    ext = &["sass"],
    filenames = &[],
    compound = &[],
    grammar = None,
    profiles = &[],
    probe = None
);

manifest!(
    STYL_MANIFEST,
    display = "Stylus",
    aliases = &[],
    ext = &["styl"],
    filenames = &[],
    compound = &[],
    grammar = None,
    profiles = &[],
    probe = None
);

manifest!(
    LUA_MANIFEST,
    display = "Lua",
    aliases = &[],
    ext = &["lua"],
    filenames = &[],
    compound = &[],
    grammar = Some(&lua::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    LUAU_MANIFEST,
    display = "Luau",
    aliases = &[],
    ext = &["luau"],
    filenames = &[],
    compound = &[],
    grammar = Some(&luau::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    NIX_MANIFEST,
    display = "Nix",
    aliases = &[],
    ext = &["nix"],
    filenames = &[],
    compound = &[],
    grammar = Some(&nix::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    PERL_MANIFEST,
    display = "Perl",
    aliases = &[],
    ext = &["pl", "pm", "psgi", "cgi"],
    filenames = &[],
    compound = &[],
    grammar = Some(&perl::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    ASM_MANIFEST,
    display = "Assembly",
    aliases = &[],
    ext = &["asm", "s", "65816", "inc", "z80", "spc"],
    filenames = &[],
    compound = &[],
    grammar = None,
    profiles = &[],
    probe = None
);

manifest!(
    BLADE_MANIFEST,
    display = "Blade",
    aliases = &[],
    ext = &[],
    filenames = &[],
    compound = &["blade.php"],
    grammar = None,
    profiles = &[],
    probe = None
);

manifest!(
    RAZOR_MANIFEST,
    display = "Razor",
    aliases = &[],
    ext = &["cshtml", "razor"],
    filenames = &[],
    compound = &[],
    grammar = None,
    profiles = &[],
    probe = None
);

manifest!(
    EJS_MANIFEST,
    display = "EJS",
    aliases = &["embedded_template"],
    ext = &["ejs"],
    filenames = &[],
    compound = &[],
    grammar = Some(&ejs::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    VERSE_MANIFEST,
    display = "Verse",
    aliases = &[],
    ext = &["verse"],
    filenames = &[],
    compound = &[],
    grammar = None,
    profiles = &[],
    probe = None
);

manifest!(
    ELIXIR_MANIFEST,
    display = "Elixir",
    aliases = &[],
    ext = &["ex", "exs"],
    filenames = &[],
    compound = &[],
    grammar = Some(&elixir::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    ERLANG_MANIFEST,
    display = "Erlang",
    aliases = &[],
    ext = &["erl", "hrl"],
    filenames = &[],
    compound = &[],
    grammar = Some(&erlang::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    FORTRAN_MANIFEST,
    display = "Fortran",
    aliases = &[],
    ext = &["f", "f90", "f95", "f03", "f08", "for", "fpp"],
    filenames = &[],
    compound = &[],
    grammar = Some(&fortran::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    GLEAM_MANIFEST,
    display = "Gleam",
    aliases = &[],
    ext = &["gleam"],
    filenames = &[],
    compound = &[],
    grammar = Some(&gleam::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    VUE_MANIFEST,
    display = "Vue",
    aliases = &[],
    ext = &["vue"],
    filenames = &[],
    compound = &[],
    grammar = None,
    profiles = &[],
    probe = None
);

manifest!(
    AL_MANIFEST,
    display = "AL",
    aliases = &[],
    ext = &["al"],
    filenames = &[],
    compound = &[],
    grammar = None,
    profiles = &[],
    probe = None
);

manifest!(
    MARKDOWN_MANIFEST,
    display = "Markdown",
    aliases = &[],
    ext = &["md", "markdown"],
    filenames = &[],
    compound = &[],
    grammar = None,
    profiles = &[],
    probe = None
);

manifest!(
    TOML_MANIFEST,
    display = "TOML",
    aliases = &[],
    ext = &["toml"],
    filenames = &[],
    compound = &[],
    grammar = None,
    profiles = &[],
    probe = None
);

manifest!(
    TREE_SITTER_QUERY_MANIFEST,
    display = "Tree-sitter query",
    aliases = &[],
    ext = &["scm"],
    filenames = &[],
    compound = &[],
    grammar = None,
    profiles = &[],
    probe = None
);

// Host-profile languages. Routing is handled by the registry's profile
// dispatch — these manifests intentionally register no extensions; the
// host languages (YAML, JSON) still claim them.
manifest!(
    OPENAPI_MANIFEST,
    display = "OpenAPI",
    aliases = &["swagger"],
    ext = &[],
    filenames = &[],
    compound = &[],
    grammar = Some(&yaml::GRAMMAR),
    profiles = &[],
    probe = None
);

manifest!(
    ANSIBLE_MANIFEST,
    display = "Ansible",
    aliases = &[],
    ext = &[],
    filenames = &[],
    compound = &[],
    grammar = Some(&yaml::GRAMMAR),
    profiles = &[],
    probe = None
);

const CURATED: &[LanguageId] = &[
    RUST,
    TYPESCRIPT,
    TSX,
    JAVASCRIPT,
    PYTHON,
    GO,
    JAVA,
    RUBY,
    PHP,
    CSHARP,
    CPP,
    SWIFT,
    KOTLIN,
    DART,
    BASH,
    SQL,
    YAML,
    JSON,
    XML,
    CSS,
    SCSS,
    LUA,
    LUAU,
    PERL,
    ELIXIR,
    // Every other grammar-backed language inherits the default SCM pipeline
    // via `Language::extract_symbols`, which qualifies as a curated-query-pack
    // tier.
    crate::manifest::C,
    crate::manifest::JSX,
    crate::manifest::NIX,
    crate::manifest::ERLANG,
    crate::manifest::FORTRAN,
    crate::manifest::GLEAM,
    crate::manifest::VUE,
];

/// Register every builtin language in `registry`.
pub fn register_builtins(registry: &mut LanguageRegistry) {
    registry.register(Arc::new(rust::Rust));
    registry.register(Arc::new(typescript::TypeScript));
    registry.register(Arc::new(tsx::Tsx));
    registry.register(Arc::new(javascript::JavaScript));
    registry.register(Arc::new(jsx::Jsx));
    registry.register(Arc::new(python::Python));
    registry.register(Arc::new(go::Go));
    registry.register(Arc::new(java::Java));
    registry.register(Arc::new(ruby::Ruby));
    registry.register(Arc::new(php::Php));
    registry.register(Arc::new(csharp::CSharp));
    registry.register(Arc::new(cpp::Cpp));
    registry.register(Arc::new(c::C));
    registry.register(Arc::new(swift::Swift));
    registry.register(Arc::new(kotlin::Kotlin));
    registry.register(Arc::new(dart::Dart));
    registry.register(Arc::new(bash::Bash));
    registry.register(Arc::new(sql::SqlSupport));
    registry.register(Arc::new(yaml::YamlSupport));
    registry.register(Arc::new(json::JsonSupport));
    registry.register(Arc::new(xml::XmlSupport));
    registry.register(Arc::new(css::CssSupport));
    registry.register(Arc::new(scss::ScssSupport));
    registry.register(Arc::new(less::LessSupport));
    registry.register(Arc::new(sass::SassSupport));
    registry.register(Arc::new(styl::StylSupport));
    registry.register(Arc::new(lua::Lua));
    registry.register(Arc::new(luau::LuauSupport));
    registry.register(Arc::new(nix::NixSupport));
    registry.register(Arc::new(perl::Perl));
    registry.register(Arc::new(asm::AsmSupport));
    registry.register(Arc::new(blade::BladeSupport));
    registry.register(Arc::new(razor::RazorSupport));
    registry.register(Arc::new(ejs::EjsSupport));
    registry.register(Arc::new(verse::VerseSupport));
    registry.register(Arc::new(elixir::Elixir));
    registry.register(Arc::new(erlang::Erlang));
    registry.register(Arc::new(fortran::FortranSupport));
    registry.register(Arc::new(gleam::GleamSupport));
    registry.register(Arc::new(vue::VueSupport));
    registry.register(Arc::new(al::AlSupport));
    registry.register(Arc::new(markdown::MarkdownSupport));
    registry.register(Arc::new(toml::TomlSupport));
    registry.register(Arc::new(tree_sitter_query::TreeSitterQuerySupport));
    registry.register(Arc::new(openapi::Openapi));
    registry.register(Arc::new(ansible::Ansible));
    for id in CURATED {
        registry.mark_curated(*id);
    }
}
