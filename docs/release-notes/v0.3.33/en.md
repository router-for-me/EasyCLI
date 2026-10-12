- If you have any questions or suggestions about the software, please contact us on [Discord](https://discord.gg/PxvX4D9kgs) or submit them through [GitHub Issues](https://github.com/router-for-me/EasyCLIProxyAPI/issues), and we will address them as soon as possible.

## Added

- Added a sidebar privacy mode that masks account emails, account IDs, and project IDs and remembers your preference. Masking affects display only; requests, searches, copied values, and exports remain unchanged.
- Added a GitHub access token setting under Service & Access, with show/hide controls and a link to create a token. GitHub API rate-limit messages in the plugin store now link directly to this setting.
- Added an Enable cooldowns switch under Models & Requests. Cooldowns default to off when no setting exists; explicit settings are preserved.

## Improved

- OAuth sign-in cards now show saved accounts, account counts, and disabled status, with a shortcut to authentication files. Account lists refresh after successful authorization to make multiple accounts easier to manage.
- Moved Retries & Cooldowns to Models & Requests and updated settings search navigation and widescreen layouts.
- Updated the bundled CLIProxyAPI core to v8.0.24.

## Fixed

- Corrected OAuth authorization messages to distinguish adding an account from signing in to an existing one. Signing in again updates that account's credentials; the messages no longer claim every authorization creates a new account.
- Fixed startup configuration merging potentially overwriting explicit cooldown settings with stale GUI settings or template defaults. Users' enabled or disabled choices are preserved.
