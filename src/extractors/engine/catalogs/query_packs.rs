use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::adaptive_languages::project_query_pack_roots;
use crate::extractors::engine::scm::{ScmMetadata, parse_scm_header};

include!(concat!(env!("OUT_DIR"), "/builtin_query_packs.rs"));

#[derive(Clone)]
pub struct QueryPackDefinition {
    pub pack_id: String,
    pub query_source: String,
    pub injections_source: Option<String>,
    pub metadata: ScmMetadata,
}

pub struct QueryPackCatalog {
    packs: Vec<QueryPackDefinition>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryPackMetadataFile {
    language: Option<String>,
    extensions: Option<Vec<String>>,
    grammar: Option<String>,
    override_extractor: Option<String>,
}

impl QueryPackMetadataFile {
    fn apply_to(self, metadata: &mut ScmMetadata) {
        if let Some(language) = self.language {
            metadata.name = language;
        }
        if let Some(extensions) = self.extensions {
            metadata.extensions = extensions;
        }
        if let Some(grammar) = self.grammar {
            metadata.grammar = grammar;
        }
        if let Some(override_extractor) = self.override_extractor {
            metadata.override_extractor = Some(override_extractor);
        }
    }
}

impl QueryPackCatalog {
    pub fn builtin() -> Self {
        let packs = BUILTIN_QUERY_PACKS
            .iter()
            .map(
                |(pack_id, query_source, injections_source, metadata_source)| {
                    Self::build_definition(
                        pack_id,
                        query_source.to_string(),
                        injections_source.map(ToString::to_string),
                        metadata_source.map(ToString::to_string),
                    )
                },
            )
            .collect::<Result<Vec<_>>>()
            .expect("invalid built-in query pack metadata");
        Self { packs }
    }

    pub fn for_workspace_root(project_root: &Path) -> Result<Self> {
        let mut catalog = Self::builtin();
        let overlay = Self::from_roots(&project_query_pack_roots(project_root))?;
        catalog.overlay(overlay);
        Ok(catalog)
    }

    pub fn from_roots(roots: &[PathBuf]) -> Result<Self> {
        let mut packs = Vec::new();
        for root in roots {
            if !root.is_dir() {
                continue;
            }

            for entry in
                fs::read_dir(root).with_context(|| format!("read_dir {}", root.display()))?
            {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                if let Some(pack) = Self::load_pack_dir(&entry.path())? {
                    packs.push(pack);
                }
            }
        }
        packs.sort_by(|left, right| left.pack_id.cmp(&right.pack_id));
        Ok(Self { packs })
    }

    pub fn iter(&self) -> impl Iterator<Item = &QueryPackDefinition> {
        self.packs.iter()
    }

    pub fn curated_language_names(&self) -> BTreeSet<String> {
        self.packs
            .iter()
            .map(|pack| pack.metadata.name.clone())
            .collect()
    }

    pub fn find_by_pack_id(&self, pack_id: &str) -> Option<&QueryPackDefinition> {
        self.packs.iter().find(|pack| pack.pack_id == pack_id)
    }

    fn overlay(&mut self, other: Self) {
        let mut merged: BTreeMap<String, QueryPackDefinition> = self
            .packs
            .drain(..)
            .map(|pack| (pack.pack_id.clone(), pack))
            .collect();
        for pack in other.packs {
            merged.insert(pack.pack_id.clone(), pack);
        }
        self.packs = merged.into_values().collect();
    }

    fn load_pack_dir(dir: &Path) -> Result<Option<QueryPackDefinition>> {
        let query_path = dir.join("tags.scm");
        if !query_path.is_file() {
            return Ok(None);
        }

        let query_source = fs::read_to_string(&query_path)
            .with_context(|| format!("read {}", query_path.display()))?;
        let injections_path = dir.join("injections.scm");
        let injections_source = if injections_path.is_file() {
            Some(
                fs::read_to_string(&injections_path)
                    .with_context(|| format!("read {}", injections_path.display()))?,
            )
        } else {
            None
        };

        let metadata_path = dir.join("pack.toml");
        let metadata_source = if metadata_path.is_file() {
            Some(
                fs::read_to_string(&metadata_path)
                    .with_context(|| format!("read {}", metadata_path.display()))?,
            )
        } else {
            None
        };

        let pack_id = dir
            .file_name()
            .and_then(|name| name.to_str())
            .context("query pack directory missing UTF-8 name")?;

        Ok(Some(Self::build_definition(
            pack_id,
            query_source,
            injections_source,
            metadata_source,
        )?))
    }

    fn build_definition(
        pack_id: &str,
        query_source: String,
        injections_source: Option<String>,
        metadata_source: Option<String>,
    ) -> Result<QueryPackDefinition> {
        let mut metadata = parse_scm_header(&query_source);
        if let Some(metadata_source) = metadata_source {
            let file_metadata = toml::from_str::<QueryPackMetadataFile>(&metadata_source)
                .with_context(|| format!("parse pack.toml for query pack {pack_id}"))?;
            file_metadata.apply_to(&mut metadata);
        }
        if metadata.name.is_empty() {
            metadata.name = pack_id.to_string();
        }
        validate_pack_metadata(pack_id, &metadata)?;

        Ok(QueryPackDefinition {
            pack_id: pack_id.to_string(),
            query_source,
            injections_source,
            metadata,
        })
    }
}

fn validate_pack_metadata(pack_id: &str, metadata: &ScmMetadata) -> Result<()> {
    if metadata.extensions.is_empty() {
        bail!("query pack {pack_id} is missing extensions metadata");
    }
    if metadata.grammar.is_empty() && metadata.override_extractor.is_none() {
        bail!("query pack {pack_id} must declare either grammar or override_extractor");
    }
    Ok(())
}
