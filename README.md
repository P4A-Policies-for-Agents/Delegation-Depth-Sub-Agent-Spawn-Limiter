# Delegation Depth & Sub-Agent Spawn Limiter — MuleSoft Omni/Flex Gateway Policy

An **MCP-native, inbound** custom policy for the MuleSoft Omni/Flex Gateway. It
enforces a **hard ceiling on how *deep* a multi-agent delegation chain may go and
how *wide* it may fan out** — by inspecting a delegation header on every MCP
`tools/call`. It stops runaway agent subcontracting (an agent spawning sub-agents
that spawn sub-agents…) and sub-agent explosion before they exhaust budget.
Addresses **OWASP ASI-T07 Authority Drift**.

Built with the PDK, Rust → `wasm32-wasip1`, split-model.

---

## Table of contents

- [Why this policy](#why-this-policy)
- [The delegation header](#the-delegation-header)
- [How it decides](#how-it-decides)
- [Configuration reference](#configuration-reference)
- [Depth vs. fan-out — state model](#depth-vs-fan-out--state-model)
- [Repository layout](#repository-layout)
- [Build, test & release](#build-test--release)
- [Live demo (multi-agent chain)](#live-demo-multi-agent-chain)
- [Design notes & gotchas](#design-notes--gotchas)
- [Skills used](#skills-used)

---

## Why this policy

In a multi-agent system an agent can delegate to sub-agents, which delegate
further. Two failure modes follow: **unbounded depth** (recursive delegation that
never terminates) and **unbounded fan-out** (one agent spawning a swarm of
sub-agents). Both burn budget and drift away from the original human authorization.
Frameworks (LangChain, CrewAI) bury these limits in code the gateway can't see —
this policy makes the ceiling an enforced, portable gateway control.

> **Key caveat — the ceiling is only as trustworthy as the hop signal.** The
> policy reads a delegation header that cooperating agents stamp and increment as
> they spawn children. Position the gateway as the **choke point** for agent
> traffic; an agent that never traverses it can't be counted. Header signing
> (HMAC/JWT) to make hops unforgeable is a documented extension — today the header
> is advisory (pair with an auth policy if you need it tamper-evident).

---

## The delegation header

Each agent sends `x-agent-delegation` (configurable) as JSON:

```json
{ "root": "chain-abc123", "depth": 2, "agent": "researcher", "parent": "planner" }
```

- `root` — id of the originating chain (fan-out is counted per root).
- `depth` — how many delegation hops deep this call is (root = 0).
- `agent` — this agent's id (fan-out counts distinct agents per root).
- `parent` — the delegating agent (provenance/telemetry).

A **missing** header is treated as a **root call** (depth 0). Cooperating agents
increment `depth` and set a fresh `agent` id when they spawn a child.

---

## How it decides

For each MCP `tools/call`:

```
parse x-agent-delegation  (absent → root, depth 0; malformed → reject)

depth  > maxDepth                         → DENY   (stateless)
fanout: distinct agents seen for `root`
        within windowMillis  > maxFanout  → DENY   (per-root, shared storage)
otherwise                                 → ALLOW
```

Denial is **HTTP 403** with an in-band JSON-RPC error (`-32005`):

```json
{ "jsonrpc":"2.0", "id":2, "error": { "code":-32005,
  "message":"delegation depth 4 exceeds limit 3" } }
```

Non-`tools/call` methods (`initialize`, `tools/list`, notifications) pass through.
`onExceed: report` logs instead of blocking (for tuning).

---

## Configuration reference

| Property | Type | Default | Description |
|---|---|---|---|
| `maxDepth` | integer (≥0) | `5` | Max delegation depth. `0` = only root calls (no delegation). |
| `maxFanout` | integer (≥0) | `10` | Max **distinct** sub-agents per root within the window. `0` disables fan-out enforcement. |
| `windowMillis` | integer (≥1000) | `60000` | Window over which distinct sub-agents per root are counted. |
| `headerName` | string | `x-agent-delegation` | Delegation-context header. |
| `onExceed` | `reject` \| `report` | `reject` | Break the call, or only log. |

---

## Depth vs. fan-out — state model

- **Depth is stateless** — a pure comparison of the header's `depth` to `maxDepth`.
  It trips at *exactly* the limit, deterministically, on every replica.
- **Fan-out is stateful** — it counts distinct `agent` ids per `root` in PDK
  **local** shared storage (in-memory, **per gateway replica**). On a multi-replica
  managed gateway each replica counts independently, so the block may land after
  roughly `maxFanout × replicas` distinct spawns; the fan-out is still bounded.
  For **exact global** fan-out counting, switch to `store_builder.remote(ns, ttl)`
  (Redis) — a one-line change in `configure`.

---

## Repository layout

```
delegation-depth-limiter-definition/   # gcl.yaml (schema), exchange.json, Makefile
delegation-depth-limiter-flex/          # Rust implementation
  src/lib.rs         # entrypoint (injects DataStorageBuilder) + inbound filter
  src/delegation.rs  # PURE: header parse + depth check + distinct fan-out — unit tested
  src/generated/     # config.rs generated from gcl.yaml
  tests/requests.rs  # Docker-based integration test (make test)
demo/
  mcp-metadata.json     # MCP manifest published to Exchange as type=mcp
  config.json           # policy config applied in the demo (maxDepth 3, maxFanout 4)
  agent.py              # multi-agent orchestrator: deep chain + wide fan-out (LIVE demo)
  demo.sh               # runs agent.py against the governed endpoint
  env.local.sh.example  # copy to env.local.sh (gitignored) with your endpoint
  PROVISION.md          # exact anypoint-cli + A2D commands to reproduce
```

---

## Build, test & release

```bash
cd delegation-depth-limiter-definition && make release   # publish definition
cd ../delegation-depth-limiter-flex
make build-asset-files
cargo build --target wasm32-wasip1 --release
cargo test --lib            # 6 pure unit tests (parse, depth ceiling, distinct fan-out)
make release                # publish implementation
```

> Note: the Exchange **implementation** asset description must be ≤256 chars; the
> policy is published at **1.0.1** (the initial 1.0.0 definition description was
> shortened to satisfy this).

---

## Live demo (multi-agent chain)

A mock MCP server exposes `run_subtask`. A **Python orchestrator** (`demo/agent.py`)
plays a real multi-agent workflow against the governed gateway
(`maxDepth 3`, `maxFanout 4`):

```bash
cp demo/env.local.sh.example demo/env.local.sh   # set DD_GW_URL to your governed endpoint
./demo/demo.sh
```

Observed live output:

```
 PHASE 1 — DEEP DELEGATION CHAIN  (policy maxDepth = 3)
  depth 0: root orchestrator delegated run_subtask ✓  (HTTP 200)
  depth 1: sub-agent (depth 1) delegated run_subtask ✓  (HTTP 200)
  depth 2: sub-agent (depth 2) delegated run_subtask ✓  (HTTP 200)
  depth 3: sub-agent (depth 3) delegated run_subtask ✓  (HTTP 200)
  depth 4: ⛔ gateway blocked delegation — delegation depth 4 exceeds limit 3  (HTTP 403)
🛑 The delegation chain was cut at depth 4.

 PHASE 2 — WIDE SUB-AGENT FAN-OUT  (policy maxFanout = 4)
  spawn #1..#N: sub-agent 'worker-i' ran run_subtask ✓  (HTTP 200)
  spawn #N: ⛔ gateway blocked spawn — sub-agent fan-out 5 for root 'fanout-…' exceeds limit 4  (HTTP 403)
🛑 The root was stopped from spawning more sub-agents.
```

Phase 1 (depth) trips *exactly* at the limit. Phase 2 (fan-out) blocks once a
replica's distinct-agent count exceeds `maxFanout` — the spawn number varies with
replica count (see [state model](#depth-vs-fan-out--state-model)).

Full provisioning is in [`demo/PROVISION.md`](demo/PROVISION.md).

---

## Design notes & gotchas

- **Stateful half uses `DataStorageBuilder`** injected into `configure`;
  `store_builder.local(ns)` gives a per-replica store; `get → Option<(T,cas)>`,
  `store(key,&StoreMode,&T)`.
- **Deny = HTTP 403** with JSON-RPC `-32005` (server-defined) — deliberate, so a
  client can route on status.
- **Fail-open** on non-JSON / non-`tools/call` and on storage read errors; **fail
  closed** on a malformed delegation header (an untrustworthy chain is rejected).
- **MCP Support not required** — this is request-gating with no response rewrite;
  the mcp endpoint proxies MCP natively.
- **MCP-only today.** `assetTypes: mcp`, and enforcement keys on `tools/call`.
  A2A support (recognize `message/send`, key depth/fan-out off the A2A context) is
  a contained extension — the `delegation.rs` core is protocol-agnostic.

---

## Skills used

- **PDK** (`omni-gateway-pdk-skills`): `pdk-create-policy`, `pdk-mcp`,
  `pdk-data-storage`, `pdk-schema-definition`, `pdk-request-headers-bodies`,
  `pdk-stop-execution`.
- **P4A** (`p4a-skills`): `p4a-build-policy`, `p4a-verify-requirements`,
  `p4a-mcp-usage`, `p4a-test-mcp-policies-with-a2d`.
