# Demo provisioning runbook

Steps to stand up the live Delegation Depth & Sub-Agent Spawn Limiter demo,
following `p4a-test-mcp-policies-with-a2d`. Done with `anypoint-cli-v4` (already
authenticated) + the A2D MCP tools — **no bearer token / connected-app secret
needed**. Replace `<...>` placeholders with your own (identifiers, not secrets).

| Placeholder | What it is | How to get it |
|---|---|---|
| `<orgId>` | Business-group / org id | `anypoint-cli-v4 account:business-group:list` |
| `<mockServerId>` | A2D mock MCP server id | returned by `design_mcp_server` |
| `<gatewayId>` | Managed Flex Gateway **resource** id | `runtime-mgr:gateways:managed:list --environment Sandbox` |
| `<gatewayPublicHost>` | Gateway public ingress host | `runtime-mgr:gateways:managed:describe <gatewayId>` → `configuration.ingress.publicUrl` |
| `<apiInstanceId>` | API Manager instance id | returned by `api-mgr:api:manage` |

Mock surface: `https://www.a2d-ai.com/api/platform/<mockServerId>/mcp`
Governed endpoint: `https://<gatewayPublicHost>/delegation-depth-demo/mcp`
Policy version: **1.0.1**

## 1. A2D mock (A2D MCP tools)

`design_mcp_server` (type `mock`, provider org + URL) → `add_mcp_tool`
`run_subtask` (schemas meet A2D's quality gate: tool desc ≥200, property desc ≥25,
property name ≥5) returning `{"resultSummary":"subtask complete"}`.

## 2. Publish the manifest to Exchange as `type=mcp`

```bash
anypoint-cli-v4 exchange:asset:upload \
  --name "Delegation Depth Test Server" --type mcp --status published \
  --description "Mock task-execution MCP server for the Delegation Depth & Sub-Agent Spawn Limiter demo" \
  --properties='{"platform":"a2d"}' \
  --files='{"mcp-metadata.json":"./mcp-metadata.json"}' \
  delegation-depth-test-server/1.0.0
```

## 3. Create + deploy the MCP Flex API instance

```bash
anypoint-cli-v4 api-mgr:api:manage delegation-depth-test-server 1.0.0 <orgId> \
  --environment Sandbox --isFlex --type mcp \
  --uri "https://www.a2d-ai.com/api/platform/<mockServerId>/" \
  --apiInstanceLabel "delegation-depth-demo"

anypoint-cli-v4 api-mgr:api:edit <apiInstanceId> --environment Sandbox --isFlex --type mcp \
  --withProxy --scheme http --port 8081 --path "/delegation-depth-demo/" \
  --uri "https://www.a2d-ai.com/api/platform/<mockServerId>/"

# target = the GATEWAY resource id (not its targetId)
anypoint-cli-v4 api-mgr:api:deploy <apiInstanceId> --environment Sandbox \
  --target <gatewayId> --gatewayVersion 1.0.0 --overwrite
```

## 4. Apply the policy (v1.0.1)

```bash
anypoint-cli-v4 api-mgr:policy:apply <apiInstanceId> delegation-depth-limiter \
  --environment Sandbox --groupId <orgId> \
  --policyVersion 1.0.1 --configFile ./config.json
anypoint-cli-v4 api-mgr:api:redeploy <apiInstanceId> --environment Sandbox
```

## 5. Run the agent

```bash
cp env.local.sh.example env.local.sh   # set DD_GW_URL to your governed endpoint
./demo.sh
```

Expected: chain allowed to depth 3, blocked at depth 4 (`exceeds limit 3`);
fan-out allowed up to the limit, then blocked (`fan-out N exceeds limit 4`).

## Notes

- **Inbound & request-gating**; no response rewrite → **MCP Support not required**.
- **Depth is stateless** (exact/deterministic). **Fan-out is per gateway replica**
  (local storage); on a multi-replica gateway the block lands after ~`maxFanout ×
  replicas` distinct spawns. Use `remote` (Redis) storage for exact global counts.
- Upstream URI is the mock surface **minus** `/mcp`; deploy target is the gateway
  **resource** id; `api:manage` alone leaves `deployment: null` until
  `api:edit --withProxy` + `api:deploy`.
- Implementation asset description must be ≤256 chars → policy published at 1.0.1.
