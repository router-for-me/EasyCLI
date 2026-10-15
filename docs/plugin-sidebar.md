# Pin plugin pages to the sidebar

In **Plugins > Local plugins**, tick **Pin to sidebar** beside a plugin page.
The page appears under **Pinned plugins** in the main navigation. Untick the
checkbox to remove it. Each page can be pinned independently.

Pins are off by default and saved in this desktop webview/browser profile's
localStorage (`cpa-gui-plugin-pins-v1`). They survive restarts and plugin upgrades;
clearing app/browser site data removes them. Pins store only the plugin ID and
declared resource path, never a management key, resource URL or menu index.

Disabled, unloaded and unavailable plugins are hidden. Disabling a plugin or the
global plugin feature preserves its pins. Successful catalog refreshes remove
uninstalled plugins and pages no longer declared by active plugins. Catalog
errors and unsupported/stopped cores do not erase preferences. Uninstalling from
the plugin card removes its pins immediately, even if unloading needs a restart.

Navigation resolves the current menu index from its path. Opening a shortcut uses
the existing native resource URL command, shared iframe restrictions and
**Open in browser** action. A changed menu order between the catalog lookup and
native URL lookup cannot silently open another page. Logos use existing asset
URL validation with a generic plugin icon on failure. No plugin ABI change or
plugin-specific application code is required.

The catalog refreshes when the plugin page announces a change, on window focus,
and every 15 seconds while visible and pins exist. Stopping the core hides pins
immediately through the existing runtime state.

## Verification

```sh
bun test tests/pluginPins.test.ts tests/plugins.test.ts
bun run test:ui:plugin-pins
node tests/plugins-ui.cjs
```

Browser tests follow the existing Playwright setup: install Playwright in the
test environment, or set `PLAYWRIGHT_MODULE` to its module path. Windows defaults
to installed Edge; `PLAYWRIGHT_CHANNEL` can select another installed channel.
The sidebar test runs the complete app with its in-memory browser mock and does
not connect to a real core or change installed plugins.
