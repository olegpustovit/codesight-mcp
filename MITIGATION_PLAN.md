# MITIGATION_PLAN.md

Detailed plan for steering coding agents toward token-efficient codesight-mcp
usage — via response-level mitigation, agentpack plugins (hooks + MCP config),
and session telemetry. Every mechanism must match or exceed prior-art-mcp's
steering depth while staying true to codesight's consolidated tool model.

---

## Problem Statement

codesight-mcp has the right structural tools — `search_catalog`,
`inspect_structure`, `inspect_graph`, `read_content` — but nothing *forces*
or even *nudges* models toward them at runtime. The guidance lives in
`README.md` and `AGENTS.md`, which are prompt-time hints that models can
(and do) skip under pressure.

### What prior-art does that we don't (yet)

prior-art uses a **multi-layer steering stack**:

| Layer | Mechanism | Strength |
|-------|-----------|----------|
| 1. System prompt | `tweakcc` rewrites Claude's system prompt | Strongest |
| 2. Hard hooks | `PreToolUse` exit-code-2 blocks on Bash/Grep/Glob | Strong |
| 3. Soft hooks | `PreToolUse` stderr warnings on large Read calls | Medium |
| 4. Auto-actions | `PostToolUse` auto-reindex after Edit/Write | Free |
| 5. Session context | `PreCompact` injects session snapshot before compaction | Medium |
| 6. Prompt policies | `CLAUDE.md`, `.cursorrules`, `.windsurfrules` templates | Weak-Medium |
| 7. Response nudges | `_meta.tip`, budget warnings, batch suggestions | Weak |
| 8. One-command setup | `prior-art-mcp init` installs hooks + policies + MCP entry | Force multiplier |

### What codesight has today

- `_meta` with `tokens_saved`, `cost_avoided`, `timing_ms` (every response)
- `next_step_hint` on verify/hash reads only (not plain reads)
- `STALE_DATA_WARNING` on stale-index reads
- `SEMANTIC_FALLBACK` when dense retrieval fails
- `CALL_HEURISTIC` on low-confidence graph results
- Session stats: per-tool call counts, tokens saved, timing, error counts
- `AGENTS.md` with exhaustive routing table (internal dogfood policy)
- `README.md` with copy-paste CLAUDE.md snippet (passive)
- `codesight-mcp init --claude-code` writes `.mcp.json` only

### The gap

1. **No runtime interception.** No hooks block or warn when the model reaches
   for native `Read`/`Grep`/`Glob` instead of codesight.
2. **No response-level mitigation.** `read_content` doesn't warn on expensive
   whole-file reads or suggest cheaper alternatives.
3. **No pluggable onboarding.** `init` writes `.mcp.json` but doesn't install
   hooks, agent policies, or integrate with agentpack.
4. **No auto-reindex.** Edits go stale until the next explicit `ingest_sources`.
5. **No compaction survival.** Session context is lost when Claude Code compacts.
6. **No read-shape telemetry.** Session stats can't distinguish efficient symbol
   reads from wasteful full-file reads.

---

## Desired Outcome

After this plan lands:

1. `agentpack add ../plugins/codesight-hooks` installs Claude Code hooks that
   intercept native tool calls and steer toward codesight.
2. `agentpack add ../plugins/codesight-mcp` registers the MCP server from PATH
   and installs routing rules + agent policy.
3. Every expensive `read_content` call self-identifies and suggests cheaper
   alternatives in the response.
4. Edits auto-reindex via PostToolUse hooks (zero model overhead).
5. Session snapshots survive context compaction via PreCompact hooks.
6. Session stats reveal whether the agent is reading efficiently.
7. Setup is `agentpack add` + `agentpack sync` — standard agentpack workflow.

Success metric: an agent using codesight with both plugins installed should
**never** fall back to native `Read`/`Grep`/`Glob` for code exploration unless
codesight genuinely can't answer the query.

---

## Design Constraints

- Do not split `read_content` into new tools.
- Do not hard-block whole-file reads from the MCP response side. Warn-only
  is correct — hooks are the enforcement layer.
- Do not add a required session protocol. Per-request behavior remains default.
- Do not inflate savings numbers.
- Do not push agents back to native `Read` for routine discovery.
- Hook *definitions* (hooks.json) are delivered via agentpack plugins. Hook
  *logic* lives in the codesight binary (`src/cli/hooks.rs`). The server
  works standalone; plugins make it better.
- Hooks require only `codesight-mcp` on PATH. No shell scripts, no Python,
  no Node, no jq.
- The two plugins are independent — `codesight-mcp` plugin works without
  `codesight-hooks`, and vice versa.

---

## Architecture: Two Agentpack Plugins

The steering system ships as **two agentpack plugins** in the `plugins/`
directory of this repo, following the standard agentpack plugin layout.

### Why two plugins instead of one

- **`codesight-mcp`**: Registers the MCP server + installs routing rules and
  agent instructions. Every codesight user needs this.
- **`codesight-hooks`**: Installs Claude Code hooks that intercept native
  Read/Grep/Glob/Bash. This is enforcement — optional but recommended. Some
  users may want codesight without the interception layer (e.g., when testing
  codesight itself, or when another hook system is already in place).

Keeping them separate means users can `agentpack add` either or both.

