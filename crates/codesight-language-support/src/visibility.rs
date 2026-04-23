//! Shared visibility predicates reused across languages that model the
//! same `public` / `private` / `protected` convention at the declaration
//! line. One implementation per convention — callers dispatch via the
//! `Language::is_restricted` override.

/// C-family convention (C#, C++, Java, Kotlin, Dart). A declaration is
/// restricted when it begins with `private`/`protected`/`internal`, or
/// when it does not begin with `public`.
pub fn cfamily_is_restricted(source_text: &str) -> bool {
    let trimmed = source_text.trim_start();
    if trimmed.starts_with("private ")
        || trimmed.starts_with("protected ")
        || trimmed.starts_with("internal ")
    {
        return true;
    }
    !trimmed.starts_with("public ")
}

/// BEAM convention (Elixir, Erlang): `defp` / `defmacrop` declares a
/// private function; everything else is public.
pub fn beam_is_restricted(source_text: &str) -> bool {
    let trimmed = source_text.trim_start();
    trimmed.starts_with("defp ") || trimmed.starts_with("defmacrop ")
}

/// JS/TS export convention. A declaration is restricted when it begins
/// with `private`/`protected`, or when it is not an `export`.
pub fn js_like_is_restricted(source_text: &str) -> bool {
    let trimmed = source_text.trim_start();
    if trimmed.starts_with("private ") || trimmed.starts_with("protected ") {
        return true;
    }
    !trimmed.starts_with("export ")
}
