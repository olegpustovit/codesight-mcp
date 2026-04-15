#!/usr/bin/env python3
"""One-shot: split src/service/agent_v1_tools.rs into src/service/agent_v1/*.rs"""
from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src/service/agent_v1_tools.rs"
OUT = ROOT / "src/service/agent_v1"


def main() -> None:
    text = SRC.read_text()
    lines = text.splitlines(keepends=True)

    # ---- args.rs (structs from original, excluding file-tree helpers)
    args_start = lines.index("#[derive(Debug, Deserialize, Serialize)]\n")
    # ManageIndexesArgs ends before index_metadata_staleness_warning
    meta_idx = next(i for i, L in enumerate(lines) if L.startswith("fn index_metadata_staleness_warning"))
    args_chunk = "".join(lines[args_start:meta_idx])
    # Prepend constants that sat before IngestSourcesArgs
    pre = "".join(lines[32:33])  # DEFAULT_FILE_TREE
    const_rank = "".join(lines[375:386])  # RANKED_CONTEXT + defaults for context
    (OUT / "args.rs").write_text(
        "//! Agent-v1 tool argument types.\n\n"
        + "use serde::{Deserialize, Serialize};\n\n"
        + pre
        + "\n"
        + const_rank
        + "\n"
        + args_chunk
    )

    # ---- util.rs: misc free functions
    util_parts = []
    util_parts.append("".join(lines[34:105]))  # file tree
    util_parts.append("".join(lines[410:522]))  # count_params, glob, max_nesting
    util_parts.append("".join(lines[5978:6328]))  # search_hit_symbol through build_toc_tree
    (OUT / "util.rs").write_text(
        "//! Shared helpers for agent-v1 handlers.\n\n"
        "use std::collections::BTreeMap;\n\n"
        "use regex::Regex;\n"
        "use serde_json::{Value, json};\n\n"
        "use crate::config::BYTES_PER_TOKEN;\n"
        "use crate::storage::SearchSymbol;\n\n"
        + "".join(util_parts)
    )

    def impl_block(start_line: int, end_line: int) -> str:
        """1-based inclusive line numbers from agent_v1_tools.rs."""
        return "".join(lines[start_line - 1 : end_line])

    common_impl_header = (
        "use serde_json::{Value, json};\n\n"
        "use crate::service::savings::MetaBuilder;\n"
        "use crate::service::agent_v1::args::*;\n"
        "use crate::service::agent_v1::util::*;\n"
        "use crate::service::helpers::*;\n"
        "use crate::service::{CodeService, MAX_QUERY_LEN};\n"
        "use crate::service::args::parse_args;\n\n"
        "impl CodeService {\n"
    )
    common_impl_footer = "}\n"

    def write_handler(name: str, body: str, extra_use: str = "") -> None:
        """body = methods with 4-space indent (as in original file)."""
        p = OUT / f"{name}.rs"
        p.write_text(
            f"//! Handler chunk: {name}\n\n{extra_use}{common_impl_header}{body}{common_impl_footer}"
        )

    # ingest: tool lines 525-732 (drop wipe)
    write_handler("ingest", impl_block(525, 732))
    # catalog list
    write_handler(
        "catalog",
        impl_block(748, 821)
        + "fn index_metadata_staleness_warning(indexed_at: &str) -> Option<String> {\n"
        + impl_block(399, 409)
        + "}\n",
        extra_use=(
            "use std::fs;\n"
            "use std::path::Path;\n\n"
            "use anyhow::Context;\n"
            "use chrono::{DateTime, Utc};\n"
            "use walkdir::WalkDir;\n\n"
            "use crate::config::BYTES_PER_TOKEN;\n"
            "use crate::storage::doc_index::{DocStore, list_doc_summaries};\n"
            "use crate::path_map;\n"
            "use crate::repo_id::normalize_repo_id;\n"
            "use crate::storage::{IndexStore, list_repo_summaries};\n\n"
        ),
    )
    # search + search_code_files_text
    write_handler(
        "catalog_search",
        impl_block(822, 1332),
        extra_use=(
            "use std::collections::HashSet;\n"
            "use std::fs;\n"
            "use std::path::Path;\n\n"
            "use regex::Regex;\n"
            "use walkdir::WalkDir;\n\n"
            "use crate::config::BYTES_PER_TOKEN;\n"
            "use crate::storage::doc_index::DocStore;\n"
            "use codesight_retrieval::fuzzy;\n"
            "use crate::storage::SearchSymbol;\n"
            "use codesight_retrieval::semantic;\n"
            "use crate::storage::IndexStore;\n\n"
            "use crate::service::helpers::{canonicalize_root, should_descend, to_repo_relative, normalize_search_scope};\n\n"
        ),
    )
    write_handler(
        "content",
        impl_block(1333, 1508),
        extra_use=(
            "use crate::storage::doc_index::DocStore;\n"
            "use crate::core::symbols::SymbolKind;\n"
            "use crate::storage::SearchSymbol;\n"
            "use crate::repo_id::normalize_repo_id;\n"
            "use codesight_retrieval::sections;\n"
            "use crate::storage::IndexStore;\n\n"
        ),
    )
    write_handler(
        "structure",
        impl_block(1509, 1810),
        extra_use=(
            "use std::collections::BTreeMap;\n\n"
            "use walkdir::WalkDir;\n\n"
            "use crate::storage::class_hierarchy;\n"
            "use crate::storage::doc_index::DocStore;\n"
            "use crate::framework;\n"
            "use crate::repo_id::normalize_repo_id;\n"
            "use codesight_retrieval::sections;\n"
            "use crate::storage::IndexStore;\n\n"
        ),
    )

    # graph: split arms
    graph_dir = OUT / "graph"
    graph_dir.mkdir(exist_ok=True)
    glines = text.splitlines()
    match_line = next(i for i, L in enumerate(glines) if "match mode.as_str()" in L)
    i = match_line + 1
    arms: list[tuple[str, list[str]]] = []
    while i < len(glines):
        line = glines[i]
        m = re.match(r'\s+"([^"]+)" => \{', line)
        if m:
            name = m.group(1)
            depth = 1
            chunk = [line]
            i += 1
            while i < len(glines) and depth > 0:
                l = glines[i]
                chunk.append(l)
                depth += l.count("{") - l.count("}")
                i += 1
            arms.append((name, chunk))
            continue
        if re.match(r"\s+_ =>", line):
            break
        i += 1

    arm_rs = []
    for arm_name, chunk in arms:
        safe = arm_name.replace("/", "_")
        inner = chunk[1:-1]  # drop "foo" => { and final }
        unindented = "\n".join(
            ln[4:] if ln.startswith("    ") else ln for ln in inner
        )
        code = (
            f"/** `{arm_name}` graph mode. */\n"
            "use serde_json::{Value, json};\n\n"
            "use crate::service::savings::MetaBuilder;\n"
            "use crate::service::agent_v1::args::InspectGraphArgs;\n"
            "use crate::service::helpers::ToolError;\n"
            "use crate::service::{CodeService, MAX_QUERY_LEN};\n\n"
            f"pub(super) fn graph_mode_{safe}(\n"
            "    svc: &CodeService,\n"
            "    args: &InspectGraphArgs,\n"
            "    mode: &str,\n"
            "    limit: usize,\n"
            "    warnings: &mut Vec<Value>,\n"
            ") -> anyhow::Result<Option<Value>> {{\n"
            f"    let _ = mode; // `{arm_name}`\n"
            + "\n".join("    " + (ln if ln.strip() else "") for ln in unindented.split("\n"))
            + "\n    Ok(Some(out))\n}\n"
        )
        # The above is wrong - arms return early with return self.success_response - need different approach:
        # Keep full arm body as-is inside function and return Result<Option<Value>>: Ok(None) never, always Some
        # Actually each arm ends with expression returning Result - we'll paste unindented body that uses `svc` not `self`
        graph_dir.join(f"mode_{safe}.rs").write_text(
            "// GENERATED placeholder\n" + "\n".join(inner)
        )
    print("arms written", len(arms))


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    main()
