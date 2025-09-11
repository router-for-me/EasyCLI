/* Watch and copy web assets to dist-web for Tauri dev */
const fs = require('fs');
const path = require('path');
const chokidar = require('chokidar');

function copyFile(src, dest) {
  const dir = path.dirname(dest);
  if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
  fs.copyFileSync(src, dest);
  console.log('[copy]', src, '->', dest);
}

function removePath(p) {
  if (fs.existsSync(p)) {
    const stat = fs.statSync(p);
    if (stat.isDirectory()) {
      fs.rmSync(p, { recursive: true, force: true });
    } else {
      fs.unlinkSync(p);
    }
    console.log('[remove]', p);
  }
}

function copyRecursive(src, dest) {
  if (!fs.existsSync(src)) return;
  const stat = fs.statSync(src);
  if (stat.isDirectory()) {
    for (const entry of fs.readdirSync(src)) {
      copyRecursive(path.join(src, entry), path.join(dest, entry));
    }
  } else {
    copyFile(src, dest);
  }
}

const root = path.resolve(__dirname, '..');
const outDir = path.join(root, 'dist-web');

// initial sync
for (const p of ['login.html', 'settings.html', 'css', 'js', 'images']) {
  copyRecursive(path.join(root, p), path.join(outDir, p));
}
console.log('[watch] initial sync complete ->', outDir);

const watcher = chokidar.watch([
  path.join(root, 'login.html'),
  path.join(root, 'settings.html'),
  path.join(root, 'css'),
  path.join(root, 'js'),
  path.join(root, 'images')
], { ignoreInitial: true });

watcher.on('add', (p) => {
  const rel = path.relative(root, p);
  copyRecursive(p, path.join(outDir, rel));
});
watcher.on('change', (p) => {
  const rel = path.relative(root, p);
  copyRecursive(p, path.join(outDir, rel));
});
watcher.on('unlink', (p) => {
  const rel = path.relative(root, p);
  removePath(path.join(outDir, rel));
});
watcher.on('unlinkDir', (p) => {
  const rel = path.relative(root, p);
  removePath(path.join(outDir, rel));
});

console.log('[watch] watching web assets...');
// keep process alive
process.stdin.resume();

