#!/usr/bin/env python3
"""Replay `agent-preferred-model`'s judge over recent Agent/Task launches.

Reads Claude Code transcripts (main and subagent) modified in the last N days
(default 14) and judges each launch against the agent definitions installed
today in the config directory: a fire is a launch of a definition that says
`model: inherit` and declares `preferred-model`, with no model on the call and
no `model-fallback:` first line. Prints the launch count, the reviewer
launches without a model, the fires (expected equal) and the fires by type.
Read-only.
"""
import collections
import glob
import json
import os
import re
import sys
import time


def definitions(config):
    found = {}
    for path in glob.glob(os.path.join(config, "agents", "*.md")):
        parts = open(path, encoding="utf-8", errors="replace").read().split("---", 2)
        if len(parts) < 3:
            continue
        model = re.search(r"^model:\s*(\S+)", parts[1], re.M)
        preferred = re.search(r"^preferred-model:\s*(\S+)", parts[1], re.M)
        found[os.path.basename(path)[:-3]] = (
            model and model.group(1),
            preferred and preferred.group(1),
        )
    return found


def main():
    days = int(sys.argv[1]) if len(sys.argv) > 1 else 14
    config = os.environ.get("CLAUDE_CONFIG_DIR") or os.path.expanduser("~/.claude")
    agents = definitions(config)
    cutoff = time.time() - days * 86400
    transcripts = glob.glob(os.path.join(config, "projects", "*", "*.jsonl"))
    transcripts += glob.glob(os.path.join(config, "projects", "*", "*", "subagents", "*.jsonl"))
    launches = without_model = 0
    fires = collections.Counter()
    for path in transcripts:
        if os.path.getmtime(path) < cutoff:
            continue
        for line in open(path, encoding="utf-8", errors="replace"):
            if '"tool_use"' not in line or "subagent_type" not in line:
                continue
            try:
                entry = json.loads(line)
            except ValueError:
                continue
            for block in (entry.get("message") or {}).get("content") or []:
                if not isinstance(block, dict) or block.get("type") != "tool_use":
                    continue
                if block.get("name") not in ("Agent", "Task"):
                    continue
                call = block.get("input") or {}
                kind = call.get("subagent_type") or ""
                launches += 1
                model, preferred = agents.get(kind, (None, None))
                no_model = not (call.get("model") or "").strip()
                prompt = (call.get("prompt") or "").lstrip("﻿").lstrip()
                if preferred and no_model:
                    without_model += 1
                if model == "inherit" and preferred and no_model and not prompt.startswith("model-fallback:"):
                    fires[kind] += 1
    print(f"Agent/Task launches ({days}d): {launches}")
    print(f"reviewer launches without a model: {without_model}; fires: {sum(fires.values())}")
    print("fires by type:", dict(fires.most_common()))


if __name__ == "__main__":
    main()
