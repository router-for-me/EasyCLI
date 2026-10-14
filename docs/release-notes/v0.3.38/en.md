- If you have any questions or suggestions about the software, please contact us on [Discord](https://discord.gg/PxvX4D9kgs) or submit them through [GitHub Issues](https://github.com/router-for-me/EasyCLIProxyAPI/issues), and we will address them as soon as possible.

## Added

- Added Warm Sand, Graphite, Midnight Blue, and Sakura themes. Choose separate light and dark themes and switch between them automatically with the system appearance.

## Improved

- Simplified the sidebar Appearance button's background, border, and icons to better match the surrounding controls.
- Core updates now finish downloading and extracting before stopping the running proxy, reducing downtime. If the new version fails to start, the update can roll back to the previous version.
- Improved Codex CLI service restarts: other page controls remain usable while running tasks finish, with a waiting indicator and protection against duplicate restarts.

## Fixed

- Fixed core update retries potentially overwriting rollback backups of the previous version.
- Fixed macOS updates potentially hanging on DMG eject commands, and improved cleanup of child processes and temporary directories after timeouts.
- Fixed theme rendering at startup and migration of saved themes, and made page backgrounds more consistent.
- When Codex API keys are configured and no image generation policy has been manually selected, the default is now `chat` mode. Manually saved policies are preserved instead of being overwritten by defaults on restart.

# Selected Previous Updates

## Added

- Added individual spending limit details for Codex Business accounts, including used and total credits, remaining percentage, and reset time.
- Added support for restarting the Codex CLI app-server.
- Added support for configuring VS Code Copilot.
- Added experimental context editing functionality.

## Improved

- Codex model catalogs now refresh during startup update checks and manual version checks. If the catalog download fails, the current configuration remains available and the version check continues.
- Updated bundled Codex model templates and default parameters. Saved custom configurations are preserved across catalog updates and app restarts.
- Simplified restoring model defaults with a single “Use defaults” action. Click Save to apply the restored values.

## Fixed

- Fixed quota queries reporting unrecognized data for some Codex Business accounts that return individual spending limits.
