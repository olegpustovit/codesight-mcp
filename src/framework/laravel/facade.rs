use std::collections::HashMap;

use regex::Regex;

use crate::framework::ExtraImport;

/// Map of Laravel facade names to their underlying classes.
fn facade_map() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("Cache", r"Illuminate\Cache\CacheManager");
    m.insert("DB", r"Illuminate\Database\DatabaseManager");
    m.insert("Log", r"Illuminate\Log\LogManager");
    m.insert("Auth", r"Illuminate\Auth\AuthManager");
    m.insert("Event", r"Illuminate\Events\Dispatcher");
    m.insert("Queue", r"Illuminate\Queue\QueueManager");
    m.insert("Mail", r"Illuminate\Mail\Mailer");
    m.insert("Route", r"Illuminate\Routing\Router");
    m.insert("Session", r"Illuminate\Session\SessionManager");
    m.insert("Storage", r"Illuminate\Filesystem\FilesystemManager");
    m.insert("Validator", r"Illuminate\Validation\Factory");
    m.insert("View", r"Illuminate\View\Factory");
    m.insert("Config", r"Illuminate\Config\Repository");
    m.insert("Hash", r"Illuminate\Hashing\HashManager");
    m.insert("Notification", r"Illuminate\Notifications\ChannelManager");
    m
}

/// Extract facade usage from PHP content and return resolved imports.
pub fn extract_facade_imports(content: &str) -> Vec<ExtraImport> {
    let facades = facade_map();
    let re = Regex::new(r"\b(\w+)::(\w+)\(").unwrap();

    let mut seen = std::collections::HashSet::new();
    let mut imports = Vec::new();

    for cap in re.captures_iter(content) {
        let facade_name = &cap[1];
        let method = &cap[2];

        // Skip ::class references
        if method == "class" {
            continue;
        }

        if let Some(resolved) = facades.get(facade_name) {
            if seen.insert(facade_name.to_string()) {
                imports.push(ExtraImport {
                    specifier: resolved.to_string(),
                    names: vec![facade_name.to_string()],
                });
            }
        }
    }

    imports
}
