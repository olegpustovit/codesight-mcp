#!/usr/bin/env python3
"""
Stress test the dowser-mcp `search_catalog` handler (and friends) over stdio
JSON-RPC. Fires a battery of real-world + adversarial queries, records per-call
wall-clock latency, and flags anything that:

  - exceeds the documented `HANDLER_TIMEOUT_MS` (2000ms) budget, or
  - silently hangs without producing a response (classic dowser bug:
    `dispatch_with_timeout` Err is swallowed in `handle_request` so the
    client never sees a reply).

Run:
    python3 scripts/stress/stress_search.py [--bin PATH] [--source SOURCE_ID]
                                            [--out REPORT.json]
                                            [--soft-deadline-ms 5000]
                                            [--parallel N]
                                            [--only pattern]

Exit code is non-zero when any call crossed the soft deadline. Every outcome
goes into the JSON report regardless.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import select
import subprocess
import sys
import threading
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Optional

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BIN = REPO_ROOT / "target" / "release" / "dowser-mcp"


# ---------------------------------------------------------------------------
# MCP stdio client
# ---------------------------------------------------------------------------


class McpClient:
    """
    Minimal JSON-RPC-over-stdio client. Single pending request at a time is
    fine for stress testing because dowser serializes permits internally
    (DISPATCH_CONCURRENCY=4) and each request carries a unique id.
    """

    def __init__(self, bin_path: Path, workspace: Path, env_overrides: dict[str, str] | None = None):
        env = os.environ.copy()
        env.setdefault("DOWSER_LOG_FORMAT", "json")
        env.setdefault("RUST_LOG", "warn,dowser_mcp=warn")
        if env_overrides:
            env.update(env_overrides)
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
        self._alive = True
        self._stderr_buf: list[str] = []
        self._reader = threading.Thread(target=self._read_loop, daemon=True)
        self._reader.start()
        self._err_reader = threading.Thread(target=self._err_loop, daemon=True)
        self._err_reader.start()

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
        self._alive = False

    def _err_loop(self) -> None:
        assert self.proc.stderr is not None
        for raw in self.proc.stderr:
            self._stderr_buf.append(raw.decode("utf-8", errors="replace"))

    def call(self, method: str, params: dict[str, Any], *, timeout_s: float) -> tuple[Optional[dict[str, Any]], float, bool]:
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
            # Give up and tell the caller it hung. We leave the request_id
            # in _pending; if a reply eventually comes it'll be silently
            # dropped by _read_loop (q already removed).
            with self._lock:
                self._pending.pop(req_id, None)
            return None, time.monotonic() - start, True

    def call_tool(self, tool: str, arguments: dict[str, Any], *, timeout_s: float) -> tuple[Optional[dict[str, Any]], float, bool]:
        """Wraps `tool` in a `tools/call` JSON-RPC envelope."""
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

    def stderr_tail(self, n: int = 40) -> str:
        return "".join(self._stderr_buf[-n:])


# ---------------------------------------------------------------------------
# Test cases
# ---------------------------------------------------------------------------


@dataclass
class Case:
    name: str
    tool: str
    args: dict[str, Any]
    expect_ok: bool = True
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
    raw: Optional[dict[str, Any]] = None
    note: str = ""


def build_cases(source_id: str) -> list[Case]:
    S = source_id
    return [
        # -------------------- reported hang reproducer --------------------
        Case(
            "repro_docs_dowser_feedback_md",
            "search_catalog",
            {
                "source_id": S, "domain": "docs", "query": "DOWSER_FEEDBACK.md",
                "query_mode": "keyword", "limit": 10, "max_tokens": 3000,
            },
            note="Exact reproducer from user bug report (120s hang in MCP client).",
        ),
        # -------------------- tokenization corner cases -------------------
        Case(
            "docs_single_long_token", "search_catalog",
            {"source_id": S, "domain": "docs", "query": "DOWSER", "query_mode": "keyword", "limit": 10},
        ),
        Case(
            "docs_filename_with_dot", "search_catalog",
            {"source_id": S, "domain": "docs", "query": "REFACTOR_WORKLOG.md", "query_mode": "keyword", "limit": 10},
        ),
        Case(
            "docs_filename_with_slash", "search_catalog",
            {"source_id": S, "domain": "docs", "query": "src/service/agent_v1/mod.rs", "query_mode": "keyword", "limit": 5},
        ),
        Case(
            "docs_all_punctuation", "search_catalog",
            {"source_id": S, "domain": "docs", "query": ".!?@#$%^&*()", "query_mode": "keyword", "limit": 5},
            note="Tokenizer should strip to empty; fast NOOP expected.",
        ),
        Case(
            "docs_empty_query", "search_catalog",
            {"source_id": S, "domain": "docs", "query": "", "query_mode": "keyword", "limit": 5},
        ),
        Case(
            "docs_whitespace_query", "search_catalog",
            {"source_id": S, "domain": "docs", "query": "   \t\n  ", "query_mode": "keyword", "limit": 5},
        ),
        Case(
            "docs_very_long_query", "search_catalog",
            {"source_id": S, "domain": "docs", "query": "feedback " * 100, "query_mode": "keyword", "limit": 5},
            note="1000-byte query, all distinct repeated tokens.",
        ),
        Case(
            "docs_unicode_query", "search_catalog",
            {"source_id": S, "domain": "docs", "query": "café übermensch 日本語 🚀", "query_mode": "keyword", "limit": 5},
        ),
        # -------------------- very common tokens (hot path) ---------------
        Case(
            "code_common_token_the", "search_catalog",
            {"source_id": S, "domain": "code", "query": "the", "query_mode": "keyword", "limit": 10},
            note="Ultra-common token: big BM25 working set.",
        ),
        Case(
            "code_common_token_fn", "search_catalog",
            {"source_id": S, "domain": "code", "query": "fn", "query_mode": "keyword", "unit_kinds": ["symbol"], "limit": 10},
        ),
        Case(
            "code_ultra_short_1char", "search_catalog",
            {"source_id": S, "domain": "code", "query": "a", "query_mode": "keyword", "limit": 10},
        ),
        # -------------------- fuzzy path ----------------------------------
        Case(
            "code_fuzzy_typo", "search_catalog",
            {"source_id": S, "domain": "code", "query": "dispach_with_timeot", "fuzzy": True,
             "limit": 10},
        ),
        Case(
            "code_fuzzy_autotrigger", "search_catalog",
            {"source_id": S, "domain": "code", "query": "xxxyyyNoSuchSymbolZZZ", "limit": 10},
            note="Nothing matches → auto-fuzzy kicks in; shouldn't hang on empty lexical.",
        ),
        # -------------------- regex path (files) --------------------------
        Case(
            "code_regex_simple", "search_catalog",
            {"source_id": S, "domain": "code", "query": "fn\\s+dispatch", "query_mode": "regex", "limit": 10},
        ),
        Case(
            "code_regex_catastrophic", "search_catalog",
            {"source_id": S, "domain": "code", "query": "(a+)+b", "query_mode": "regex", "limit": 10},
            note="Classic catastrophic backtracking pattern; hopefully rejected or bounded.",
        ),
        Case(
            "code_regex_nested_star", "search_catalog",
            {"source_id": S, "domain": "code", "query": "(.*)*", "query_mode": "regex", "limit": 10},
            note="Unbounded nested repetition.",
        ),
        Case(
            "code_regex_invalid", "search_catalog",
            {"source_id": S, "domain": "code", "query": "[unclosed", "query_mode": "regex", "limit": 10},
        ),
        # -------------------- hybrid + semantic ---------------------------
        Case(
            "hybrid_keyword_common", "search_catalog",
            {"source_id": S, "domain": "hybrid", "query": "index", "query_mode": "keyword", "limit": 10},
        ),
        Case(
            "hybrid_nlq", "search_catalog",
            {"source_id": S, "domain": "hybrid", "query": "how does the server handle shutdown",
             "query_mode": "natural_language", "limit": 10},
            note="NLQ triggers dense lane (embedder).",
        ),
        Case(
            "code_semantic_flag", "search_catalog",
            {"source_id": S, "domain": "code", "query": "graceful teardown", "semantic": True, "limit": 10},
        ),
        # -------------------- scope / exclude clauses ---------------------
        Case(
            "code_scope_narrow", "search_catalog",
            {"source_id": S, "domain": "code", "query": "dispatch", "scope": "src/service/",
             "query_mode": "keyword", "limit": 10},
        ),
        Case(
            "code_file_pattern_glob", "search_catalog",
            {"source_id": S, "domain": "code", "query": "dispatch", "file_pattern": "*.rs",
             "query_mode": "keyword", "limit": 10},
        ),
        Case(
            "code_exclude_pattern", "search_catalog",
            {"source_id": S, "domain": "code", "query": "dispatch", "exclude": ["target/", "tests/"],
             "query_mode": "keyword", "limit": 10},
        ),
        # -------------------- unit_kinds variations -----------------------
        Case(
            "code_files_only", "search_catalog",
            {"source_id": S, "domain": "code", "query": "dispatch", "unit_kinds": ["file"],
             "query_mode": "keyword", "limit": 10},
        ),
        Case(
            "code_symbols_only", "search_catalog",
            {"source_id": S, "domain": "code", "query": "dispatch", "unit_kinds": ["symbol"],
             "query_mode": "keyword", "limit": 10},
        ),
        Case(
            "code_count_mode", "search_catalog",
            {"source_id": S, "domain": "code", "query": "dispatch", "output_mode": "count"},
        ),
        Case(
            "code_files_with_matches", "search_catalog",
            {"source_id": S, "domain": "code", "query": "dispatch", "output_mode": "files_with_matches"},
        ),
        # -------------------- high-limit stress ---------------------------
        Case(
            "code_high_limit_50", "search_catalog",
            {"source_id": S, "domain": "code", "query": "pub", "query_mode": "keyword", "limit": 50,
             "include_body": True, "context_lines": 5},
            note="Max lim=50 + bodies + context → largest response.",
        ),
        Case(
            "code_max_tokens_tight", "search_catalog",
            {"source_id": S, "domain": "code", "query": "pub", "query_mode": "keyword", "limit": 50,
             "max_tokens": 200},
            note="Tiny budget forces aggressive token-pack truncation.",
        ),
        # -------------------- auxiliary tools (sanity) --------------------
        Case("catalog_sources", "catalog_sources", {}),
        Case("inspect_structure_repo_outline", "inspect_structure",
             {"source_id": S, "view": "repo_outline", "max_tokens": 2000}),
        Case("inspect_structure_file_tree", "inspect_structure",
             {"source_id": S, "view": "file_tree"}),
    ]


# ---------------------------------------------------------------------------
# Runner
# ---------------------------------------------------------------------------


def run_case(client: McpClient, case: Case, soft_deadline_ms: int) -> CaseResult:
    deadline_s = max(soft_deadline_ms / 1000.0, 0.5)
    # Give the wire a bit of grace over the soft deadline, so we distinguish
    # "handler exceeded budget but server did reply" from "server never replied".
    wire_timeout_s = deadline_s + 5.0
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
    result.raw = msg
    # tools/call wraps the envelope inside result.content[0].text
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


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=str(DEFAULT_BIN), help="Path to dowser-mcp binary.")
    ap.add_argument("--workspace", default=str(REPO_ROOT),
                    help="Workspace the server runs inside (cwd + .dowser.toml lookup).")
    ap.add_argument("--source", default="OlegHQ/dowser-mcp",
                    help="source_id to hammer.")
    ap.add_argument("--soft-deadline-ms", type=int, default=5000,
                    help="Any call slower than this counts as a failure.")
    ap.add_argument("--wire-warmup", action="store_true",
                    help="Skip initialize handshake (server still handles it, just skip our own wait).")
    ap.add_argument("--out", default=str(REPO_ROOT / "scripts/stress/stress_search_report.json"))
    ap.add_argument("--only", default="", help="Substring filter: only run cases whose name contains this.")
    ap.add_argument("--repeat", type=int, default=1, help="Run each case N times; report per-run latencies.")
    ap.add_argument("--parallel", type=int, default=1,
                    help="Fire N cases concurrently (stresses DISPATCH_CONCURRENCY=4).")
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
        # initialize handshake
        init_msg, init_elapsed, init_hung = client.call(
            "initialize",
            {"protocolVersion": "2024-11-05", "capabilities": {},
             "clientInfo": {"name": "stress", "version": "0.0.0"}},
            timeout_s=10.0,
        )
        if init_hung:
            print("initialize hung; aborting", file=sys.stderr)
            return 3
        results: list[CaseResult] = []

        expanded: list[Case] = []
        for c in cases:
            for i in range(args.repeat):
                if args.repeat == 1:
                    expanded.append(c)
                else:
                    expanded.append(Case(f"{c.name}#{i+1}", c.tool, dict(c.args), c.expect_ok, c.note))

        if args.parallel <= 1:
            for c in expanded:
                r = run_case(client, c, args.soft_deadline_ms)
                tag = _format_tag(r)
                print(f"{tag} {r.name:<42} {r.elapsed_ms:>8.1f}ms")
                results.append(r)
        else:
            sem = threading.Semaphore(args.parallel)
            lock = threading.Lock()
            out: list[CaseResult] = []
            threads: list[threading.Thread] = []

            def worker(case: Case) -> None:
                with sem:
                    r = run_case(client, case, args.soft_deadline_ms)
                    with lock:
                        tag = _format_tag(r)
                        print(f"{tag} {r.name:<42} {r.elapsed_ms:>8.1f}ms")
                        out.append(r)

            for c in expanded:
                t = threading.Thread(target=worker, args=(c,))
                t.start()
                threads.append(t)
            for t in threads:
                t.join()
            results = out

        report = {
            "binary": str(bin_path),
            "source_id": args.source,
            "soft_deadline_ms": args.soft_deadline_ms,
            "initialize_elapsed_ms": round(init_elapsed * 1000.0, 1),
            "cases": [asdict(r) for r in results],
            "stderr_tail": client.stderr_tail(60),
        }
        Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        Path(args.out).write_text(json.dumps(report, indent=2))

        hung = [r for r in results if r.hung]
        slow = [r for r in results if r.soft_deadline_exceeded and not r.hung]
        budget = [r for r in results if r.handler_budget_exceeded and not r.hung]
        print("")
        print(f"hung (no reply in wire window): {len(hung)}")
        for r in hung:
            print(f"  - {r.name}  wire_wait={r.elapsed_ms:.0f}ms  args={json.dumps(r.args)[:180]}")
        print(f"soft-deadline exceeded: {len(slow)}")
        for r in slow:
            print(f"  - {r.name}  {r.elapsed_ms:.0f}ms")
        print(f"handler 2s budget exceeded: {len(budget)}")
        for r in budget:
            print(f"  - {r.name}  {r.elapsed_ms:.0f}ms")
        print(f"report → {args.out}")

        return 1 if (hung or slow) else 0
    finally:
        client.close()


def _format_tag(r: CaseResult) -> str:
    if r.hung:
        return "HANG"
    if r.soft_deadline_exceeded:
        return "SLOW"
    if r.envelope_error:
        return "ERRR"
    if r.response_ok is False:
        return "FAIL"
    return "  OK"


if __name__ == "__main__":
    sys.exit(main())
