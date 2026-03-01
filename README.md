# agit

Git tracks what changed. agit tracks **why it's like this.**

agit is a knowledge layer for AI agents that travels with your code. Every file accumulates structured annotations — decisions made, lessons learned, constraints discovered, failed approaches — that any agent reads before making changes.

```
Code:    what the software does          (source files)
Git:     what changed and when           (commits, diffs)
agit:    why it's like this              (knowledge annotations)
```

## The Problem

Every time an agent touches a file, it starts from zero understanding. It will try the same failed approach that was attempted months ago, refactor something that was deliberately written that way, or break a subtle invariant that isn't obvious from the code.

Git tells you *what* changed. Agents need to know *why it's like this* and *what to watch out for.*

## How It Works

agit creates a `.agit/` shadow directory that mirrors your source tree. Each file gets up to two companions:

- **`.md`** — synthesized knowledge (decisions, constraints, lessons learned)
- **`.jsonl`** — append-only log of raw observations from agents

Three operations drive the cycle:

1. **Read** — agent is about to modify a file, agit injects relevant knowledge (hierarchical: root → directory → file)
2. **Write** — agent finished working, appends what it learned to the log
3. **Compact** — when the log grows large, an LLM synthesizes it into clean, current knowledge

Knowledge is self-healing: if code changes without an agent, the next agent notices discrepancies and writes corrections, which eventually triggers compaction.

## Install

```bash
cargo install --path agit
```

## Quick Start

```bash
# Initialize in your project (auto-seeds from git history, registers MCP server)
agit init

# Read knowledge for a file
agit read src/auth/login.tsx

# Write an observation
agit write src/auth/login.tsx \
  --agent claude-code \
  --type decision \
  --content "Switched from JWT to session cookies because mobile WebView can't store tokens reliably."

# Check what needs attention
agit status

# Compact stale knowledge with an LLM
agit compact --stale --llm --provider anthropic
```

## MCP Server

agit exposes tools to AI agents via MCP (JSON-RPC over stdio). `agit init` auto-registers with detected agents.

```
agit_read(file_path, depth?)         — knowledge + recent log entries
agit_write(file_path, entry)         — append to log
agit_status()                        — what needs compaction
agit_compact(file_path)              — get compaction prompt (agent processes it)
agit_compact_finish(file_path, body) — save agent's compaction result
```

Supported agents: Claude Code, Cursor, VS Code (Copilot), Windsurf.

## Scope Convention

agit works at three levels:

| Scope | Path format | Shadow files |
|---|---|---|
| File | `src/auth/login.ts` | `login.ts.md` + `login.ts.jsonl` |
| Directory | `src/auth/` (trailing slash) | `_dir.md` + `_dir.jsonl` |
| Project root | `/` | `_root.md` + `_root.jsonl` |

Knowledge is hierarchical. Reading a file returns root → directory → file knowledge, general to specific.

## Git Integration

- **`.gitattributes`** — `.agit/**/*.jsonl` marked as `linguist-generated` (no PR noise), `.agit/**/*.md` uses `merge=union` (branch merges work without conflicts)
- **Post-commit hook** — `agit sync` auto-detects file renames and deletions, moves shadow files to match
- **Orphan detection** — `agit status` catches anything the hook missed

## Architecture

Borrowed from git itself:

| Git | agit |
|---|---|
| Loose objects (fast writes) | JSONL log entries (fast append) |
| Packfiles (compressed, efficient) | Knowledge `.md` (synthesized, authoritative) |
| `git gc` (compaction) | `agit compact` (compaction) |
| Threshold triggers gc | Threshold triggers compaction (~10 entries) |

## License

[MIT](LICENSE)
