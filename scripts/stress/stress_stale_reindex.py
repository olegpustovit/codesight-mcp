#!/usr/bin/env python3
"""
Targeted repro for the real bug behind "search_catalog gets stuck 120s":

1. `ensure_doc_store` + `ensure_repo_store` check staleness on every
   read-path call. For *docs* the stale branch is **synchronous**:
   `do_index_local(&root)` blocks the handler thread until the docs have
   been fully re-ingested.
2. The handler dispatch is wrapped in a 2-second `HANDLER_TIMEOUT_MS`
   compile-time budget. When the budget expires, `dispatch_with_timeout`
   returns `Err(TIMEOUT)`.
3. `handle_request` propagates that `Err` with `?` and **never writes a
   response** to stdout. Only `tracing::error!("dispatch task failed")`
   is logged. Meanwhile the blocking thread keeps running (SQLite busy,
   vector embedder, file walker) and may hold connection locks.
4. The MCP client observes "no reply" and eventually fires its own
   120-second watchdog — exactly what the user reported.

This script forces condition (1) by rewriting a tracked doc file's mtime
*before* calling `search_catalog domain=docs`. If symslice ever returns
HANG (empty-wire) for an otherwise-well-formed call, the bug is
reproduced.

Run:
    python3 scripts/stress/stress_stale_reindex.py \
        [--bin target/release/symslice-mcp] \
        [--source olegpustovit/symslice-mcp]
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from stress_search import McpClient  # type: ignore  # noqa: E402


def touch_many(paths: list[Path]) -> None:
    for p in paths:
        if p.exists():
            now = time.time()
            os.utime(p, (now, now))


def candidate_doc_files() -> list[Path]:
    # Files most likely to be indexed by the docs domain on this repo.
    names = [
        "README.md", "AGENTS.md", "CLAUDE.md", "ARCHITECTURE.md",
        "REFACTOR_WORKLOG.md", "SYMSLICE_FEEDBACK.md", "CONFIGURATION.md",
        "AGENT_READINESS_V2.md", "IDF_REPLACEMENT.md", "MITIGATION_PLAN.md",
    ]
    return [REPO_ROOT / n for n in names]


def run_once(bin_path: Path, source: str, wire_timeout_s: float) -> dict:
    client = McpClient(bin_path, REPO_ROOT)
    try:
        init_msg, init_elapsed, init_hung = client.call(
            "initialize",
            {"protocolVersion": "2024-11-05", "capabilities": {},
             "clientInfo": {"name": "stale-repro", "version": "0"}},
            timeout_s=10.0,
        )
        assert not init_hung, f"initialize hung: {init_msg}"

        print(">>> warmup call (fresh index)")
        _, warm_ms, warm_hung = client.call_tool(
            "search_catalog",
            {"source_id": source, "domain": "docs",
             "query": "SYMSLICE_FEEDBACK", "query_mode": "keyword", "limit": 5},
            timeout_s=wire_timeout_s,
        )
        print(f"    warmup: {warm_ms*1000:.1f}ms  hung={warm_hung}")

        print(">>> touching 10 tracked doc files to mark index stale")
        touch_many(candidate_doc_files())

        # Send 5 concurrent stale-triggering docs searches on separate
        # threads. With DISPATCH_CONCURRENCY=4 and one sync reindex on the
        # blocking pool, we expect:
        #   - one handler to be bogged down in `do_index_local`,
        #   - the others to queue and/or get TIMEOUT,
        #   - the client to observe at least one HANG (bug signature).
        import threading
        results = []
        lock = threading.Lock()

        def fire(i: int) -> None:
            _, ms, hung = client.call_tool(
                "search_catalog",
                {"source_id": source, "domain": "docs",
                 "query": f"SYMSLICE_FEEDBACK {i}",
                 "query_mode": "keyword", "limit": 5},
                timeout_s=wire_timeout_s,
            )
            with lock:
                results.append({"i": i, "ms": ms * 1000, "hung": hung})

        threads = [threading.Thread(target=fire, args=(i,)) for i in range(5)]
        t0 = time.monotonic()
        for t in threads: t.start()
        for t in threads: t.join()
        elapsed = time.monotonic() - t0
        print(f">>> 5 concurrent stale-doc searches finished in {elapsed*1000:.0f}ms")
        for r in sorted(results, key=lambda r: r["i"]):
            tag = "HANG" if r["hung"] else "  OK"
            print(f"    {tag} i={r['i']} elapsed={r['ms']:.1f}ms")

        hung_count = sum(1 for r in results if r["hung"])
        budget_exceed = sum(1 for r in results if not r["hung"] and r["ms"] > 2000)
        print("")
        print(f"summary: hung={hung_count}  >2s (handler budget)={budget_exceed}")
        print("stderr tail:")
        print(client.stderr_tail(30))
        return {"hung": hung_count, "over_budget": budget_exceed, "results": results}
    finally:
        client.close()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=str(REPO_ROOT / "target/release/symslice-mcp"))
    ap.add_argument("--source", default="olegpustovit/symslice-mcp")
    ap.add_argument("--wire-timeout-s", type=float, default=15.0,
                    help="How long the test client waits for each reply before calling it hung.")
    args = ap.parse_args()
    bin_path = Path(args.bin)
    if not bin_path.exists():
        print(f"binary not found: {bin_path}", file=sys.stderr)
        return 2
    r = run_once(bin_path, args.source, args.wire_timeout_s)
    return 1 if (r["hung"] or r["over_budget"]) else 0


if __name__ == "__main__":
    sys.exit(main())
