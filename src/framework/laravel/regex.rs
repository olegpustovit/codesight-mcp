//! Compiled Laravel-specific regular expressions.
//!
//! All Laravel submodules share this single `LazyLock<LaravelRegexes>` so
//! each pattern is compiled exactly once and the only `.expect()` site lives
//! here. Patterns are hard-coded literals; compilation can only fail at
//! startup if the literal is malformed — which is covered by the unit test
//! that forces initialization below.

use std::sync::LazyLock;

use regex::Regex;

pub(super) struct LaravelRegexes {
    // migrations.rs
    pub migration_modifier_chain: Regex,
    pub migration_single_modifier: Regex,

    // routes.rs
    pub route_array_callback: Regex,

    // imports.rs
    pub route_controller_fqn: Regex,
    pub model_fqn_class: Regex,

    // models.rs
    pub model_relationship: Regex,
    pub model_fillable: Regex,
    pub fillable_item: Regex,
    pub scope_method: Regex,

    // api_client.rs
    pub fetch_api: Regex,
    pub axios_api: Regex,

    // facade.rs
    pub facade_call: Regex,

    // blade.rs
    pub blade_extends: Regex,
    pub blade_include: Regex,
    pub blade_include_when: Regex,
    pub blade_include_first: Regex,
    pub blade_component: Regex,

    // php_structure.rs
    pub schema_create: Regex,
    pub route_verb: Regex,
    pub route_name: Regex,

    // inertia.rs
    pub inertia_render: Regex,
    pub inertia_helper: Regex,
}

impl LaravelRegexes {
    fn new() -> Self {
        Self {
            migration_modifier_chain: Regex::new(r"^(->(\w+)\(\))+")
                .expect("laravel regex literal"),
            migration_single_modifier: Regex::new(r"->(\w+)\(\)")
                .expect("laravel regex literal"),
            route_array_callback: Regex::new(
                r#"Route::(get|post|put|delete|patch)\(\s*['"]([^'"]+)['"]\s*,\s*\[([A-Za-z\\]+)::class\s*,\s*['"](\w+)['"]\s*\]\s*\)(?:\s*->\s*name\(\s*['"]([^'"]+)['"]\s*\))?"#,
            )
            .expect("laravel regex literal"),
            route_controller_fqn: Regex::new(r#"(\\[A-Za-z\\]+Controller)::class"#)
                .expect("laravel regex literal"),
            model_fqn_class: Regex::new(r#"(\\[A-Za-z\\]+)::class"#)
                .expect("laravel regex literal"),
            model_relationship: Regex::new(
                r#"\$this->(hasMany|hasOne|belongsTo|belongsToMany|hasManyThrough|hasOneThrough|morphMany|morphOne|morphTo|morphToMany|morphedByMany)\(\s*\\?([A-Za-z\\]+)::class"#,
            )
            .expect("laravel regex literal"),
            model_fillable: Regex::new(r#"\$fillable\s*=\s*\[(.*?)\]"#)
                .expect("laravel regex literal"),
            fillable_item: Regex::new(r#"['"](\w+)['"]"#).expect("laravel regex literal"),
            scope_method: Regex::new(r"(?:public\s+)?function\s+scope(\w+)")
                .expect("laravel regex literal"),
            fetch_api: Regex::new(r#"fetch\(\s*['"](/api/[^'"]+)['"]\s*"#)
                .expect("laravel regex literal"),
            axios_api: Regex::new(r#"axios\.\w+\(\s*['"](/api/[^'"]+)['"]\s*"#)
                .expect("laravel regex literal"),
            facade_call: Regex::new(r"\b(\w+)::(\w+)\(").expect("laravel regex literal"),
            blade_extends: Regex::new(r#"@extends\(\s*['"]([^'"]+)['"]\s*\)"#)
                .expect("laravel regex literal"),
            blade_include: Regex::new(r#"@include\(\s*['"]([^'"]+)['"]\s*\)"#)
                .expect("laravel regex literal"),
            blade_include_when: Regex::new(r#"@includeWhen\([^,]+,\s*['"]([^'"]+)['"]\s*\)"#)
                .expect("laravel regex literal"),
            blade_include_first: Regex::new(r#"@includeFirst\(\s*\[\s*['"]([^'"]+)['"]"#)
                .expect("laravel regex literal"),
            blade_component: Regex::new(r"<x-([\w.-]+)\s*[/> ]")
                .expect("laravel regex literal"),
            schema_create: Regex::new(r#"Schema::create\s*\(\s*['"](?P<t>[\w]+)['"]"#)
                .expect("laravel regex literal"),
            route_verb: Regex::new(r"Route::(get|post|put|delete|patch)\s*\(")
                .expect("laravel regex literal"),
            route_name: Regex::new(r#"->\s*name\(\s*['"]([^'"]+)['"]\s*\)"#)
                .expect("laravel regex literal"),
            inertia_render: Regex::new(r#"Inertia::render\(\s*['"]([^'"]+)['"]\s*"#)
                .expect("laravel regex literal"),
            inertia_helper: Regex::new(r#"(?:^|[^:\w])inertia\(\s*['"]([^'"]+)['"]\s*"#)
                .expect("laravel regex literal"),
        }
    }
}

pub(super) static RE: LazyLock<LaravelRegexes> = LazyLock::new(LaravelRegexes::new);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_patterns_compile() {
        // Touching each field forces compilation via LazyLock::force + field access.
        let r = &*RE;
        let _ = (
            r.migration_modifier_chain.as_str(),
            r.route_array_callback.as_str(),
            r.route_controller_fqn.as_str(),
            r.model_fqn_class.as_str(),
            r.model_relationship.as_str(),
            r.model_fillable.as_str(),
            r.fillable_item.as_str(),
            r.scope_method.as_str(),
            r.fetch_api.as_str(),
            r.axios_api.as_str(),
            r.facade_call.as_str(),
            r.blade_extends.as_str(),
            r.blade_include.as_str(),
            r.blade_include_when.as_str(),
            r.blade_include_first.as_str(),
            r.blade_component.as_str(),
            r.schema_create.as_str(),
            r.route_verb.as_str(),
            r.route_name.as_str(),
            r.inertia_render.as_str(),
            r.inertia_helper.as_str(),
        );
    }
}
