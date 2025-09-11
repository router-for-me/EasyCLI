/* Copy web assets to an isolated folder for Tauri */
const fs = require('fs');
const path = require('path');

function copyRecursive(src, dest) {
    if (!fs.existsSync(src)) return;
    const stat = fs.statSync(src);
    if (stat.isDirectory()) {
        if (!fs.existsSync(dest)) fs.mkdirSync(dest, { recursive: true });
        for (const entry of fs.readdirSync(src)) {
            copyRecursive(path.join(src, entry), path.join(dest, entry));
        }
    } else {
        const dir = path.dirname(dest);
        if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
        fs.copyFileSync(src, dest);
    }
}

// src-tauri cwd -> project root is one level up
const projectRoot = path.resolve(__dirname, '..');
const outDir = path.join(projectRoot, 'dist-web');

if (fs.existsSync(outDir)) {
    fs.rmSync(outDir, { recursive: true, force: true });
}
fs.mkdirSync(outDir, { recursive: true });

const includeFiles = [
    'login.html',
    'settings.html',
];
const includeDirs = [
    'css',
    'js',
    'images',
];

for (const f of includeFiles) {
    copyRecursive(path.join(projectRoot, f), path.join(outDir, f));
}
for (const d of includeDirs) {
    copyRecursive(path.join(projectRoot, d), path.join(outDir, d));
}

console.log('Prepared dist-web for Tauri:', outDir);

// Ensure Tauri default window icon exists to satisfy tauri-build
try {
    const iconsDir = path.join(__dirname, 'icons');
    if (!fs.existsSync(iconsDir)) fs.mkdirSync(iconsDir, { recursive: true });
    const pngSrc = path.join(projectRoot, 'images', 'icon.png');
    const pngDest = path.join(iconsDir, 'icon.png');
    if (fs.existsSync(pngSrc)) {
        fs.copyFileSync(pngSrc, pngDest);
    }
    const icnsSrc = path.join(projectRoot, 'images', 'icon.icns');
    const icnsDest = path.join(iconsDir, 'icon.icns');
    if (fs.existsSync(icnsSrc)) {
        fs.copyFileSync(icnsSrc, icnsDest);
    }
    const icoSrc = path.join(projectRoot, 'images', 'icon.ico');
    const icoDest = path.join(iconsDir, 'icon.ico');
    if (fs.existsSync(icoSrc)) {
        fs.copyFileSync(icoSrc, icoDest);
    }
    console.log('Ensured icons in', iconsDir);
} catch (e) {
    console.warn('Failed to ensure icons for tauri-build:', e.message);
}
