//! Integration tests: yaml language.

use rustc_hash::FxHashMap;
use std::path::Path;

use symslice_core::SymbolKind;
use symslice_language_support::LanguageRegistry;
use symslice_language_support::languages::yaml::extractor::YamlExtractor;

// ---------------------------------------------------------------------------
// Inline fixtures (matching the Python module-level constants)
// ---------------------------------------------------------------------------

const GENERIC_YAML_SOURCE: &str = "\
services:
  api:
    image: example/api:latest
    ports:
      - \"8080:8080\"
feature_flags:
  signup: true
";

const ANSIBLE_PLAYBOOK_SOURCE: &str = "\
- name: Configure web nodes
  hosts: web
  tasks:
    - name: Install nginx
      ansible.builtin.package:
        name: nginx
        state: present
    - name: Start service
      ansible.builtin.service:
        name: nginx
        state: started
  handlers:
    - name: Restart nginx
      ansible.builtin.service:
        name: nginx
        state: restarted
";

const ANSIBLE_TASKS_SOURCE: &str = "\
- name: Create app directory
  ansible.builtin.file:
    path: /srv/app
    state: directory

- ansible.builtin.template:
    src: app.conf.j2
    dest: /etc/app.conf
";

const ANSIBLE_VARS_SOURCE: &str = "\
app_port: 8080
app_debug: true
database:
  host: db.internal
  port: 5432
";

const MULTI_DOC_YAML_SOURCE: &str = "\
services:
  api:
    image: example/api:latest
---
feature_flags:
  signup: true
";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_yaml(source: &str, file_path: &str) -> Vec<symslice_core::StoredSymbol> {
    YamlExtractor
        .extract(file_path, source)
        .expect("yaml extraction should not fail")
}

fn by_qualified(
    symbols: &[symslice_core::StoredSymbol],
) -> FxHashMap<&str, &symslice_core::StoredSymbol> {
    symbols
        .iter()
        .map(|s| (s.qualified_name.as_str(), s))
        .collect()
}

// ---------------------------------------------------------------------------
// 1. test_yaml_extension_mapping
// ---------------------------------------------------------------------------
///
/// We verify via `LanguageRegistry` that both extensions resolve to the yaml
/// extractor whose `language_name()` is `"yaml"`.
#[test]
fn test_yaml_extension_mapping() {
    let reg = LanguageRegistry::default();

    let yaml_ext = reg.extractor_for_path(Path::new("settings.yaml"));
    assert!(yaml_ext.is_some(), ".yaml should be recognized");
    assert_eq!(yaml_ext.unwrap().id().as_str(), "yaml");

    let yml_ext = reg.extractor_for_path(Path::new("settings.yml"));
    assert!(yml_ext.is_some(), ".yml should be recognized");
    assert_eq!(yml_ext.unwrap().id().as_str(), "yaml");
}

