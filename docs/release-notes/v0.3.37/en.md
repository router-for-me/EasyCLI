- If you have any questions or suggestions about the software, please contact us on [Discord](https://discord.gg/PxvX4D9kgs) or submit them through [GitHub Issues](https://github.com/router-for-me/EasyCLIProxyAPI/issues), and we will address them as soon as possible.

## Added

- Added individual spending limit details for Codex Business accounts, including used and total credits, remaining percentage, and reset time.

## Improved

- Codex model catalogs now refresh during startup update checks and manual version checks. If the catalog download fails, the current configuration remains available and the version check continues.
- Updated bundled Codex model templates and default parameters. Saved custom configurations are preserved across catalog updates and app restarts.
- Simplified restoring model defaults with a single “Use defaults” action. Click Save to apply the restored values.

## Fixed

- Fixed quota queries reporting unrecognized data for some Codex Business accounts that return individual spending limits.

# Selected Previous Updates

## Added

- Added support for restarting the Codex CLI app-server.
- Added support for configuring VS Code Copilot.
- Added experimental context editing functionality.

## Improved

- Improved the VS Code Copilot icon display.
- Improved how usage records are displayed.
- Improved the process of adding custom APIs.
