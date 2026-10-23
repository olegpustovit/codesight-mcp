use super::regex::RE;

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
    for cap in RE.blade_extends.captures_iter(content) {
        imports.push(blade_dot_to_path(&cap[1]));
    }

    // @include('name')
    for cap in RE.blade_include.captures_iter(content) {
        imports.push(blade_dot_to_path(&cap[1]));
    }

    // @includeWhen($cond, 'name')
    for cap in RE.blade_include_when.captures_iter(content) {
        imports.push(blade_dot_to_path(&cap[1]));
    }

    // @includeFirst(['name1', 'name2']) - only take first
    for cap in RE.blade_include_first.captures_iter(content) {
        imports.push(blade_dot_to_path(&cap[1]));
    }

    // <x-component-name /> or <x-component-name>
    for cap in RE.blade_component.captures_iter(content) {
        imports.push(blade_component_to_path(&cap[1]));
    }

    imports
}
