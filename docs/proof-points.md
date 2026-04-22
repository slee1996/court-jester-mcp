# Proof Points

These are the strongest concrete repo-shaped wins to reference when explaining why Court Jester is worth wiring into an agent loop or CI job.

## Real Finds

- `correlation.ts`
  - Court Jester found a crash-a-request denial-of-service path triggered by a CRLF-injected `X-Correlation-Id` header.
  - This shipped code had no covering test and no useful lint signal.

- `authz/check.ts:buildCacheKey`
  - Court Jester found a nil-deref under malformed `CheckAccessParams`.
  - This was reached through factory-to-nested fuzzing rather than a direct top-level helper call.

- Complexity hotspots
  - `edge-router:fetch` measured cognitive complexity `35`
  - `projected-prices:syncProjectedPricesForTenant` measured cognitive complexity `43`
  - These were real maintainability hotspots that normal style linting did not surface clearly.

- Repeated Node portability issues
  - Multiple files tripped strict Node ESM portability failures that only became obvious once portability was split from execute behavior.

## Auditable Green

- Security-boundary files including `hmac`, `bearer-secret`, `clerk-webhook`, and search normalization paths survived roughly `575` fuzz executions with zero crashes.
- The important part is not just that they stayed green. The `coverage` stage made that green auditable by showing what was actually fuzzed, skipped, or blocked.

## Why These Matter

- The value case is not only “Court Jester found a bug.”
- The stronger claim is:
  - it can find real hidden defects
  - it can explain what it actually exercised
  - and it can produce defensible green results instead of silent skip-driven green
