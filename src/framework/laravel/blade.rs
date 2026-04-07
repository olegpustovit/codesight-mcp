use regex::Regex;

/// Convert blade dot-notation to a file path.
/// `welcome` -> `resources/views/welcome.blade.php`
/// `layouts.app` -> `resources/views/layouts/app.blade.php`
pub fn blade_dot_to_path(dot_name: &str) -> String {
    let parts: Vec<&str> = dot_name.split('.').collect();
    format!("resources/views/{}.blade.php", parts.join("/"))
}

/// Convert a Blade component name to a file path.
/// `alert` -> `resources/views/components/alert.blade.php`
/// `forms.input` -> `resources/views/components/forms/input.blade.php`
pub fn blade_component_to_path(component_name: &str) -> String {
    let parts: Vec<&str> = component_name.split('.').collect();
    format!("resources/views/components/{}.blade.php", parts.join("/"))
}

/// Extract Blade import references from template content.
pub fn extract_blade_imports(content: &str) -> Vec<String> {
    let mut imports = Vec::new();

    // @extends('name')
    let extends_re = Regex::new(r#"@extends\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap();
    for cap in extends_re.captures_iter(content) {
        imports.push(blade_dot_to_path(&cap[1]));
    }

    // @include('name')
    let include_re = Regex::new(r#"@include\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap();
    for cap in include_re.captures_iter(content) {
        imports.push(blade_dot_to_path(&cap[1]));
    }

    // @includeWhen($cond, 'name')
    let include_when_re = Regex::new(r#"@includeWhen\([^,]+,\s*['"]([^'"]+)['"]\s*\)"#).unwrap();
    for cap in include_when_re.captures_iter(content) {
        imports.push(blade_dot_to_path(&cap[1]));
    }

    // @includeFirst(['name1', 'name2']) - only take first
    let include_first_re = Regex::new(r#"@includeFirst\(\s*\[\s*['"]([^'"]+)['"]"#).unwrap();
    for cap in include_first_re.captures_iter(content) {
        imports.push(blade_dot_to_path(&cap[1]));
    }

    // <x-component-name /> or <x-component-name>
    let component_re = Regex::new(r"<x-([\w.-]+)\s*[/> ]").unwrap();
    for cap in component_re.captures_iter(content) {
        imports.push(blade_component_to_path(&cap[1]));
    }

    imports
}
