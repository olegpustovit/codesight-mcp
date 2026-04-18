#!/usr/bin/env python3
"""
Stress-reproduce the slow-tool timeouts I hit during the 2026-04-17 quality
push (see CODESIGHT_FEEDBACK_2.md). Each case targets a specific code path
that either exceeded the 2000ms HANDLER_TIMEOUT or came dangerously close.

Cases, in priority order:

1. `evaluate_quality analysis=refactor_targets` on a 445-file repo.
   Reported result: [TIMEOUT] handler 'evaluate_quality' exceeded 2000ms.
   Several `top_files` configurations. This is the flagship refactor-advisor
   analysis; losing it costs the whole session's prioritization.
2. `read_content` of a mid-sized symbol body
   (src/service/agent_v1/content.rs::CodeService.read_content#function,
   ~220 lines). Timed out on a single read. Bounded file-range reads of
   the same span work in <100ms via native Read, so this is likely an
   N+1 or a stale-check-on-hot-path.
3. `evaluate_quality analysis=hotspots|quality_signal|test_gaps` baselines
   for context. These worked but the p95 is uncomfortably close to the
   2000ms cap.
4. Heavy `evaluate_impact` + `inspect_graph` analyses that I didn't hit
   this session but are likely on the same slow path.

Run:
    python3 scripts/stress/stress_slow_tools.py [--bin PATH] [--source SOURCE_ID]
                                                [--repeat N] [--out REPORT.json]
                                                [--soft-deadline-ms 1800]

Exit non-zero if any case hangs or exceeds the soft deadline. Writes a
JSON report to `scripts/stress/stress_slow_tools_report.json`.
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
DEFAULT_BIN = REPO_ROOT / "target" / "release" / "codesight-mcp"


class McpClient:
    def __init__(self, bin_path: Path, workspace: Path, env_overrides: dict[str, str] | None = None):
        env = os.environ.copy()
        env.setdefault("CODESIGHT_LOG_FORMAT", "json")
        env.setdefault("RUST_LOG", "warn,codesight_mcp=warn")
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
            with self._lock:
                self._pending.pop(req_id, None)
            return None, time.monotonic() - start, True

    def call_tool(self, tool: str, arguments: dict[str, Any], *, timeout_s: float) -> tuple[Optional[dict[str, Any]], float, bool]:
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


def build_cases(source_id: str) -> list[Case]:
    S = source_id
    return [
        # ---- 1. evaluate_quality refactor_targets (flagship timeout) ----
        Case(
            "eq_refactor_targets_default",
            "evaluate_quality",
            {"source_id": S, "analysis": "refactor_targets"},
            note="Default args — this is what an agent types first; timed out on my session.",
        ),
        Case(
            "eq_refactor_targets_top15_budget3k",
            "evaluate_quality",
            {"source_id": S, "analysis": "refactor_targets", "top_files": 15, "max_tokens": 3000},
            note="Exact args that timed out in the 2026-04-17 quality session.",
        ),
        Case(
            "eq_refactor_targets_top5",
            "evaluate_quality",
            {"source_id": S, "analysis": "refactor_targets", "top_files": 5},
            note="Smaller top_files to bisect the slow path — is it per-file cost or fixed?",
        ),
        Case(
            "eq_refactor_targets_top30",
            "evaluate_quality",
            {"source_id": S, "analysis": "refactor_targets", "top_files": 30},
            note="Twice the default; stresses the per-file analyser if that's the bottleneck.",
        ),
        # ---- 2. read_content of the specific timing-out symbol ----
        Case(
            "rc_symbol_read_content_fn",
            "read_content",
            {
                "source_id": S,
                "refs": [{
                    "kind": "symbol",
                    "stable_id": "src/service/agent_v1/content.rs::CodeService.read_content#function",
                }],
            },
            note="Exact symbol that timed out on stable_id read. ~220 lines.",
        ),
        Case(
            "rc_symbol_read_content_with_body",
            "read_content",
            {
                "source_id": S,
                "refs": [{
                    "kind": "symbol",
                    "stable_id": "src/service/agent_v1/content.rs::CodeService.read_content#function",
                }],
                "include": ["body"],
            },
            note="Same symbol, explicit body include.",
        ),
        Case(
            "rc_symbol_medium_search_catalog",
            "read_content",
            {
                "source_id": S,
                "refs": [{
                    "kind": "symbol",
                    "stable_id": "src/service/agent_v1/catalog_search/mod.rs::CodeService.search_catalog#function",
                }],
            },
            note="Similar-size symbol, control for 'symbol size caused it'.",
        ),
        Case(
            "rc_symbol_batch_5",
            "read_content",
            {
                "source_id": S,
                "refs": [
                    {"kind": "symbol", "stable_id": "src/service/agent_v1/util.rs::outline_neighbors#function"},
                    {"kind": "symbol", "stable_id": "src/service/agent_v1/util.rs::search_hit_symbol#function"},
                    {"kind": "symbol", "stable_id": "src/service/agent_v1/util.rs::resolve_symbol_id#function"},
                    {"kind": "symbol", "stable_id": "src/service/agent_v1/util.rs::pack_catalog_items_to_token_budget#function"},
                    {"kind": "symbol", "stable_id": "src/service/agent_v1/util.rs::truncate_content_to_token_budget#function"},
                ],
            },
            note="Batch read of 5 small symbols — does batch amortize or compound?",
        ),
        Case(
            "rc_file_bounded_range",
            "read_content",
            {
                "source_id": S,
                "refs": [{
                    "kind": "file",
                    "path": "src/service/agent_v1/content.rs",
                    "start_line": 170,
                    "end_line": 380,
                }],
            },
            note="Same 210-line span as the timing-out symbol, but via file-range read. Control path.",
        ),
        # ---- 3. evaluate_quality baselines (worked but slow) ----
        Case(
            "eq_quality_signal",
            "evaluate_quality",
            {"source_id": S, "analysis": "quality_signal"},
            note="Baseline: took ~1s in my session.",
        ),
        Case(
            "eq_hotspots_top20",
            "evaluate_quality",
            {"source_id": S, "analysis": "hotspots", "min_complexity": 20, "top_n": 20, "max_tokens": 2500},
            note="Worked in ~250ms; control for slow-path comparison.",
        ),
        Case(
            "eq_test_gaps_top25",
            "evaluate_quality",
            {"source_id": S, "analysis": "test_gaps", "top_n": 25, "max_tokens": 3000},
            note="Worked in ~180ms.",
        ),
        Case(
            "eq_dead_code",
            "evaluate_quality",
            {"source_id": S, "analysis": "dead_code", "granularity": "symbol", "max_tokens": 3000},
        ),
        Case(
            "eq_repo_health",
            "evaluate_quality",
            {"source_id": S, "analysis": "repo_health"},
        ),
        # ---- 4. heavy graph + impact paths (suspected slow too) ----
        Case(
            "ig_dependencies",
            "inspect_graph",
            {"source_id": S, "graph": "dependencies", "max_tokens": 3000},
        ),
        Case(
            "ig_module_coupling",
            "inspect_graph",
            {
                "source_id": S,
                "graph": "module_coupling",
                "module_path": "src/service/agent_v1/catalog_search/mod.rs",
            },
            note="Requires module_path; must be an indexed file, not a directory.",
        ),
        Case(
            "ig_centrality_pagerank",
            "inspect_graph",
            {"source_id": S, "graph": "centrality", "algorithm": "pagerank", "top_n": 20},
        ),
        Case(
            "ei_blast_radius_search_catalog",
            "evaluate_impact",
            {
                "source_id": S,
                "analysis": "blast_radius",
                "refs": [{
                    "kind": "symbol",
                    "stable_id": "src/service/agent_v1/catalog_search/mod.rs::CodeService.search_catalog#function",
                }],
            },
            note="High fan-in symbol — blast_radius is O(fan_in) in the worst case.",
        ),
    ]


def run_case(client: McpClient, case: Case, soft_deadline_ms: int) -> CaseResult:
    # Always allow some grace over the soft deadline on the wire, so we can
    # distinguish "handler exceeded its own 2000ms cap" from "server never replied".
    wire_timeout_s = max(soft_deadline_ms / 1000.0, 2.5) + 10.0
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


def _format_tag(r: CaseResult) -> str:
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
    ap.add_argument("--source", default="OlegHQ/codesight-mcp")
    ap.add_argument("--soft-deadline-ms", type=int, default=1800,
                    help="Warn below the 2000ms handler cap so we see the danger zone.")
    ap.add_argument("--repeat", type=int, default=3,
                    help="Each case runs N times to expose p95 latency.")
    ap.add_argument("--only", default="", help="Substring filter.")
    ap.add_argument("--out", default=str(REPO_ROOT / "scripts/stress/stress_slow_tools_report.json"))
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
        init_msg, init_elapsed, init_hung = client.call(
            "initialize",
            {"protocolVersion": "2024-11-05", "capabilities": {},
             "clientInfo": {"name": "stress_slow", "version": "0.0.0"}},
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
            tag = _format_tag(r)
            print(f"{tag:>7} {r.name:<48} {r.elapsed_ms:>8.1f}ms")
            results.append(r)

        # Summary stats per logical case (grouped by prefix before '#')
        groups: dict[str, list[float]] = {}
        for r in results:
            base = r.name.split("#")[0]
            groups.setdefault(base, []).append(r.elapsed_ms)
        print()
        print(f"{'case':<48} {'min':>8} {'p50':>8} {'p95':>8} {'max':>8}")
        for base, xs in groups.items():
            xs_sorted = sorted(xs)
            mn = xs_sorted[0]
            mx = xs_sorted[-1]
            p50 = xs_sorted[len(xs_sorted) // 2]
            p95 = xs_sorted[int(0.95 * (len(xs_sorted) - 1))] if len(xs_sorted) > 1 else xs_sorted[0]
            print(f"{base:<48} {mn:>8.1f} {p50:>8.1f} {p95:>8.1f} {mx:>8.1f}")

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

        fails = [r for r in results if r.hung or r.handler_budget_exceeded or r.soft_deadline_exceeded or r.envelope_error]
        return 1 if fails else 0
    finally:
        client.close()


if __name__ == "__main__":
    sys.exit(main())
