use tree_sitter::Language;

#[derive(Clone, Copy)]
pub struct GrammarDefinition {
    pub language_name: &'static str,
    pub grammar_name: &'static str,
    pub extensions: &'static [&'static str],
    loader: fn() -> Language,
}

impl GrammarDefinition {
    pub fn language(self) -> Language {
        (self.loader)()
    }
}

pub struct GrammarCatalog {
    entries: &'static [GrammarDefinition],
}

impl GrammarCatalog {
    pub const fn builtin() -> Self {
        Self {
            entries: BUILTIN_GRAMMARS,
        }
    }

    pub fn find_by_grammar(&self, grammar_name: &str) -> Option<&'static GrammarDefinition> {
        self.entries
            .iter()
            .find(|entry| entry.grammar_name == grammar_name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &'static GrammarDefinition> {
        self.entries.iter()
    }
}

const BUILTIN_GRAMMARS: &[GrammarDefinition] = &[
    GrammarDefinition {
        language_name: "bash",
        grammar_name: "bash",
        extensions: &["sh", "bash"],
        loader: || tree_sitter_bash::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "csharp",
        grammar_name: "c_sharp",
        extensions: &["cs"],
        loader: || tree_sitter_c_sharp::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "cpp",
        grammar_name: "cpp",
        extensions: &["c", "cc", "cpp", "cxx", "h", "hpp", "hh", "hxx"],
        loader: || tree_sitter_cpp::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "css",
        grammar_name: "css",
        extensions: &["css"],
        loader: || tree_sitter_css::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "dart",
        grammar_name: "dart",
        extensions: &["dart"],
        loader: || tree_sitter_dart::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "elixir",
        grammar_name: "elixir",
        extensions: &["ex", "exs"],
        loader: || tree_sitter_elixir::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "erlang",
        grammar_name: "erlang",
        extensions: &["erl", "hrl"],
        loader: || tree_sitter_erlang::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "fortran",
        grammar_name: "fortran",
        extensions: &["f", "f90", "f95", "f03", "f08"],
        loader: || tree_sitter_fortran::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "gleam",
        grammar_name: "gleam",
        extensions: &["gleam"],
        loader: || tree_sitter_gleam::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "go",
        grammar_name: "go",
        extensions: &["go"],
        loader: || tree_sitter_go::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "java",
        grammar_name: "java",
        extensions: &["java"],
        loader: || tree_sitter_java::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "javascript",
        grammar_name: "javascript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        loader: || tree_sitter_javascript::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "json",
        grammar_name: "json",
        extensions: &["json"],
        loader: || tree_sitter_json::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "kotlin",
        grammar_name: "kotlin",
        extensions: &["kt", "kts"],
        loader: || tree_sitter_kotlin_ng::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "lua",
        grammar_name: "lua",
        extensions: &["lua"],
        loader: || tree_sitter_lua::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "luau",
        grammar_name: "luau",
        extensions: &["luau"],
        loader: || tree_sitter_luau::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "nix",
        grammar_name: "nix",
        extensions: &["nix"],
        loader: || tree_sitter_nix::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "perl",
        grammar_name: "perl",
        extensions: &["pl", "pm", "psgi", "cgi"],
        loader: || tree_sitter_perl::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "php",
        grammar_name: "php",
        extensions: &["php"],
        loader: || tree_sitter_php::LANGUAGE_PHP.into(),
    },
    GrammarDefinition {
        language_name: "python",
        grammar_name: "python",
        extensions: &["py"],
        loader: || tree_sitter_python::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "ruby",
        grammar_name: "ruby",
        extensions: &["rb"],
        loader: || tree_sitter_ruby::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "rust",
        grammar_name: "rust",
        extensions: &["rs"],
        loader: || tree_sitter_rust::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "scss",
        grammar_name: "scss",
        extensions: &["scss"],
        loader: || tree_sitter_scss::language(),
    },
    GrammarDefinition {
        language_name: "sql",
        grammar_name: "sql",
        extensions: &["sql"],
        loader: || tree_sitter_sequel::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "swift",
        grammar_name: "swift",
        extensions: &["swift"],
        loader: || tree_sitter_swift::LANGUAGE.into(),
    },
    GrammarDefinition {
        language_name: "typescript",
        grammar_name: "typescript",
        extensions: &["ts"],
        loader: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    },
    GrammarDefinition {
        language_name: "tsx",
        grammar_name: "tsx",
        extensions: &["tsx"],
        loader: || tree_sitter_typescript::LANGUAGE_TSX.into(),
    },
    GrammarDefinition {
        language_name: "xml",
        grammar_name: "xml",
        extensions: &["xml", "svg"],
        loader: || tree_sitter_xml::LANGUAGE_XML.into(),
    },
    GrammarDefinition {
        language_name: "yaml",
        grammar_name: "yaml",
        extensions: &["yaml", "yml"],
        loader: || tree_sitter_yaml::LANGUAGE.into(),
    },
];
