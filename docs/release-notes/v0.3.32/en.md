## Added

- Added a copy button for request error details, available in both collapsed and expanded views, to simplify troubleshooting and sharing feedback.

## Improved

- Grouped advanced API provider settings into model rules, connection and routing, and request behavior for easier navigation.
- Simplified usage filters to time range, model, source, API key, and result. The default request table now shows request key information, preferring its note when available.
- Windows automatic CLI launches now use a persistent PowerShell session for Claude Code and other clients, preserving the working directory and environment settings to improve terminal compatibility compared with direct launches ([#393](https://github.com/router-for-me/EasyCLIProxyAPI/issues/393)).

## Fixed

- Fixed usage filter dropdowns being offset or clipped by page containers and long menus jumping back to the top after scrolling. Menu scroll positions are preserved when resizing the window or scrolling a parent container ([#396](https://github.com/router-for-me/EasyCLIProxyAPI/issues/396)).
- Corrected overview TPS to use the arithmetic mean of valid per-request generation speeds.
- Fixed incomplete resolution of credential and API provider source notes, and separated request key labels from upstream sources to avoid mixing their notes.
- Fixed filter lists not updating after adding keys or changing notes. Manual and periodic refreshes now update the options while preserving the selected key.
- Fixed Oh My Pi relying solely on a launch-time environment variable for its API key. The key is now saved in `models.yml`, so manual terminal launches can also use the configured key.
