// Single integration-test binary. Each sibling tests/*.rs is pulled in as a
// module rather than compiled as its own binary — see the `autotests = false`
// + `[[test]] name = "integration"` block in the root Cargo.toml.
//
// Adding a new integration test:
//   1. Create tests/my_feature.rs with your `#[test]` / `#[tokio::test]` fns.
//   2. Add one line here:  #[path = "my_feature.rs"] mod my_feature;
//
// Nextest output still addresses each test individually, e.g.
//   integration::blast_radius::it_computes_fan_out

#[path = "agent_v1_dense.rs"]
mod agent_v1_dense;
#[path = "agent_v1_envelope.rs"]
mod agent_v1_envelope;
#[path = "agent_v1_framework_core.rs"]
mod agent_v1_framework_core;
#[path = "agent_v1_framework_dbt.rs"]
mod agent_v1_framework_dbt;
#[path = "agent_v1_framework_laravel.rs"]
mod agent_v1_framework_laravel;
#[path = "agent_v1_framework_nextjs.rs"]
mod agent_v1_framework_nextjs;
#[path = "agent_v1_framework_nuxt.rs"]
mod agent_v1_framework_nuxt;
#[path = "agent_v1_input_validation.rs"]
mod agent_v1_input_validation;
#[path = "agent_v1_staleness.rs"]
mod agent_v1_staleness;
#[path = "analysis_health.rs"]
mod analysis_health;
#[path = "catalog_search_defaults.rs"]
mod catalog_search_defaults;
#[path = "crate_graph.rs"]
mod crate_graph;
#[path = "doc_fixtures_present.rs"]
mod doc_fixtures_present;
#[path = "doc_parser_jupyter.rs"]
mod doc_parser_jupyter;
#[path = "envelope_shape.rs"]
mod envelope_shape;
#[path = "extra_extensions.rs"]
mod extra_extensions;
#[path = "extraction_tiers.rs"]
mod extraction_tiers;
#[path = "extractor_registry.rs"]
mod extractor_registry;
#[path = "file_summaries.rs"]
mod file_summaries;
#[path = "find_dead_code.rs"]
mod find_dead_code;
#[path = "find_importers.rs"]
mod find_importers;
#[path = "fuzzy_search.rs"]
mod fuzzy_search;
#[path = "fuzzy_two_lane.rs"]
mod fuzzy_two_lane;
#[path = "hooks_cli.rs"]
mod hooks_cli;
#[path = "hotspots.rs"]
mod hotspots;
#[path = "import_rendering.rs"]
mod import_rendering;
#[path = "incremental.rs"]
mod incremental;
#[path = "index_file.rs"]
mod index_file;
#[path = "packages_walker.rs"]
mod packages_walker;
#[path = "parse_warnings.rs"]
mod parse_warnings;
#[path = "path_map.rs"]
mod path_map;
#[path = "quality_signal.rs"]
mod quality_signal;
#[path = "query_pack_catalog.rs"]
mod query_pack_catalog;
#[path = "read_content_mitigation.rs"]
mod read_content_mitigation;
#[path = "read_content_next_step_hint.rs"]
mod read_content_next_step_hint;
#[path = "read_content_oversized.rs"]
mod read_content_oversized;
#[path = "reliability_probe.rs"]
mod reliability_probe;
#[path = "repo_health.rs"]
mod repo_health;
#[path = "routing_drift.rs"]
mod routing_drift;
#[path = "search_catalog_offset.rs"]
mod search_catalog_offset;
#[path = "search_catalog_savings.rs"]
mod search_catalog_savings;
#[path = "search_perf.rs"]
mod search_perf;
#[path = "search_scope_exclusion.rs"]
mod search_scope_exclusion;
#[path = "self_dogfood.rs"]
mod self_dogfood;
#[path = "session_stats.rs"]
mod session_stats;
#[path = "stdio_pipeline.rs"]
mod stdio_pipeline;
#[path = "symbol_body_in_search.rs"]
mod symbol_body_in_search;
#[path = "symbol_not_found_in_code.rs"]
mod symbol_not_found_in_code;
#[path = "test_tools_index.rs"]
mod test_tools_index;
#[path = "tokens_saved_audit.rs"]
mod tokens_saved_audit;
#[path = "truncated_flag.rs"]
mod truncated_flag;
