'use strict';
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const https = require('node:https');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');

const RELEASE_TAG = 'v' + require('../package.json').version;
const REPO = 'ebercamargo/rux';
const TARGETS = {
  'win32-x64': ['rux-windows-x86_64.zip', 'rux.exe'],
  'linux-x64': ['rux-linux-x86_64.tar.gz', 'rux'],
  'darwin-arm64': ['rux-macos-aarch64.tar.gz', 'rux'],
};
const key = process.platform + '-' + process.arch;
const target = TARGETS[key];
if (!target) {
  console.error('Unsupported RUX platform: ' + key);
  process.exit(1);
}
const [assetName, executable] = target;
const vendor = path.join(__dirname, '..', 'vendor');
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'rux-npm-'));

function download(url, redirects = 0) {
  if (redirects > 5) return Promise.reject(new Error('Too many redirects'));
  if (!url.startsWith('https://')) return Promise.reject(new Error('HTTPS required'));
  return new Promise((resolve, reject) => {
    https.get(url, {
      headers: { 'User-Agent': 'rux-lang-npm-installer', Accept: 'application/vnd.github+json' },
      timeout: 30000,
    }, response => {
      if ([301, 302, 303, 307, 308].includes(response.statusCode)) {
        response.resume();
        return resolve(download(new URL(response.headers.location, url).href, redirects + 1));
      }
      if (response.statusCode !== 200) {
        response.resume();
        return reject(new Error('HTTP ' + response.statusCode + ' for ' + url));
      }
      const chunks = [];
      response.on('data', chunk => chunks.push(chunk));
      response.on('end', () => resolve(Buffer.concat(chunks)));
      response.on('error', reject);
    }).on('timeout', function () {
      this.destroy(new Error('Download timeout'));
    }).on('error', reject);
  });
}

async function install() {
  const api = 'https://api.github.com/repos/' + REPO + '/releases/tags/' + RELEASE_TAG;
  const release = JSON.parse((await download(api)).toString('utf8'));
  const assets = new Map((release.assets || []).map(asset => [asset.name, asset.browser_download_url]));
  if (!assets.has(assetName) || !assets.has(assetName + '.sha256')) {
    throw new Error('Release ' + RELEASE_TAG + ' is missing artifacts for ' + key);
  }
  const [archive, checksum] = await Promise.all([
    download(assets.get(assetName)),
    download(assets.get(assetName + '.sha256')),
  ]);
  const expected = checksum.toString('utf8').trim().split(/\s+/)[0].toLowerCase();
  if (!/^[a-f0-9]{64}$/.test(expected)) throw new Error('Invalid SHA256 checksum file');
  const actual = crypto.createHash('sha256').update(archive).digest('hex');
  if (actual !== expected) throw new Error('SHA256 mismatch; installation aborted');

  const archivePath = path.join(tmp, assetName);
  fs.writeFileSync(archivePath, archive);
  if (process.platform === 'win32') {
    execFileSync('powershell.exe', [
      '-NoProfile', '-NonInteractive', '-Command',
      'Expand-Archive -LiteralPath ' + JSON.stringify(archivePath) + ' -DestinationPath ' + JSON.stringify(tmp) + ' -Force',
    ]);
  } else {
    execFileSync('tar', ['-xzf', archivePath, '-C', tmp]);
  }
  const source = path.join(tmp, executable);
  if (!fs.existsSync(source)) throw new Error('Missing ' + executable + ' in release archive');
  fs.mkdirSync(vendor, { recursive: true });
  fs.copyFileSync(source, path.join(vendor, executable));
  if (process.platform !== 'win32') fs.chmodSync(path.join(vendor, executable), 0o755);
  console.log('RUX ' + RELEASE_TAG + ' installed for ' + key);
}

install().catch(error => {
  console.error('RUX installation failed: ' + error.message);
  console.error('A matching GitHub Release must be published before npm installation.');
  process.exitCode = 1;
}).finally(() => fs.rmSync(tmp, { recursive: true, force: true }));
