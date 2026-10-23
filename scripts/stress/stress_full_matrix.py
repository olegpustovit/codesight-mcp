#!/usr/bin/env python3
"""
Full tool × enum stress matrix for dowser-mcp. Covers every
evaluate_quality analysis, every inspect_graph type, every
inspect_structure view, every assemble_context mode, every
evaluate_impact analysis, and every manage_indexes action we can
safely invoke without side-effects (invalidate/delete/rebuild are
excluded).

Designed to surface:
  1. Any tool that approaches/exceeds the 2000ms handler cap.
  2. Tools with high variance (cold vs. warm cache p50 vs. p95).
  3. N+1 patterns that show up as latency scaling with the size of
     input (many refs, many file_paths).

Shares the McpClient with stress_slow_tools.py via copy (kept self-
contained so the harness can be vendored into any repo).

Run:
    python3 scripts/stress/stress_full_matrix.py [--repeat N] [--soft-deadline-ms 1500]

Exit non-zero on any tool past the soft deadline or any HUNG case.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import subprocess
import sys
import threading
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Optional

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BIN = REPO_ROOT / "target" / "release" / "dowser-mcp"


class McpClient:
    def __init__(self, bin_path: Path, workspace: Path):
        env = os.environ.copy()
        env.setdefault("DOWSER_LOG_FORMAT", "json")
        env.setdefault("RUST_LOG", "warn,dowser_mcp=warn")
        self.proc = subprocess.Popen(
            [str(bin_path), "serve"],
            cwd=str(workspace),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            bufsize=0,
        )
        self._lock = threading.Lock()
        self._pending: dict[int, queue.Queue[dict[str, Any]]] = {}
        self._next_id = 1
        self._stderr_buf: list[str] = []
        threading.Thread(target=self._read_loop, daemon=True).start()
        threading.Thread(target=self._err_loop, daemon=True).start()

    def _read_loop(self) -> None:
        assert self.proc.stdout is not None
        for raw in self.proc.stdout:
            line = raw.decode("utf-8", errors="replace").strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            req_id = msg.get("id")
            if isinstance(req_id, int):
                with self._lock:
                    q = self._pending.pop(req_id, None)
                if q is not None:
                    q.put(msg)

    def _err_loop(self) -> None:
        assert self.proc.stderr is not None
        for raw in self.proc.stderr:
            self._stderr_buf.append(raw.decode("utf-8", errors="replace"))

    def call(self, method: str, params: dict[str, Any], *, timeout_s: float):
        with self._lock:
            req_id = self._next_id
            self._next_id += 1
            reply_q: queue.Queue[dict[str, Any]] = queue.Queue(maxsize=1)
            self._pending[req_id] = reply_q
        payload = {"jsonrpc": "2.0", "id": req_id, "method": method, "params": params}
        line = (json.dumps(payload) + "\n").encode("utf-8")
        start = time.monotonic()
        try:
            assert self.proc.stdin is not None
            self.proc.stdin.write(line)
            self.proc.stdin.flush()
        except (BrokenPipeError, OSError) as e:
            return {"error": {"message": f"broken pipe: {e}"}}, time.monotonic() - start, True
        try:
            msg = reply_q.get(timeout=timeout_s)
            return msg, time.monotonic() - start, False
        except queue.Empty:
            with self._lock:
                self._pending.pop(req_id, None)
            return None, time.monotonic() - start, True

    def call_tool(self, tool: str, arguments: dict[str, Any], *, timeout_s: float):
        return self.call("tools/call", {"name": tool, "arguments": arguments}, timeout_s=timeout_s)

    def close(self) -> None:
        try:
            if self.proc.stdin:
                self.proc.stdin.close()
        except Exception:
            pass
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()

    def stderr_tail(self, n: int = 80) -> str:
        return "".join(self._stderr_buf[-n:])


@dataclass
class Case:
    name: str
    tool: str
    args: dict[str, Any]
    note: str = ""


@dataclass
class CaseResult:
    name: str
    tool: str
    args: dict[str, Any]
    elapsed_ms: float
    hung: bool
    soft_deadline_ms: int
    soft_deadline_exceeded: bool
    handler_budget_exceeded: bool
    response_ok: Optional[bool]
    envelope_error: Optional[dict[str, Any]]
    envelope_meta: Optional[dict[str, Any]]
    warnings: list[Any] = field(default_factory=list)
    note: str = ""


# Real files/symbols in this repo, used as test refs.
HIGH_FANIN_FILE = "src/service/agent_v1/util.rs"
HOT_SYMBOL = "src/service/agent_v1/catalog_search/mod.rs::CodeService.search_catalog#function"
MANY_FILES_SAMPLE = [
    "src/service/agent_v1/util.rs",
    "src/service/agent_v1/catalog_search/mod.rs",
    "src/service/agent_v1/index_admin.rs",
    "src/service/agent_v1/content.rs",
    "src/service/agent_v1/impact_more.rs",
    "src/service/agent_v1/quality_more.rs",
    "src/service/agent_v1/ingest.rs",
    "src/service/agent_v1/freshness.rs",
    "src/indexing/code_index.rs",
    "src/indexing/doc_index.rs",
    "src/mcp.rs",
    "crates/dowser-storage/src/storage/index_store.rs",
    "crates/dowser-storage/src/storage/analysis_reads.rs",
    "crates/dowser-storage/src/storage/symbol_reads.rs",
    "crates/dowser-storage/src/storage/graph_reads.rs",
    "crates/dowser-analysis/src/lib.rs",
    "crates/dowser-extractors/src/lib.rs",
    "crates/dowser-retrieval/src/lib.rs",
    "crates/dowser-core/src/lib.rs",
    "crates/dowser-framework/src/lib.rs",
]
MANY_REFS_SYMBOLS = [
    {"kind": "symbol", "stable_id": f"src/service/agent_v1/util.rs::{name}#function"}
    for name in [
        "round_score",
        "search_hit_symbol",
        "search_hit_section",
        "token_estimate_for_context",
        "pack_catalog_items_to_token_budget",
        "resolve_symbol_id",
        "err_item",
        "doc_format_to_type_label",
        "truncate_content_to_token_budget",
        "heading_path_ancestors",
        "direct_child_sections",
        "outline_neighbors",
    ]
]


def build_cases(S: str) -> list[Case]:
    return [
        # ---------- catalog_sources / manage_indexes (safe actions) ----------
        Case("catalog_sources", "catalog_sources", {}),
        Case("mi_session_stats", "manage_indexes", {"action": "session_stats", "source_id": S}),
        Case("mi_status", "manage_indexes", {"action": "status", "source_id": S}),
        Case(
            "mi_snapshot_quality",
            "manage_indexes",
            {"action": "snapshot_quality", "source_id": S, "snapshot_name": "stress_harness_tmp"},
        ),
        # ---------- inspect_structure: every view ----------
        Case(
            "is_repo_outline",
            "inspect_structure",
            {"source_id": S, "view": "repo_outline", "max_tokens": 2000},
        ),
        Case("is_file_tree", "inspect_structure", {"source_id": S, "view": "file_tree"}),
        Case(
            "is_repository_rollup",
            "inspect_structure",
            {"source_id": S, "view": "repository_rollup"},
        ),
        Case(
            "is_file_outline_single",
            "inspect_structure",
            {"source_id": S, "view": "file_outline", "file_path": HIGH_FANIN_FILE},
        ),
        Case(
            "is_file_outline_batch_5",
            "inspect_structure",
            {"source_id": S, "view": "file_outline", "file_paths": MANY_FILES_SAMPLE[:5]},
        ),
        Case(
            "is_file_outline_batch_20",
            "inspect_structure",
            {"source_id": S, "view": "file_outline", "file_paths": MANY_FILES_SAMPLE},
            note="Batch outline of 20 files — regression canary for store.file_outline per path.",
        ),
        Case(
            "is_file_outline_batch_20_repeat_5x",
            "inspect_structure",
            {"source_id": S, "view": "file_outline",
             "file_paths": MANY_FILES_SAMPLE + MANY_FILES_SAMPLE + MANY_FILES_SAMPLE
                          + MANY_FILES_SAMPLE + MANY_FILES_SAMPLE},
            note="100-path batch: catches 2N→2 fan-out regressions.",
        ),
        Case(
            "is_toc",
            "inspect_structure",
            {"source_id": S, "view": "toc", "file_path": "AGENTS.md"},
        ),
        Case(
            "is_toc_tree",
            "inspect_structure",
            {"source_id": S, "view": "toc_tree", "file_path": "AGENTS.md"},
        ),
        Case(
            "is_document_outline",
            "inspect_structure",
            {"source_id": S, "view": "document_outline", "file_path": "AGENTS.md"},
        ),
        Case(
            "is_class_hierarchy",
            "inspect_structure",
            {"source_id": S, "view": "class_hierarchy", "symbol_id": HOT_SYMBOL},
        ),
        # ---------- inspect_graph: every type ----------
        Case(
            "ig_references",
            "inspect_graph",
            {"source_id": S, "graph": "references", "ref": {"kind": "symbol", "stable_id": HOT_SYMBOL}},
        ),
        Case(
            "ig_related",
            "inspect_graph",
            {"source_id": S, "graph": "related", "ref": {"kind": "symbol", "stable_id": HOT_SYMBOL}},
        ),
        Case(
            "ig_imports",
            "inspect_graph",
            {"source_id": S, "graph": "imports", "file_path": HIGH_FANIN_FILE},
        ),
        Case(
            "ig_importers_high_fanin",
            "inspect_graph",
            {"source_id": S, "graph": "importers", "file_path": HIGH_FANIN_FILE},
            note="High-fan-in file; stresses per-importer file_outline N+1 path.",
        ),
        Case(
            "ig_calls",
            "inspect_graph",
            {"source_id": S, "graph": "calls", "symbol_id": HOT_SYMBOL},
        ),
        Case(
            "ig_dependencies",
            "inspect_graph",
            {"source_id": S, "graph": "dependencies", "max_tokens": 3000},
        ),
        Case(
            "ig_dependency_cycles",
            "inspect_graph",
            {"source_id": S, "graph": "dependency_cycles"},
        ),
        Case(
            "ig_centrality_pagerank",
            "inspect_graph",
            {"source_id": S, "graph": "centrality", "algorithm": "pagerank", "top_n": 20},
        ),
        Case(
            "ig_module_coupling",
            "inspect_graph",
            {
                "source_id": S,
                "graph": "module_coupling",
                "module_path": "src/service/agent_v1/catalog_search/mod.rs",
            },
        ),
        Case(
            "ig_layer_violations",
            "inspect_graph",
            {"source_id": S, "graph": "layer_violations"},
        ),
        Case(
            "ig_find_references",
            "inspect_graph",
            {"source_id": S, "graph": "find_references", "identifier": "search_catalog"},
        ),
        Case(
            "ig_resolve_fqn",
            "inspect_graph",
            {"source_id": S, "graph": "resolve_fqn", "name": "CodeService"},
        ),
        # ---------- evaluate_impact: every analysis ----------
        Case(
            "ei_blast_radius_hot",
            "evaluate_impact",
            {
                "source_id": S,
                "analysis": "blast_radius",
                "refs": [{"kind": "symbol", "stable_id": HOT_SYMBOL}],
            },
            note="High-fan-in symbol → N+1 over importers (file_outline per importer).",
        ),
        Case(
            "ei_blast_radius_many_refs",
            "evaluate_impact",
            {"source_id": S, "analysis": "blast_radius", "refs": MANY_REFS_SYMBOLS[:8]},
            note="Batch blast_radius over 8 symbols — compounds importer walks.",
        ),
        Case(
            "ei_impact_preview",
            "evaluate_impact",
            {
                "source_id": S,
                "analysis": "impact_preview",
                "refs": [{"kind": "symbol", "stable_id": HOT_SYMBOL}],
            },
        ),
        Case(
            "ei_rename_safety",
            "evaluate_impact",
            {
                "source_id": S,
                "analysis": "rename_safety",
                "refs": [{"kind": "symbol", "stable_id": HOT_SYMBOL, "new_name": "do_search"}],
            },
        ),
        Case(
            "ei_changed_units",
            "evaluate_impact",
            {"source_id": S, "analysis": "changed_units"},
        ),
        Case(
            "ei_change_velocity",
            "evaluate_impact",
            {"source_id": S, "analysis": "change_velocity"},
        ),
        # ---------- evaluate_quality: every analysis ----------
        Case(
            "eq_quality_signal",
            "evaluate_quality",
            {"source_id": S, "analysis": "quality_signal"},
        ),
        Case("eq_hotspots", "evaluate_quality",
             {"source_id": S, "analysis": "hotspots", "top_n": 20}),
        Case(
            "eq_dead_code",
            "evaluate_quality",
            {"source_id": S, "analysis": "dead_code", "granularity": "symbol", "max_tokens": 3000},
            note="Suspected N+1 on symbol_reference_count per symbol.",
        ),
        Case(
            "eq_dead_code_file",
            "evaluate_quality",
            {"source_id": S, "analysis": "dead_code", "granularity": "file", "max_tokens": 3000},
        ),
        Case(
            "eq_test_gaps",
            "evaluate_quality",
            {"source_id": S, "analysis": "test_gaps", "top_n": 25, "max_tokens": 3000},
        ),
        Case(
            "eq_refactor_targets_default",
            "evaluate_quality",
            {"source_id": S, "analysis": "refactor_targets"},
        ),
        Case(
            "eq_refactor_targets_top30",
            "evaluate_quality",
            {"source_id": S, "analysis": "refactor_targets", "top_files": 30},
            note="Previously TIMED OUT; now a regression canary.",
        ),
        Case("eq_repo_health", "evaluate_quality", {"source_id": S, "analysis": "repo_health"}),
        Case(
            "eq_doc_coverage",
            "evaluate_quality",
            {"source_id": S, "analysis": "doc_coverage"},
        ),
        Case(
            "eq_broken_links",
            "evaluate_quality",
            {"source_id": S, "analysis": "broken_links"},
        ),
        Case(
            "eq_complexity_repo",
            "evaluate_quality",
            {"source_id": S, "analysis": "complexity"},
        ),
        Case(
            "eq_regression_check",
            "evaluate_quality",
            {"source_id": S, "analysis": "regression_check", "snapshot_name": "pre_improvement"},
        ),
        # ---------- assemble_context: every mode ----------
        Case(
            "ac_bundle_1_sym",
            "assemble_context",
            {
                "source_id": S,
                "mode": "bundle",
                "refs": [{"kind": "symbol", "stable_id": HOT_SYMBOL}],
                "max_tokens": 4000,
            },
        ),
        Case(
            "ac_context_bundle_1_sym",
            "assemble_context",
            {
                "source_id": S,
                "mode": "context_bundle",
                "refs": [{"kind": "symbol", "stable_id": HOT_SYMBOL}],
                "max_tokens": 4000,
            },
            note="Suspected N+1 inside gather: 4 DB calls per ref.",
        ),
        Case(
            "ac_context_bundle_8_syms",
            "assemble_context",
            {
                "source_id": S,
                "mode": "context_bundle",
                "refs": MANY_REFS_SYMBOLS[:8],
                "max_tokens": 8000,
            },
            note="Regression canary: was 4×8 = 32 DB calls before batching.",
        ),
        Case(
            "ac_context_bundle_12_syms",
            "assemble_context",
            {
                "source_id": S,
                "mode": "context_bundle",
                "refs": MANY_REFS_SYMBOLS,
                "max_tokens": 12000,
            },
            note="Largest bundle we'd ship: 12 refs × 4 DB calls = 48 trips before fix.",
        ),
        Case(
            "ac_section_context",
            "assemble_context",
            {
                "source_id": S,
                "mode": "section_context",
                "refs": [{"kind": "section", "path": "AGENTS.md", "title": "Layout"}],
                "max_tokens": 4000,
            },
        ),
        Case(
            "ac_ranked",
            "assemble_context",
            {"source_id": S, "mode": "ranked", "query": "how does search_catalog work", "max_tokens": 4000},
        ),
        Case(
            "ac_query_suggestions",
            "assemble_context",
            {"source_id": S, "mode": "query_suggestions"},
        ),
        Case(
            "ac_rename_plan",
            "assemble_context",
            {
                "source_id": S,
                "mode": "rename_plan",
                "refs": [{"kind": "symbol", "stable_id": HOT_SYMBOL, "new_name": "do_search"}],
            },
        ),
        # ---------- read_content: batch sanity ----------
        Case(
            "rc_batch_12_symbols",
            "read_content",
            {"source_id": S, "refs": MANY_REFS_SYMBOLS},
            note="12 small symbols; should amortize across one DB connection.",
        ),
        # ---------- Cohort C: cross-source scaling ----------
        # Fans out over every registered source on this machine. Warm path
        # should be ≤ 200 ms once the SourceRegistry is populated; cold path
        # trades an extra SELECT indexed_at per source for correctness
        # against external reindexes.
        Case(
            "cs_catalog_sources_all",
            "catalog_sources",
            {},
            note="Cohort C: per-source repo_summary + root_path via SourceRegistry snapshot.",
        ),
    ]


def run_case(client: McpClient, case: Case, soft_deadline_ms: int) -> CaseResult:
    wire_timeout_s = max(soft_deadline_ms / 1000.0, 2.5) + 15.0
    msg, elapsed, hung = client.call_tool(case.tool, case.args, timeout_s=wire_timeout_s)
    elapsed_ms = elapsed * 1000.0
    result = CaseResult(
        name=case.name,
        tool=case.tool,
        args=case.args,
        elapsed_ms=round(elapsed_ms, 1),
        hung=hung,
        soft_deadline_ms=soft_deadline_ms,
        soft_deadline_exceeded=elapsed_ms > soft_deadline_ms,
        handler_budget_exceeded=elapsed_ms > 2000.0,
        response_ok=None,
        envelope_error=None,
        envelope_meta=None,
        note=case.note,
    )
    if msg is None:
        return result
    result_field = msg.get("result")
    if result_field is None:
        err = msg.get("error")
        if isinstance(err, dict):
            result.envelope_error = err
        return result
    content = result_field.get("content") if isinstance(result_field, dict) else None
    if not isinstance(content, list) or not content:
        return result
    text = content[0].get("text")
    if not isinstance(text, str):
        return result
    try:
        env = json.loads(text)
    except json.JSONDecodeError:
        return result
    if isinstance(env, dict):
        result.response_ok = env.get("ok")
        result.envelope_meta = env.get("_meta") or env.get("meta")
        result.warnings = env.get("warnings") or []
        if env.get("error"):
            result.envelope_error = env["error"]
    return result


def _tag(r: CaseResult) -> str:
    if r.hung:
        return "HUNG"
    if r.handler_budget_exceeded:
        return "TIMEOUT"
    if r.soft_deadline_exceeded:
        return "SLOW"
    if r.envelope_error:
        return "ERR"
    if r.response_ok is False:
        return "FAIL"
    return "OK"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=str(DEFAULT_BIN))
    ap.add_argument("--workspace", default=str(REPO_ROOT))
    ap.add_argument("--source", default="olegpustovit/dowser-mcp")
    ap.add_argument("--soft-deadline-ms", type=int, default=1500,
                    help="Warn below the 2000ms handler cap.")
    ap.add_argument("--repeat", type=int, default=3)
    ap.add_argument("--only", default="")
    ap.add_argument("--out", default=str(REPO_ROOT / "scripts/stress/stress_full_matrix_report.json"))
    args = ap.parse_args()

    bin_path = Path(args.bin)
    if not bin_path.exists():
        print(f"binary not found: {bin_path}", file=sys.stderr)
        return 2

    cases = [c for c in build_cases(args.source) if args.only in c.name]
    if not cases:
        print("no cases matched --only filter", file=sys.stderr)
        return 2

    client = McpClient(bin_path, Path(args.workspace))
    try:
        init_msg, _, init_hung = client.call(
            "initialize",
            {"protocolVersion": "2024-11-05", "capabilities": {},
             "clientInfo": {"name": "stress_full", "version": "0.0.0"}},
            timeout_s=10.0,
        )
        if init_hung:
            print("initialize hung; aborting", file=sys.stderr)
            return 3

        expanded: list[Case] = []
        for c in cases:
            for i in range(args.repeat):
                label = c.name if args.repeat == 1 else f"{c.name}#{i+1}"
                expanded.append(Case(label, c.tool, dict(c.args), c.note))

        results: list[CaseResult] = []
        for c in expanded:
            r = run_case(client, c, args.soft_deadline_ms)
            print(f"{_tag(r):>7} {r.name:<50} {r.elapsed_ms:>8.1f}ms")
            results.append(r)

        # per-logical-case stats
        groups: dict[str, list[float]] = {}
        for r in results:
            base = r.name.split("#")[0]
            groups.setdefault(base, []).append(r.elapsed_ms)
        print()
        print(f"{'case':<50} {'min':>8} {'p50':>8} {'p95':>8} {'max':>8}")
        # Sort by p95 desc to spotlight slow tools
        rows = []
        for base, xs in groups.items():
            xs_sorted = sorted(xs)
            mn = xs_sorted[0]
            mx = xs_sorted[-1]
            p50 = xs_sorted[len(xs_sorted) // 2]
            p95 = xs_sorted[int(0.95 * (len(xs_sorted) - 1))] if len(xs_sorted) > 1 else xs_sorted[0]
            rows.append((base, mn, p50, p95, mx))
        rows.sort(key=lambda r: -r[3])  # desc p95
        for base, mn, p50, p95, mx in rows:
            marker = " !!" if p95 > 1500 else ("  !" if p95 > 500 else "   ")
            print(f"{base:<50} {mn:>8.1f} {p50:>8.1f} {p95:>8.1f} {mx:>8.1f} {marker}")

        report = {
            "binary": str(bin_path),
            "source_id": args.source,
            "soft_deadline_ms": args.soft_deadline_ms,
            "repeat": args.repeat,
            "cases": [asdict(r) for r in results],
            "stderr_tail": client.stderr_tail(),
        }
        Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        Path(args.out).write_text(json.dumps(report, indent=2))
        print(f"\nreport: {args.out}")

        fails = [r for r in results if r.hung or r.handler_budget_exceeded or r.soft_deadline_exceeded]
        return 1 if fails else 0
    finally:
        client.close()


if __name__ == "__main__":
    sys.exit(main())
