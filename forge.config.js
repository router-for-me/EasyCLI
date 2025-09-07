const path = require('path');
const { FusesPlugin } = require('@electron-forge/plugin-fuses');
const { FuseV1Options, FuseVersion } = require('@electron/fuses');

module.exports = {
  packagerConfig: {
    asar: true,
    icon: path.join(__dirname, 'images/icon'),
  },
  rebuildConfig: {},
  makers: [
    {
      name: '@electron-forge/maker-squirrel',
      config: {
        // iconUrl must be an HTTP(S) URL to a .ico used by Squirrel's Update.exe
        // Using Electron's official icon as a stable default; replace with your own if desired
        iconUrl: 'https://raw.githubusercontent.com/electron/electron/main/shell/browser/resources/win/electron.ico',
        // setupIcon must be a local .ico path
        setupIcon: path.join(__dirname, 'images', 'icon.ico'),
      },
    },
    {
      name: '@electron-forge/maker-zip',
      platforms: ['darwin'],
      config: {
        icon: './images/icon.png',
      },
    },
    {
      name: '@electron-forge/maker-deb',
      config: {
        icon: './images/icon.png',
      },
    },
    {
      name: '@electron-forge/maker-rpm',
      config: {
        icon: './images/icon.png',
      },
    },
  ],
  plugins: [
    {
      name: '@electron-forge/plugin-auto-unpack-natives',
      config: {},
    },
    // Fuses are used to enable/disable various Electron functionality
    // at package time, before code signing the application
    new FusesPlugin({
      version: FuseVersion.V1,
      [FuseV1Options.RunAsNode]: false,
      [FuseV1Options.EnableCookieEncryption]: true,
      [FuseV1Options.EnableNodeOptionsEnvironmentVariable]: false,
      [FuseV1Options.EnableNodeCliInspectArguments]: false,
      [FuseV1Options.EnableEmbeddedAsarIntegrityValidation]: true,
      [FuseV1Options.OnlyLoadAppFromAsar]: true,
    }),
  ],
};
