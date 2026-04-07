//! Shared Laravel parsing types (used by tree-sitter and regex passes).

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct MigrationInfo {
    pub table: String,
    pub columns: HashMap<String, Vec<String>>, // column_name -> modifiers
}

#[derive(Debug, Clone, Default)]
pub struct ModelInfo {
    pub relationships: Vec<Relationship>,
    pub fillable: Vec<String>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Relationship {
    pub kind: String,   // hasMany, belongsTo, etc.
    pub target: String, // Post, Team, etc.
}

#[derive(Debug, Clone)]
pub struct RouteInfo {
    pub verb: String,
    pub uri: String,
    pub controller: String,
    pub action: String,
    pub name: Option<String>,
}
