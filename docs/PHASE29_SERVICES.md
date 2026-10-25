# Phase 29 — Services (Offline)

External Studio parity: `UNTESTED`. Nothing here talks to Roblox
cloud services, and nothing fakes that it does.

## DataStoreService (`PARTIAL`, local emulation)
- `GetDataStore(name, scope?)`, `GetOrderedDataStore` (same store;
  `GetSortedAsync` errors explicitly as unsupported).
- `GetAsync/SetAsync/UpdateAsync/IncrementAsync/RemoveAsync` with
  Roblox-shaped semantics: 1-128 char keys, nil for missing keys,
  UpdateAsync transform semantics, values must be JSON-serializable.
- `GetRequestBudgetForRequestType` returns the local budget (60).
- Server-only: client contexts error. Persistence is a JSON file in
  `<place>.bloxdata/datastores.json`, written atomically
  (temp+rename) on every mutation and loaded by each new Runtime, so
  data survives engine restarts (`run`, `play`; net server wiring
  uses the same `runtime/begin` parameter). This is labelled local
  emulation everywhere; it is not Roblox production DataStore access.
- Not implemented: versions/list APIs, OrderedDataStore paging,
  cross-process locking for simultaneous writers, throttling/failure
  injection beyond budget reporting.

## HttpService (`PARTIAL`)
- Local: `JSONEncode`, `JSONDecode`, `GenerateGUID`.
- `GetAsync/PostAsync/RequestAsync` error explicitly: RBXFoundry
  performs no outbound network access. No fixture HTTP transport is
  provided in this milestone.

## MarketplaceService / TeleportService (`UNSUPPORTED`)
No purchase prompts, receipts, or teleports are simulated; no Robux
or entitlement claims are made. APIs remain absent rather than
returning fabricated success.

## Plugin API (`PARTIAL` via CLI/MCP)
Studio editing from scripts is available through the CLI/MCP surface
(instance ops, ChangeHistory via undo/redo history, selection in
Phase 25 sessions). DockWidgetPluginGui and toolbar UI semantics are
not reproduced (no GUI editor exists); plugin Luau context beyond
the existing runtime contexts is not provided.

## Tests
`crates/bloxcli/tests/phase29.rs`: persistence across two engine
processes, UpdateAsync, IncrementAsync, key validation, budget,
JSON round-trip, GUID shape, offline request rejection.