### Plugin locations

```
plugins/
├── codesight-hooks/           # Claude Code hooks plugin
│   ├── .claude-plugin/plugin.json
│   ├── .cursor-plugin/plugin.json
│   ├── agentpack.toml
│   ├── hooks/
│   │   └── hooks.json         # PreToolUse, PostToolUse, PreCompact
│   ├── rules/
│   │   └── codesight-hooks.mdc  # alwaysApply rule explaining hooks
│   └── README.md
│
└── codesight-mcp/             # MCP server + routing policy plugin
    ├── .claude-plugin/plugin.json
    ├── .cursor-plugin/plugin.json
    ├── agentpack.toml
    ├── mcp.json                # MCP server entry using codesight-mcp from PATH
    ├── rules/
    │   └── codesight-routing.mdc  # alwaysApply routing policy
    ├── agents/
    │   └── codesight-advisor.md   # Optional: agent that helps use codesight
    └── README.md
```

No shell scripts in the plugins. All hook commands call `codesight-mcp`
subcommands directly — the binary must be on PATH. Hook logic lives in
`src/cli/hooks.rs` inside the codesight crate.

---

## Workstream 1 — `plugins/codesight-hooks` (Claude Code Hooks)

Priority: **P0** (highest-leverage change)

Goal: intercept native tool calls at the Claude Code harness level and redirect
to codesight. This is where prior-art gets most of its steering power.

### 1A. Plugin Manifest

**`plugins/codesight-hooks/agentpack.toml`:**

```toml
name = "codesight-hooks"
version = "0.1.0"
description = "Claude Code hooks that steer agents toward codesight-mcp tools instead of native Read/Grep/Glob"
```

**`plugins/codesight-hooks/.claude-plugin/plugin.json`:**

```json
{
  "name": "codesight-hooks",
  "version": "0.1.0",
  "description": "PreToolUse/PostToolUse/PreCompact hooks for codesight-mcp steering"
}
```

**`plugins/codesight-hooks/.cursor-plugin/plugin.json`:**

```json
{
  "name": "codesight-hooks",
  "version": "0.1.0",
  "description": "Hook scripts for codesight-mcp steering (Cursor-compatible)"
}
```

### 1B. Hooks Definition

**`plugins/codesight-hooks/hooks/hooks.json`:**