// ---------------------------------------------------------------------------
// 2. test_ansible_path_heuristics_override_yaml
// ---------------------------------------------------------------------------
/// should resolve to language `"ansible"` rather than plain `"yaml"` via
/// `extractors::classifier` path layout + `.yml`/`.yaml`.
#[test]
fn test_ansible_path_heuristics_override_yaml() {
    let reg = LanguageRegistry::default();

    for path in &[
        "playbooks/site.yml",
        "playbooks/deploy.yml",
        "roles/web/tasks/main.yaml",
        "group_vars/all.yml",
        "inventory/group_vars/all.yml",
    ] {
        let ext = reg.extractor_for_path(Path::new(path));
        assert!(ext.is_some(), "{path} should be recognized");
        assert_eq!(
            ext.unwrap().id().as_str(),
            "ansible",
            "{path} should map to ansible, not yaml"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. test_openapi_basename_still_wins_over_yaml
// ---------------------------------------------------------------------------
///
/// Basename rules live in `extractors::classifier` (OpenAPI stems + extensions).
#[test]
fn test_openapi_basename_still_wins_over_yaml() {
    let reg = LanguageRegistry::default();

    let openapi = reg.extractor_for_path(Path::new("openapi.yaml"));
    assert!(openapi.is_some(), "openapi.yaml should be recognized");
    assert_eq!(openapi.unwrap().id().as_str(), "openapi");

    let swagger = reg.extractor_for_path(Path::new("swagger.yml"));
    assert!(swagger.is_some(), "swagger.yml should be recognized");
    assert_eq!(swagger.unwrap().id().as_str(), "openapi");
}

// ---------------------------------------------------------------------------
// 4. test_parse_generic_yaml_symbols
// ---------------------------------------------------------------------------
/// symbols with `type` for nested mappings and `constant` for leaf scalars.
#[test]
fn test_parse_generic_yaml_symbols() {
    let symbols = extract_yaml(GENERIC_YAML_SOURCE, "config/settings.yaml");
    let qn = by_qualified(&symbols);

    assert!(
        qn.contains_key("services"),
        "missing 'services': {:#?}",
        qn.keys().collect::<Vec<_>>()
    );
    assert_eq!(qn["services"].kind, SymbolKind::Type);

    assert!(qn.contains_key("services.api"), "missing 'services.api'");
    assert_eq!(qn["services.api"].kind, SymbolKind::Type);

    assert!(
        qn.contains_key("services.api.image"),
        "missing 'services.api.image'"
    );
    assert_eq!(qn["services.api.image"].kind, SymbolKind::Constant);

    assert!(
        qn.contains_key("feature_flags.signup"),
        "missing 'feature_flags.signup'"
    );
    assert_eq!(qn["feature_flags.signup"].kind, SymbolKind::Constant);
}

// ---------------------------------------------------------------------------
// 5. test_parse_multi_document_yaml_symbols
// ---------------------------------------------------------------------------
/// from all documents.
#[test]
fn test_parse_multi_document_yaml_symbols() {
    let symbols = extract_yaml(MULTI_DOC_YAML_SOURCE, "config/settings.yaml");
    let qn = by_qualified(&symbols);

    assert!(
        qn.contains_key("services.api.image"),
        "missing 'services.api.image' in multi-doc: {:#?}",
        qn.keys().collect::<Vec<_>>()
    );
    assert!(
        qn.contains_key("feature_flags.signup"),
        "missing 'feature_flags.signup' in multi-doc"
    );
}

// ---------------------------------------------------------------------------
// 6. test_parse_ansible_playbook_symbols
// ---------------------------------------------------------------------------
/// and nested task/handler symbols (kind=function) with qualified names like
/// `"Configure web nodes.tasks.Install nginx"`.
///
/// The current implementation produces `"Configure web nodes.Install nginx"`
/// (no `tasks`/`handlers` segment in the qualified name) because
/// `extract_task_list` uses only the play name as the parent scope. The test
/// asserts reference behavior and is ignored until the scope includes the
/// section name.
#[test]
fn test_parse_ansible_playbook_symbols() {
    let symbols = extract_yaml(ANSIBLE_PLAYBOOK_SOURCE, "playbooks/site.yml");
    let qn = by_qualified(&symbols);

    // Play itself
    assert!(
        qn.contains_key("Configure web nodes"),
        "missing play symbol"
    );
    assert_eq!(qn["Configure web nodes"].kind, SymbolKind::Class);

    // Tasks nested under play
    assert!(
        qn.contains_key("Configure web nodes.tasks.Install nginx"),
        "missing 'Configure web nodes.tasks.Install nginx': {:#?}",
        qn.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        qn["Configure web nodes.tasks.Install nginx"].kind,
        SymbolKind::Function
    );

    // Handlers nested under play
    assert!(
        qn.contains_key("Configure web nodes.handlers.Restart nginx"),
        "missing 'Configure web nodes.handlers.Restart nginx'"
    );
    assert_eq!(
        qn["Configure web nodes.handlers.Restart nginx"].kind,
        SymbolKind::Function
    );
}

// ---------------------------------------------------------------------------
// 7. test_parse_ansible_tasks_file_symbols
// ---------------------------------------------------------------------------
/// produces symbols with a synthetic `task.tasks.` prefix, and unnamed tasks
/// use the module FQCN as their name.
///
/// The current implementation:
/// - Does not prepend `task.tasks.` prefix for standalone task files.
/// - Skips tasks that lack a `name:` key (unnamed module-only tasks).
#[test]
fn test_parse_ansible_tasks_file_symbols() {
    let symbols = extract_yaml(ANSIBLE_TASKS_SOURCE, "roles/app/tasks/main.yml");
    let qn = by_qualified(&symbols);

    assert!(
        qn.contains_key("task.tasks.Create app directory"),
        "missing 'task.tasks.Create app directory': {:#?}",
        qn.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        qn["task.tasks.Create app directory"].kind,
        SymbolKind::Function
    );

    // Unnamed task: reference expects the module FQCN as the task name.
    assert!(
        qn.contains_key("task.tasks.ansible.builtin.template"),
        "missing 'task.tasks.ansible.builtin.template'"
    );
    assert_eq!(
        qn["task.tasks.ansible.builtin.template"].kind,
        SymbolKind::Function
    );
}

// ---------------------------------------------------------------------------
// 8. test_parse_ansible_var_symbols
// ---------------------------------------------------------------------------
/// mappings — leaf keys are constants, nested mappings are types.
#[test]
fn test_parse_ansible_var_symbols() {
    // The path triggers ansible mode internally, but vars files have no
    // sequences so the generic YAML mapping visitor handles them identically.
    let symbols = extract_yaml(ANSIBLE_VARS_SOURCE, "inventory/group_vars/all.yml");
    let qn = by_qualified(&symbols);

    assert!(
        qn.contains_key("app_port"),
        "missing 'app_port': {:#?}",
        qn.keys().collect::<Vec<_>>()
    );
    assert_eq!(qn["app_port"].kind, SymbolKind::Constant);

    assert!(qn.contains_key("database"), "missing 'database'");
    assert_eq!(qn["database"].kind, SymbolKind::Type);

    assert!(qn.contains_key("database.host"), "missing 'database.host'");
    assert_eq!(qn["database.host"].kind, SymbolKind::Constant);
}
