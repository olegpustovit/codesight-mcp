use regex::Regex;
use std::sync::LazyLock;

// Top-level objects: table, codeunit, enum, page, report, query, xmlport, controladdin, interface
// Also extensions: pageextension, tableextension, enumextension
static TOP_LEVEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)^(table|codeunit|page|report|query|xmlport|pageextension|tableextension|enumextension|enum|controladdin)\s+(?:\d+\s+)?"([^"]+)"(?:\s+extends\s+"[^"]+")?"#,
    )
    .expect("infallible: al top-level object regex")
});

// Interface: interface "Name"
static INTERFACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^interface\s+"([^"]+)""#).expect("infallible: al regex"));

// Unquoted controladdin: controladdin MyAddIn
static CONTROLADDIN_UNQUOTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^controladdin\s+(\w+)"#).expect("infallible: al regex"));

// Procedures
static PROCEDURE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:local\s+|internal\s+)?procedure\s+(\w+)").expect("infallible: al regex")
});

// Triggers
static TRIGGER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^trigger\s+(\w+)\s*\(").expect("infallible: al regex"));

// Events (in controladdin)
static EVENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^event\s+(\w+)\s*\(").expect("infallible: al regex"));

// Fields: field(N; Name; Type) or field(N; "Name"; Type) or field(Name; SourceExpr) in page
static FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^field\(([^)]+)\)"#).expect("infallible: al regex"));

// Enum values: value(N; Name)
static VALUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^value\(([^)]+)\)"#).expect("infallible: al regex"));

// Actions: action(Name) or action("Name")
static ACTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^action\(([^)]+)\)"#).expect("infallible: al regex"));

// Keys: key(Name; Columns)
static KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^key\(([^)]+)\)"#).expect("infallible: al regex"));

// Columns: column(Name; SourceExpr)
static COLUMN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^column\(([^)]+)\)"#).expect("infallible: al regex"));

// FieldGroups: fieldgroup(Name; Columns)
static FIELDGROUP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^fieldgroup\(([^)]+)\)"#).expect("infallible: al regex"));

// DataItems: dataitem(Name; SourceTable)
static DATAITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^dataitem\(([^)]+)\)"#).expect("infallible: al regex"));

// XMLport elements: textelement(Name), tableelement(Name; Table), fieldelement(Name; SourceExpr)
static XMLPORT_ELEM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)^(textelement|tableelement|fieldelement)\(([^)]+)\)"#)
        .expect("infallible: al xmlport element regex")
});

// Variables: Name: Type (inside var block)
static VARIABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\w+)\s*:\s*(.+?)\s*;$").expect("infallible: al regex"));

pub(super) fn top_level_object(s: &str) -> Option<(String, String)> {
    if let Some(caps) = TOP_LEVEL.captures(s) {
        return Some((caps[1].to_lowercase(), caps[2].to_string()));
    }
    if let Some(caps) = INTERFACE.captures(s) {
        return Some(("interface".to_string(), caps[1].to_string()));
    }
    if let Some(caps) = CONTROLADDIN_UNQUOTED.captures(s) {
        let name = caps[1].to_string();
        // Don't match if already matched by TOP_LEVEL (quoted version)
        if !s.contains('"') {
            return Some(("controladdin".to_string(), name));
        }
    }
    None
}

pub(super) fn procedure(s: &str) -> Option<String> {
    PROCEDURE.captures(s).map(|c| c[1].to_string())
}

pub(super) fn trigger(s: &str) -> Option<String> {
    TRIGGER.captures(s).map(|c| c[1].to_string())
}

pub(super) fn event(s: &str) -> Option<String> {
    EVENT.captures(s).map(|c| c[1].to_string())
}

pub(super) fn field(s: &str) -> Option<(String, String)> {
    if !s.to_lowercase().starts_with("field(") {
        return None;
    }
    let caps = FIELD.captures(s)?;
    let inner = &caps[1];
    let name = extract_second_part(inner)?;
    let sig = format!("field({})", inner);
    Some((name, sig))
}

pub(super) fn enum_value(s: &str) -> Option<(String, String)> {
    if !s.to_lowercase().starts_with("value(") {
        return None;
    }
    let caps = VALUE.captures(s)?;
    let inner = &caps[1];
    let name = extract_second_part(inner)?;
    let sig = format!("value({})", inner);
    Some((name, sig))
}

pub(super) fn action(s: &str) -> Option<(String, String)> {
    if !s.to_lowercase().starts_with("action(") {
        return None;
    }
    let caps = ACTION.captures(s)?;
    let inner = &caps[1];
    let name = unquote(inner.trim());
    let sig = format!("action({})", inner);
    Some((name, sig))
}

pub(super) fn key(s: &str) -> Option<(String, String)> {
    if !s.to_lowercase().starts_with("key(") {
        return None;
    }
    let caps = KEY.captures(s)?;
    let inner = &caps[1];
    let name = extract_first_part(inner)?;
    let sig = format!("key({})", inner);
    Some((name, sig))
}

pub(super) fn column(s: &str) -> Option<(String, String)> {
    if !s.to_lowercase().starts_with("column(") {
        return None;
    }
    let caps = COLUMN.captures(s)?;
    let inner = &caps[1];
    let name = extract_first_part(inner)?;
    let sig = format!("column({})", inner);
    Some((name, sig))
}

pub(super) fn fieldgroup(s: &str) -> Option<(String, String)> {
    if !s.to_lowercase().starts_with("fieldgroup(") {
        return None;
    }
    let caps = FIELDGROUP.captures(s)?;
    let inner = &caps[1];
    let name = extract_first_part(inner)?;
    let sig = format!("fieldgroup({})", inner);
    Some((name, sig))
}

pub(super) fn dataitem(s: &str) -> Option<(String, String)> {
    if !s.to_lowercase().starts_with("dataitem(") {
        return None;
    }
    let caps = DATAITEM.captures(s)?;
    let inner = &caps[1];
    let name = extract_first_part(inner)?;
    let sig = format!("dataitem({})", inner);
    Some((name, sig))
}

pub(super) fn xmlport_element(s: &str) -> Option<(String, String)> {
    let caps = XMLPORT_ELEM.captures(s)?;
    let elem_type = &caps[1];
    let inner = &caps[2];
    let name = extract_first_part(inner)?;
    let sig = format!("{}({})", elem_type.to_lowercase(), inner);
    Some((name, sig))
}

pub(super) fn variable(s: &str) -> Option<(String, String)> {
    let caps = VARIABLE.captures(s)?;
    let name = caps[1].to_string();
    let type_str = caps[2].to_string();
    // Skip keywords that look like variables
    let lower = name.to_lowercase();
    if lower == "begin"
        || lower == "end"
        || lower == "var"
        || lower == "if"
        || lower == "then"
        || lower == "else"
        || lower == "exit"
        || lower == "caption"
        || lower == "clustered"
        || lower == "sourcetable"
    {
        return None;
    }
    let sig = format!("{}: {}", name, type_str);
    Some((name, sig))
}

/// Extract the name from "N; Name" -> "Name" (second semicolon-separated part)
fn extract_second_part(inner: &str) -> Option<String> {
    let parts: Vec<&str> = inner.splitn(3, ';').collect();
    if parts.len() >= 2 {
        Some(unquote(parts[1].trim()))
    } else {
        None
    }
}

/// Extract the name from "Name; ..." -> "Name" (first semicolon-separated part)
fn extract_first_part(inner: &str) -> Option<String> {
    let parts: Vec<&str> = inner.splitn(2, ';').collect();
    if !parts.is_empty() {
        Some(unquote(parts[0].trim()))
    } else {
        None
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}
