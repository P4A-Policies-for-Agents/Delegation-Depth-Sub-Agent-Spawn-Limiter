#!/usr/bin/env python3
"""
Multi-agent delegation simulation for the Delegation Depth & Sub-Agent Spawn
Limiter demo.

Two scenarios, run against the governed gateway:

  Phase 1 — DEEP CHAIN: a root agent delegates a subtask to a sub-agent, which
  delegates to its own sub-agent, and so on. Each hop calls the MCP tool
  `run_subtask` carrying a delegation header {root, depth, agent, parent}. The
  gateway allows the chain up to maxDepth and then breaks it (HTTP 403 /
  JSON-RPC -32005) — so a runaway recursive delegation is stopped.

  Phase 2 — WIDE FAN-OUT: one root spawns many *distinct* sub-agents. The gateway
  allows up to maxFanout distinct sub-agents per root within the window, then
  blocks further spawns.

Dependency-free (stdlib only) so it runs live without pip.

Usage:
    DD_GW_URL="https://<host>/delegation-depth-demo/mcp" python3 agent.py
    # or: python3 agent.py <gateway-mcp-url>
"""
import json
import os
import ssl
import sys
import uuid
import urllib.request

GW = (sys.argv[1] if len(sys.argv) > 1 else os.environ.get("DD_GW_URL", "")).strip()
if not GW:
    sys.exit("Set DD_GW_URL (or pass the gateway MCP URL as arg1). See demo/env.local.sh.example")

_CTX = ssl.create_default_context()
_CTX.check_hostname = False
_CTX.verify_mode = ssl.CERT_NONE


def rpc(method, params=None, rpc_id=None, delegation=None):
    payload = {"jsonrpc": "2.0", "method": method}
    if rpc_id is not None:
        payload["id"] = rpc_id
    if params is not None:
        payload["params"] = params
    headers = {
        "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream",
        "Accept-Encoding": "identity",
    }
    if delegation is not None:
        headers["x-agent-delegation"] = json.dumps(delegation)
    req = urllib.request.Request(GW, data=json.dumps(payload).encode(), headers=headers, method="POST")
    try:
        resp = urllib.request.urlopen(req, timeout=20, context=_CTX)
        status, raw = resp.status, resp.read().decode()
    except urllib.error.HTTPError as e:
        status, raw = e.code, e.read().decode()
    for line in raw.splitlines():
        if line.startswith("data:"):
            raw = line[len("data:"):].strip()
            break
    try:
        return status, json.loads(raw)
    except Exception:
        return status, None


def err_msg(obj):
    if obj and isinstance(obj, dict) and obj.get("error"):
        return obj["error"].get("message", "")
    return ""


def call_subtask(delegation):
    return rpc("tools/call",
               {"name": "run_subtask", "arguments": {"taskDescription": "analyze segment"}},
               rpc_id=2, delegation=delegation)


def phase_depth():
    print("=" * 68)
    print(" PHASE 1 — DEEP DELEGATION CHAIN  (policy maxDepth = 3)")
    print("=" * 68)
    root = "chain-" + uuid.uuid4().hex[:6]
    for depth in range(0, 9):
        deleg = {"root": root, "depth": depth,
                 "agent": f"agent-{depth}", "parent": f"agent-{depth-1}" if depth else ""}
        status, obj = call_subtask(deleg)
        if status == 200:
            who = "root orchestrator" if depth == 0 else f"sub-agent (depth {depth})"
            print(f"  depth {depth}: {who} delegated run_subtask ✓  (HTTP 200)")
        else:
            print(f"  depth {depth}: ⛔ gateway blocked delegation — {err_msg(obj)}  (HTTP {status})")
            print(f"\n🛑 The delegation chain was cut at depth {depth}. "
                  f"A runaway recursive delegation cannot go deeper.")
            return
    print("\n⚠️  chain reached its own cap without the policy tripping.")


def phase_fanout():
    print("\n" + "=" * 68)
    print(" PHASE 2 — WIDE SUB-AGENT FAN-OUT  (policy maxFanout = 4)")
    print("=" * 68)
    root = "fanout-" + uuid.uuid4().hex[:6]
    for i in range(1, 9):
        deleg = {"root": root, "depth": 1, "agent": f"worker-{i}", "parent": "orchestrator"}
        status, obj = call_subtask(deleg)
        if status == 200:
            print(f"  spawn #{i}: sub-agent 'worker-{i}' ran run_subtask ✓  (HTTP 200)")
        else:
            print(f"  spawn #{i}: ⛔ gateway blocked spawn — {err_msg(obj)}  (HTTP {status})")
            print(f"\n🛑 The root was stopped from spawning more than the allowed "
                  f"number of distinct sub-agents.")
            return
    print("\n⚠️  fan-out reached its own cap without the policy tripping "
          "(note: fan-out state is per gateway replica — see README).")


def main():
    print(f"🤖 multi-agent orchestrator  →  {GW}\n")
    rpc("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                       "clientInfo": {"name": "orchestrator", "version": "1"}}, rpc_id=1)
    rpc("notifications/initialized")
    phase_depth()
    phase_fanout()


if __name__ == "__main__":
    main()
