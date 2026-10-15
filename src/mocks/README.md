# Browser preview mocks

Run `bun run dev` and open the Vite URL in a regular browser. Mock IPC is installed
only in browser development builds; Tauri and production builds use the real backend.
No backend or upstream account is required. Data is fictional and mutations are
in memory, isolated per page load. Reload to reset changes.

Use the bottom toolbar or query parameters:

| URL option | Scenario |
| --- | --- |
| `?mock=running` | Running core, configured providers, usage records and mixed credential states |
| `?mock=stopped` | Installed but stopped core; start it to explore the populated pages |
| `?mock=empty` | Uninstalled core with empty credentials, providers and usage; install/start it to inspect empty states |
| `?mock=error` | Main backend reads fail so error and retry UI can be inspected |
| `?mock=off` | Disable mock IPC |
| `?mock=running&mockDelay=500` | Delay each IPC call by 500 ms (maximum 5000 ms) |
| `?mock=running&mockPluginPages=12` | Add 12 extra plugin pages (maximum 30) so the sidebar plugin submenu auto-collapses at normal window sizes |

Plugin pages are native child webviews in the desktop app, which the browser cannot
render. The mock keeps the show, bounds, hide and close state (with the owner token contract) and draws a dashed `[data-mock-plugin-webview]` placeholder over the reported
bounds (hidden while the webview is hidden or closed). It never loads the page; its
`data-mock-url`, `data-mock-reloads` and `data-mock-labels` attributes expose the simulated state for tests.

The selected scenario is remembered locally; an explicit `mock` query overrides it.

## Credential and quota coverage

The running dataset includes more than ten OAuth files, plus a runtime credential
which the authentication-file list excludes. It covers Codex, Claude, Antigravity,
Kimi, xAI and Devin. Examples include long account names and quota labels, multiple
quota windows, a disabled account, an exhausted account with model cooldown,
an expired OAuth token, an empty upstream quota response, unknown remaining quota,
a paid API account and an account with no recent requests.

Quota replies pass through the normal application parsers. Codex and Claude reset
credits are tracked per account: consuming one decreases its balance and refreshes
its usage windows. Credential edits, enable/disable, cooldown reset, deletion,
JSON import/replacement and OAuth cancellation update the same in-memory state.
Malformed imports and missing credentials fail explicitly. Usage event filters
(time, model, provider, source, API key, failure/cancellation) run before pagination.

Unknown IPC commands, management routes and upstream URLs fail explicitly instead
of returning fake success. Add new contracts to `browserMockRuntime.ts`,
`quotaMock.ts` or `pluginMock.ts` when implementing new backend-facing UI.
Other desktop operations may still return preview-only results; the mock does not
launch real apps, write credentials to disk or make upstream API requests.

## Verification

```sh
bun test tests/browserMockRuntime.test.ts tests/browserMockQuota.test.ts
node tests/browser-mock-ui.cjs
```

The browser test uses Playwright (and Microsoft Edge on Windows), opens the real
app, and checks quota rendering, pagination and credential mutations while blocking
external HTTP requests. Its screenshot is saved under `bin-work/`.