All hooks call `codesight-mcp` subcommands directly. The binary reads JSON
from stdin (Claude Code hook protocol) and writes JSON/stderr to stdout/stderr.
No shell scripts, no jq dependency — just the Rust binary on PATH.

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Read",
        "hooks": [{
          "type": "command",
          "command": "codesight-mcp hook-pretooluse"
        }]
      },
      {
        "matcher": "Grep",
        "hooks": [{
          "type": "command",
          "command": "codesight-mcp hook-pretooluse"
        }]
      },
      {
        "matcher": "Glob",
        "hooks": [{
          "type": "command",
          "command": "codesight-mcp hook-pretooluse"
        }]
      },
      {
        "matcher": "Bash",
        "hooks": [{
          "type": "command",
          "command": "codesight-mcp hook-pretooluse"
        }]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [{
          "type": "command",
          "command": "codesight-mcp hook-posttooluse"
        }]
      }
    ],
    "PreCompact": [
      {
        "matcher": "",
        "hooks": [{
          "type": "command",
          "command": "codesight-mcp hook-precompact"
        }]
      }
    ]
  }
}
```

### 1C. Hook Subcommands in the Codesight Binary

The hook logic lives in `src/cli/hooks.rs`. Three new CLI subcommands:

```
codesight-mcp hook-pretooluse    # reads hook JSON from stdin, decides allow/warn/deny
codesight-mcp hook-posttooluse   # reads hook JSON from stdin, triggers single-file reindex
codesight-mcp hook-precompact    # reads hook JSON from stdin, returns session snapshot
```

All three:
- Read JSON from stdin (Claude Code hook protocol).
- Write JSON to stdout and/or warnings to stderr.
- Exit 0 (allow/warn) or exit 2 (deny).
- Handle malformed input gracefully (exit 0, no crash).
- Complete in <50ms (no network, no heavy I/O — just config + file stat).

Primary files:
- `src/cli/hooks.rs` (new)
- `src/cli/mod.rs` (add `pub mod hooks;`)
- `src/main.rs` (add subcommand dispatch)

#### `hook-pretooluse` Behavior

Single handler for all four matchers (Read, Grep, Glob, Bash). Dispatches
on `tool_name` from the stdin JSON.

**Read:**

1. Parse `tool_input.file_path`.
2. Not a code file (by extension — same ~50 language list codesight indexes):
   exit 0 silently.
3. Read has `offset` AND `limit` set (targeted range for edit prep):
   exit 0 silently. Edit workflows require native Read acknowledgement.
4. Small file (< `CODESIGHT_HOOK_MIN_SIZE` bytes, default 4096):
   exit 0 silently.
5. Large code file without offset/limit: **warn or deny**.

Warn mode (default):
- stderr: suggestions for `inspect_structure view=file_outline`,
  `search_catalog`, `read_content` with bounds.
- exit 0.

Block mode (`CODESIGHT_HOOK_BLOCK_READS=1`):
- stdout: deny JSON with `permissionDecision: "deny"` and
  `additionalContext` suggesting codesight alternatives.
- exit 0 (structured deny via JSON, not exit code).

**Grep:**

1. Parse `tool_input.pattern` and `tool_input.path`.
2. Non-code target path: exit 0.
3. Identifier-shaped pattern → stderr: suggest `search_catalog query_mode=keyword`.
4. Regex pattern → stderr: suggest `search_catalog query_mode=regex`.
5. Block mode: deny JSON suggesting search_catalog.

**Glob:**

1. Parse `tool_input.pattern`.
2. Tree walk (`**/*.rs`, `src/**`) → stderr: suggest `inspect_structure view=file_tree`.
3. File search → stderr: suggest `search_catalog output_mode=files_with_matches`.
4. Block mode: deny JSON.

**Bash:**

1. Parse `tool_input.command`.
2. Whitelist safe commands (`git`, `cargo`, `npm`, `yarn`, `docker`, `kubectl`,
   `pip`, `brew`, `make`, `cmake`, `python`, `node`, `bun`, `deno`): exit 0.
3. Code-exploration commands (`grep`, `rg`, `find`, `cat`, `head`, `tail`)
   targeting code files: stderr suggestion.
4. Everything else: exit 0.

**Config knobs (env vars):**

| Env var | Default | Effect |
|---------|---------|--------|
| `CODESIGHT_HOOK_MIN_SIZE` | `4096` | Files below this size pass through silently |
| `CODESIGHT_HOOK_BLOCK_READS` | `0` | `1` = deny instead of warn |
| `CODESIGHT_HOOK_ALLOW_EDIT_READS` | `1` | `1` = allow reads with offset/limit |

**Code file extensions** (built-in, ~50 languages — reuse codesight's
existing language registry from the extractor engine):

```
py ts tsx js jsx rs go java rb php cs cpp cc cxx c h hpp hxx swift kt kts
scala dart lua hs sql nix tf proto graphql jl r groovy pl pm bash sh zsh
fish vue svelte astro elm ex exs erl hrl clj cljs ml mli fs fsi v sv vhd
zig nim cr purs dhall toml yaml yml json xml
```

#### `hook-posttooluse` Behavior

Fires after Edit/Write. Auto-reindexes the edited file.

1. Parse `tool_input.file_path` from stdin JSON.
2. If file doesn't exist or path is empty: exit 0.
3. Spawn `codesight-mcp ingest --single-file <path>` as a background process
   (fire-and-forget). Do not wait for completion.
4. Exit 0 immediately.

This keeps the index fresh automatically and eliminates the "stale after edit"
problem.

#### `hook-precompact` Behavior

Fires before Claude Code context compaction. Returns a session snapshot.

1. Read `_savings.json` from the codesight storage dir.
2. Build a compact Markdown snapshot (session duration, tools used, tokens
   saved, warning count).
3. Return as `additionalContext` in the hook response JSON.

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreCompact",
    "additionalContext": "## codesight session snapshot\n- Tokens saved: 45,231\n- Tools: search_catalog(8), read_content(5), inspect_structure(3)\n- Warnings: 2 TOKEN_HEAVY_READ\n\nPrefer codesight-mcp tools for code exploration."
  }
}
```

### 1D. Always-On Rule: `rules/codesight-hooks.mdc`

```markdown
---
description: codesight-hooks active — native Read/Grep/Glob intercepted
alwaysApply: true
---

codesight-hooks are active in this workspace. Native Read, Grep, Glob, and
Bash code-exploration calls are intercepted and redirected to codesight-mcp
tools. If a hook warns or blocks, follow its suggestion — it's pointing to
a cheaper, structured alternative.
```

### 1E. Definition of Done

- Plugin installs via `agentpack add plugins/codesight-hooks && agentpack sync`.
- hooks.json is valid and references `codesight-mcp` subcommands.
- `codesight-mcp hook-pretooluse` correctly classifies Read/Grep/Glob/Bash
  and emits appropriate warnings or denials.
- `codesight-mcp hook-posttooluse` triggers background reindex.
- `codesight-mcp hook-precompact` returns a session snapshot.
- All subcommands handle malformed input gracefully (exit 0, no crash).
- Only runtime dependency: `codesight-mcp` on PATH. No shell scripts, no jq.
- Each subcommand completes in <50ms.

---

## Workstream 2 — `plugins/codesight-mcp` (MCP Server + Routing Policy)

Priority: **P0**

Goal: register codesight-mcp as an MCP server from PATH and install routing
rules that steer the agent toward codesight tools by default.

### 2A. Plugin Manifest

**`plugins/codesight-mcp/agentpack.toml`:**

```toml
name = "codesight-mcp"
version = "0.1.0"
description = "MCP server registration and routing policy for codesight-mcp"
```

**`plugins/codesight-mcp/.claude-plugin/plugin.json`:**

```json
{
  "name": "codesight-mcp",
  "version": "0.1.0",
  "description": "Registers codesight-mcp MCP server and installs code exploration routing policy"
}
```

**`plugins/codesight-mcp/.cursor-plugin/plugin.json`:**

```json
{
  "name": "codesight-mcp",
  "version": "0.1.0",
  "description": "Registers codesight-mcp MCP server and installs code exploration routing policy"
}
```

### 2B. MCP Server Entry

**`plugins/codesight-mcp/mcp.json`:**

```json
{
  "mcpServers": {
    "codesight": {
      "command": "codesight-mcp",
      "args": ["serve"]
    }
  }
}
```

This uses `codesight-mcp` from PATH. The binary must be installed separately
(e.g., `cargo install --path .` from the codesight repo, or placed on PATH
by the user). No absolute paths, no version pinning in the plugin — the user
controls which build they run.

### 2C. Routing Policy Rule: `rules/codesight-routing.mdc`

**`plugins/codesight-mcp/rules/codesight-routing.mdc`:**

```markdown
---
description: codesight-mcp code exploration routing — prefer structured retrieval over native tools
alwaysApply: true
---

## Code Exploration Policy — codesight-mcp

This workspace has codesight-mcp indexed. **Always use codesight tools for code
exploration. Never fall back to Read, Grep, Glob, or Bash for code discovery.**

Exception: use native `Read` only when your client requires a Read-before-Edit
acknowledgement. That is a client quirk, not the normal discovery path.

### Tool routing

| Task | Tool | Key args |
|------|------|----------|
| Find a symbol/function/type | `search_catalog` | `query_mode=keyword` |
| Search for a string/pattern | `search_catalog` | `query_mode=regex` |
| Fuzzy / unsure of name | `search_catalog` | `fuzzy=true` |
| Natural language question | `search_catalog` | `query_mode=natural_language`, `domain=hybrid` |
| What's in this file? | `inspect_structure` | `view=file_outline` |
| File tree / project structure | `inspect_structure` | `view=file_tree` |
| Repo overview | `inspect_structure` | `view=repo_outline` |
| Read a specific symbol body | `read_content` | `refs=[{kind:"symbol", ...}]` |
| Read a file range | `read_content` | `refs=[{kind:"file", start_line:N, end_line:M}]` |
| Batch read N items | `read_content` | pass N items in `refs` array — one call, not N |
| Who imports this file? | `inspect_graph` | `graph=importers` |
| What does this file import? | `inspect_graph` | `graph=imports` |
| Where is X referenced? | `inspect_graph` | `graph=references` |
| What does this function call? | `inspect_graph` | `graph=calls` |
| Pre-refactor blast radius | `evaluate_impact` | `analysis=blast_radius` |
| Rename safety check | `evaluate_impact` | `analysis=rename_safety` |
| Code quality / hotspots | `evaluate_quality` | `analysis=hotspots` |
| Dead code | `evaluate_quality` | `analysis=dead_code` |
| Repo health / doc coverage | `evaluate_quality` | `analysis=repo_health` |
| Context bundle for a ticket | `assemble_context` | `mode=context_bundle` |
| List indexed sources | `catalog_sources` | (no args) |

### Efficiency rules

- **Always start with `catalog_sources`.** Gets the `source_id` every tool needs.
- **Outline first.** Use `inspect_structure view=file_outline` before reading a
  whole file. Decide which symbols you need, then read those.
- **Batch reads.** `read_content` takes an array of `refs` — one call, not N.
- **Narrow scope.** Pass `scope=`, `file_pattern=`, or `unit_kinds=` to
  `search_catalog` to reduce result size.
- **Budget tokens.** Set `max_tokens=4000` for browsing, `8000` for deep reads.
- **Check `_meta.tokens_saved`.** If zero or negative, rethink the query.
- **Pick the right `query_mode`.** `keyword` for identifiers (fastest), `regex`
  for patterns, `natural_language` for questions.
```

`alwaysApply: true` ensures this rule is in context for every turn, including
subagents (Cursor) and all harnesses that agentpack supports.

### 2D. Optional Advisor Agent

**`plugins/codesight-mcp/agents/codesight-advisor.md`:**

```markdown
---
name: codesight-advisor
description: Helps diagnose codesight-mcp usage and suggests efficient retrieval strategies
---

# codesight-advisor

You help users get the most out of codesight-mcp. When invoked:

1. Run `catalog_sources` to check index freshness.
2. Run `manage_indexes action=session_stats` to see current session metrics.
3. Analyze the read efficiency — are there too many full-file reads?
4. Suggest concrete improvements to the user's retrieval patterns.
5. If the index is stale, run `ingest_sources incremental=true`.

Always explain *why* a codesight tool is better than the native alternative
for the specific case at hand.
```

### 2E. Definition of Done

- `mcp.json` registers codesight-mcp from PATH with `serve` arg.
- `rules/codesight-routing.mdc` contains the full routing table with
  `alwaysApply: true`.
- Plugin installs cleanly via `agentpack add` + `agentpack sync`.
- Works for Claude Code, Cursor, OpenCode, Codex via agentpack's
  artifact conversion.

---

## Workstream 3 — Response-Level Mitigation in `read_content`

Priority: **P0**

Goal: make expensive reads self-identifying and self-correcting *in the MCP
response*, independent of plugins. This is the server-side complement to the
hooks plugin's client-side interception.

Primary files:
- `src/service/agent_v1/content.rs`
- `src/tools.json`
- `tests/read_content_mitigation.rs` (new)
- `tests/read_content_next_step_hint.rs` (extend)

### 3A. Read Scope Classification

Classify every `read_content` ref item into one of:

| Scope | Condition |
|-------|-----------|
| `symbol` | `kind=symbol` |
| `section` | `kind=section` |
| `file_range` | `kind=file` AND both `start_line` and `end_line` are set |
| `file_full` | `kind=file` AND either `start_line` or `end_line` is missing |

Add to each item's output:

```json
{
  "ref": { ... },
  "read_scope": "file_full",
  ...
}
```

Add to `_meta`:

```json
{
  "read_scopes": {
    "symbol": 2,
    "section": 0,
    "file_range": 1,
    "file_full": 1
  },
  "returned_bytes": 24576,
  "warned_full_file_reads": 1
}
```

### 3B. TOKEN_HEAVY_READ Warning

Emit when:
- `read_scope=file_full` AND
- returned content exceeds `CODESIGHT_TOKEN_HEAVY_READ_BYTES` (default: 4096)

Warning shape:

```json
{
  "code": "TOKEN_HEAVY_READ",
  "message": "Whole-file read returned 12,480 bytes. Token-efficient alternatives: inspect_structure view=file_outline, search_catalog with include_body=true, or read_content with start_line/end_line.",
  "detail": {
    "read_scope": "file_full",
    "path": "src/service/agent_v1/content.rs",
    "returned_bytes": 12480,
    "threshold_bytes": 4096
  }
}
```

The warning must NOT appear on:
- Symbol reads (any size)
- Section reads (any size)
- Bounded file ranges (start_line + end_line both set)
- Files below the threshold

### 3C. Extended next_step_hint for Expensive Reads

Currently `next_step_hint` only appears on verify/hash reads. Extend it to
cover expensive plain reads:

```rust
// After the existing verify/hash hint block:
if has_heavy_full_file_read && !args.verify && !want_hash {
    extra.push((
        "next_step_hint".to_string(),
        json!({
            "tool": "inspect_structure",
            "reason": "This response included whole-file reads that could be replaced with targeted retrieval.",
            "alternatives": [
                {
                    "tool": "inspect_structure",
                    "args": {"view": "file_outline"},
                    "when": "to see what's in a file without reading all of it"
                },
                {
                    "tool": "search_catalog",
                    "args": {"query_mode": "keyword", "include_body": true},
                    "when": "to find and read a specific symbol or pattern"
                },
                {
                    "tool": "read_content",
                    "args": {"refs": [{"kind": "file", "start_line": "N", "end_line": "M"}]},
                    "when": "to read a bounded range instead of the whole file"
                }
            ]
        }),
    ));
}
```

Config knob:
- `CODESIGHT_TOKEN_HEAVY_READ_BYTES` (env var, default 4096)
- Add to `src/config/env.rs`

Definition of done:
- Heavy full-file reads return TOKEN_HEAVY_READ warning + next_step_hint
- Bounded reads, symbol reads, section reads do not
- `src/tools.json` documents the warning code and threshold
- New test binary `tests/read_content_mitigation.rs` covers all cases

---

## Workstream 4 — Contract and Documentation Tightening

Priority: **P0**

Goal: make the intended retrieval ladder visible in the MCP contract and docs.

Primary files:
- `src/tools.json`
- `README.md`
- `AGENTS.md`
- `CONFIGURATION.md`

### 4A. Tool Description Steering in `tools.json`

Change neutral descriptions to directive ones:

**Before:**
```json
"read_content": {
  "description": "Batch read symbols, sections, or files by ref."
}
```

**After:**
```json
"read_content": {
  "description": "Batch read symbols, sections, or bounded file ranges. Whole-file reads work but are a last resort — prefer inspect_structure view=file_outline first, then symbol/section reads or bounded ranges. May emit TOKEN_HEAVY_READ warning when a full-file read exceeds the configured threshold."
}
```

**Before:**
```json
"search_catalog": {
  "description": "Unified search over symbols, sections, and file text."
}
```

**After:**
```json
"search_catalog": {
  "description": "Unified search over symbols, sections, and file text. Preferred over native Grep for all code exploration — returns structured results with token savings. Use query_mode=keyword for identifiers, regex for patterns, natural_language for questions."
}
```

Apply similar steering language to `inspect_structure`, `inspect_graph`.

### 4B. Warning Code Registry

Add to `CONFIGURATION.md`:

```markdown
## Warning Codes

| Code | Tool | Trigger | Recovery |
|------|------|---------|----------|
| `TOKEN_HEAVY_READ` | `read_content` | Whole-file read exceeds threshold | Use file_outline, symbol reads, or bounded ranges |
| `STALE_DATA_WARNING` | all read tools | Index is stale | Run `ingest_sources incremental=true` |
| `SEMANTIC_FALLBACK` | `search_catalog` | Dense retrieval failed | Results are BM25-only; check embedding model |
| `CALL_HEURISTIC` | `inspect_graph` | Call graph used text heuristics | Use AST-based graph=calls when available |
| `REGRESSION_DETECTED` | `evaluate_quality` | Quality regression vs. snapshot | Review regressed symbols |
```

### 4C. README.md Update

Replace the current passive snippet with an agentpack-aware setup section:

```markdown
## Setup

### With agentpack (recommended)

```bash
# In your project directory:
agentpack add path/to/plugins/codesight-mcp     # MCP server + routing policy
agentpack add path/to/plugins/codesight-hooks    # Optional: hook enforcement
agentpack sync
agentpack claude   # or: agentpack agent, agentpack opencode, agentpack codex
```

### Without agentpack (manual)

1. Register the MCP server:
   ```bash
   codesight-mcp init --claude-code
   ```

2. Add this to your workspace `CLAUDE.md`:
   (routing policy snippet)

3. Index the repo:
   ```bash
   codesight-mcp ingest --path .
   ```
```

### 4D. Agent-Config Wording Fix

Docs must stop saying "use `Read` if you plan to edit" as a blanket rule.
Replace with: "some clients require an explicit native-read acknowledgement
before edit; that is a client quirk, not the normal discovery path."

Definition of done:
- `tools.json` descriptions steer toward efficient usage
- Warning code registry in CONFIGURATION.md
- README.md documents the agentpack setup path
- Wording fixed for edit-read relationship

---

## Workstream 5 — Session Telemetry for Read Efficiency

Priority: **P1**

Goal: make `manage_indexes action=session_stats` useful for diagnosing bad
agent behavior.

Primary files:
- `src/service/savings.rs`
- `src/service/agent_v1/index_admin.rs`
- `tests/session_stats.rs`

### 5A. Enrich ToolSessionStats for read_content

```rust
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct ReadContentStats {
    pub symbol_reads: u64,
    pub section_reads: u64,
    pub file_range_reads: u64,
    pub file_full_reads: u64,
    pub warning_count: u64,
    pub bytes_returned: u64,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct ToolSessionStats {
    pub calls: u64,
    pub tokens_saved: u64,
    pub timing_ms_sum: f64,
    pub error_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_content_detail: Option<ReadContentStats>,
}
```

### 5B. Surface in session_stats Response

```json
{
  "per_tool": {
    "read_content": {
      "calls": 8,
      "tokens_saved": 12340,
      "timing_ms_sum": 45.2,
      "error_count": 0,
      "read_content_detail": {
        "symbol_reads": 5,
        "section_reads": 0,
        "file_range_reads": 2,
        "file_full_reads": 1,
        "warning_count": 1,
        "bytes_returned": 34567
      }
    }
  }
}
```

### 5C. Efficiency Score

Add a computed field to session_stats:

```json
{
  "read_efficiency": {
    "score": 0.87,
    "label": "good",
    "detail": "87% of reads were targeted (symbol/section/range). 1 whole-file read triggered a TOKEN_HEAVY_READ warning."
  }
}
```

Scoring:
- 1.0 = all reads are symbol/section/range
- 0.0 = all reads are file_full
- Weighted: symbol=1.0, section=1.0, file_range=0.8, file_full=0.0

Labels: `excellent` (>=0.9), `good` (>=0.7), `fair` (>=0.5), `poor` (<0.5)

Definition of done:
- Session stats reveal whether a session was dominated by full-file reads
- Warning counts correlate with token-heavy reads
- Efficiency score is actionable (agent can self-correct)

---

## Workstream 6 — First-Class Opening Move Planner

Priority: **P1**

Goal: reduce bad opening moves before they happen. Upgrade
`assemble_context mode=query_suggestions` into a full ranked read-plan.

This ships as `evaluate_quality analysis=plan_turn` — a new enum value on
an existing tool, not a new tool.

Primary files:
- `src/service/agent_v1/quality/mod.rs`
- `src/service/agent_v1/quality/plan_turn.rs` (new)
- `src/tools.json`

### Behavior

Input:
```json
{
  "analysis": "plan_turn",
  "query": "add rate limiting to the API endpoints",
  "max_recommended": 5
}
```

Output:
```json
{
  "confidence": "high",
  "recommended_symbols": [
    {
      "id": "src/service/agent_v1/validate.rs::validate_args",
      "relevance": 0.92,
      "reason": "validation middleware — rate limiting likely hooks in here",
      "read_ref": {"kind": "symbol", "stable_id": "..."}
    }
  ],
  "recommended_files": ["src/service/agent_v1/validate.rs", "src/service/mod.rs"],
  "max_supplementary_reads": 3,
  "token_estimate": 2400,
  "gap_analysis": null
}
```

Confidence thresholds (configurable):
- `high` (BM25 score >= 2.0, >= 3 hits): "Go directly to recommended symbols"
- `medium` (score >= 0.5, >= 1 hit): "Explore related files"
- `low` (below): "This feature likely needs to be created"

The key differentiator from prior-art's `plan_turn`: our version returns
**ready-to-call `read_content` refs**, not just file names. The agent can
feed the `read_ref` directly into `read_content` with zero additional
lookups.

Definition of done:
- An agent can begin with `plan_turn` and avoid broad file reads in common
  workflows
- Confidence levels are calibrated against this repo's index

---

## Workstream 7 — Agent Config Audit

Priority: **P2**

Goal: detect stale or missing routing guidance in agent instruction files.

Ships as `manage_indexes action=audit_config`.

Primary files:
- `src/service/agent_v1/index_admin.rs`
- `src/tools.json`

### Scanned Files

| File | Agent |
|------|-------|
| `CLAUDE.md` | Claude Code, Codex, OpenCode |
| `AGENTS.md` | Open standard |
| `.cursorrules`, `.cursor/rules/*.mdc` | Cursor |
| `.github/copilot-instructions.md`, `.github/copilot/**` | GitHub Copilot |
| `.windsurfrules`, `.windsurf/rules/**` | Windsurf |
| `.continue/**` | Continue |

### Checks

1. **Stale symbol references**: Cross-check symbol names in instruction files
   against the codesight index. Flag names that don't resolve.
2. **Dead file paths**: Check file paths mentioned in instructions against the
   filesystem.
3. **Missing routing policy**: Flag if none of the scanned files mention
   codesight or structured retrieval tools.
4. **Token cost**: Report per-file token count. Large instruction files waste
   context.
5. **Redundancy**: Detect identical blocks across multiple agent config files.

### Output

```json
{
  "files_scanned": 4,
  "findings": [
    {
      "severity": "warning",
      "category": "stale_symbol",
      "file": "CLAUDE.md",
      "line": 45,
      "message": "Symbol 'validate_request' not found in index"
    },
    {
      "severity": "info",
      "category": "missing_policy",
      "file": ".cursorrules",
      "message": "No codesight routing policy found. Install the codesight-mcp agentpack plugin."
    }
  ],
  "total_instruction_tokens": 3400
}
```

Definition of done:
- The repo can audit whether its agent instructions are teaching the
  efficient path
- Stale references are flagged with line numbers
- Missing policies suggest the agentpack plugin

---

## Workstream 8 — Test Matrix

Priority: **P0** (for Workstreams 3-4), **P1** (for 5-7)

### New Test Binaries

**`tests/read_content_mitigation.rs`** (P0):
- Full-file read >= threshold -> TOKEN_HEAVY_READ warning present
- Full-file read < threshold -> no warning
- Bounded file read (start_line + end_line) -> no warning
- Symbol read -> no warning
- Section read -> no warning
- Multiple refs: only full-file refs get warnings
- `_meta.read_scopes` counts are correct
- `_meta.returned_bytes` is correct
- `_meta.warned_full_file_reads` count is correct

**`tests/read_content_next_step_hint.rs`** (extend, P0):
- verify/hash read -> existing edit-safety hint (unchanged)
- heavy plain read -> new alternatives hint with inspect_structure/search_catalog
- cheap plain read -> no hint
- verify + heavy -> both hints present

**`tests/session_stats.rs`** (extend, P1):
- `read_content_detail` counters increment correctly
- `warning_count` tracks TOKEN_HEAVY_READ emissions
- `read_efficiency.score` is correct for mixed sessions

**`tests/hooks_cli.rs`** (new, P0):
- `hook-pretooluse` with Read + large code file → stderr warning
- `hook-pretooluse` with Read + small code file → no output
- `hook-pretooluse` with Read + non-code file → no output
- `hook-pretooluse` with Read + offset/limit → no output
- `hook-pretooluse` with Grep → stderr suggestion
- `hook-pretooluse` with Glob → stderr suggestion
- `hook-pretooluse` with Bash + safe command → no output
- `hook-pretooluse` with Bash + grep pattern → stderr suggestion
- `hook-posttooluse` with Edit → spawns reindex
- `hook-precompact` → returns session snapshot JSON
- Malformed JSON input → exit 0 (no crash)
- `CODESIGHT_HOOK_BLOCK_READS=1` → deny JSON instead of stderr

**Plugin integration tests** (manual, P0):
- `agentpack add plugins/codesight-hooks && agentpack sync` succeeds
- `agentpack add plugins/codesight-mcp && agentpack sync` succeeds
- Hooks fire correctly in Claude Code session
- MCP server starts via agentpack staging
- Rules appear in agent context

### Run command

```bash
cargo nextest run --test read_content_mitigation \
  --test read_content_next_step_hint \
  --test hooks_cli \
  --test session_stats \
  --test envelope_shape
```

---

## Rollout Order

Deliberate. Each step is independently shippable and testable.

### Phase 1: Agentpack Plugins + Hook Subcommands (Workstreams 1 + 2)

Highest leverage. Adds hooks + MCP + routing policy via agentpack.

1. Implement `src/cli/hooks.rs` (hook-pretooluse, hook-posttooluse, hook-precompact).
2. Create `plugins/codesight-hooks/` with hooks.json pointing at the binary.
3. Create `plugins/codesight-mcp/` with mcp.json and routing rule.
4. Add `tests/hooks_cli.rs`.
5. Test with `agentpack add` + `agentpack sync` + `agentpack claude`.
6. Update README.md with agentpack setup instructions.

### Phase 2: Response-Level Mitigation (Workstreams 3 + 4A + 8 partial)

Server-side complement to hooks.

1. Add read scope classification in `content.rs`.
2. Emit TOKEN_HEAVY_READ warning + extended next_step_hint.
3. Update `tools.json` descriptions.
4. Add `tests/read_content_mitigation.rs`.
5. Update CONFIGURATION.md with warning code registry.

### Phase 3: Telemetry Enrichment (Workstream 5)

Makes the mitigation measurable.

1. Add `ReadContentStats` to `ToolSessionStats`.
2. Wire up counters in `content.rs`.
3. Add efficiency score to session_stats.
4. Extend `tests/session_stats.rs`.

### Phase 4: Opening Move Planner (Workstream 6)

Proactive steering.

1. Implement `evaluate_quality analysis=plan_turn`.
2. Add tests.
3. Update `tools.json`.

### Phase 5: Config Audit (Workstream 7)

Completeness check.

1. Implement `manage_indexes action=audit_config`.
2. Add tests.

---

## Comparison: After This Plan vs. prior-art

| Mechanism | prior-art | codesight (after) | Winner |
|-----------|-----------|-------------------|--------|
| Read guard hooks | Python CLI, stderr warn | Rust binary via agentpack plugin | **codesight** (faster, no Python) |
| Grep/Glob/Bash interception | Shell scripts, exit-code-2 block | Rust binary, same protocol | **codesight** (faster, no shell deps) |
| Auto-reindex after edits | Python subprocess, fire-and-forget | Rust binary, fire-and-forget | **codesight** (faster) |
| Session snapshot on compact | Python, in-process | Rust binary reading _savings.json | **codesight** (faster) |
| Agent routing policy | 62-line CLAUDE.md template | Full routing table as agentpack rule (alwaysApply) | **codesight** (richer, cross-harness) |
| Distribution | `prior-art-mcp init` (Python) | agentpack plugins (`add` + `sync`) | **codesight** (standard package manager, multi-harness) |
| Response warnings | Implicit (tips in _meta) | Explicit warning codes + next_step_hint | **codesight** (structured) |
| Session telemetry | Per-turn budget tracking | Per-tool read-scope breakdown + efficiency score | **codesight** (deeper) |
| Opening move planner | plan_turn (symbol + file recs) | plan_turn with ready-to-call read_content refs | **codesight** (zero-step-to-code) |
| Config audit | Scan 2 agent configs | Scan 6+ agent config conventions | **codesight** (universal) |
| Multi-harness support | Claude Code + Cursor + Windsurf | Claude Code + Cursor + OpenCode + Codex via agentpack | **codesight** |
| Turn budget | Per-turn token tracking | Not planned (agent owns its state) | **prior-art** |
| System prompt patching | tweakcc | Not planned (third-party concern) | **prior-art** |
| Tool count | 30+ flat tools | 10 consolidated tools | **codesight** (simpler) |

### What we deliberately skip

- **Turn budget tracking**: The agent owns its context window. Adding a
  server-side budget counter creates session-as-state coupling that violates
  our design principle. The agent's own token awareness (via `max_tokens`
  params) is the right mechanism.
- **System prompt patching (tweakcc)**: Third-party tool concern. We provide
  hooks and policies via agentpack; patching Claude's system prompt is outside
  our scope and fragile across versions.
- **AI-provider coupling**: No LLM calls from inside codesight. Summaries,
  complexity scoring, and model routing are the agent's job.
- **Session-as-state**: No mandatory `register_edit` or `get_session_context`.
  Hooks handle reindexing; the agent handles its own state.
- **Shell scripts for hooks**: Hook logic lives in the Rust binary
  (`src/cli/hooks.rs`), not in shell scripts. The agentpack plugin just has
  hooks.json pointing at `codesight-mcp` subcommands.

---

## What Not to Do

- Do not create flat aliases like `read_file`, `read_symbol`, `get_file_outline`.
- Do not deny whole-file reads from the MCP response side.
- Do not emit warnings on every read; warning spam will get ignored.
- Do not move mitigation into a client-only layer and leave the server blind.
- Do not make `_meta.tokens_saved` the only signal; read-shape telemetry is
  necessary.
- Do not put hook logic in shell scripts. It lives in the Rust binary
  (`src/cli/hooks.rs`); the agentpack plugin just has hooks.json.
- Do not require Python, Node.js, jq, or any runtime beyond the codesight
  binary for hooks.
- Do not make hooks mandatory. The server works without them.
- Do not add session-as-state coupling to the server.
- Do not bundle both plugins into one — they serve different purposes and
  should be independently installable.

---

## Immediate Next Patch

If we start now, the first implementation should do this:

1. Implement `src/cli/hooks.rs` with `hook-pretooluse`, `hook-posttooluse`,
   `hook-precompact` subcommands. Wire into `src/main.rs`.
2. Create `plugins/codesight-hooks/` — agentpack plugin with hooks.json
   pointing at `codesight-mcp hook-*` from PATH. No shell scripts.
3. Create `plugins/codesight-mcp/` — agentpack plugin with mcp.json
   (`codesight-mcp serve` from PATH) and routing policy rule.
4. Add `tests/hooks_cli.rs`.
5. Test both plugins with `agentpack add` + `agentpack sync`.

That is the highest-leverage change — it adds the entire enforcement and
routing layer with minimal Rust (hooks are thin stdin→classify→respond
functions).

The second patch then adds the server-side complement:

6. Add `file_full` / `file_range` classification in
   `src/service/agent_v1/content.rs`.
7. Emit `TOKEN_HEAVY_READ` warnings plus a new next-step hint on large
   whole-file reads.
8. Add `_meta.read_scopes` and `_meta.warned_full_file_reads`.
9. Document the behavior in `src/tools.json` and `CONFIGURATION.md`.
10. Add `tests/read_content_mitigation.rs`.
