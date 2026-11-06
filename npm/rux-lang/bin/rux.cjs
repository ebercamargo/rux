#!/usr/bin/env node
'use strict';
const path = require('node:path');
const fs = require('node:fs');
const { spawnSync } = require('node:child_process');

const binary = path.join(__dirname, '..', 'vendor', process.platform === 'win32' ? 'rux.exe' : 'rux');
if (!fs.existsSync(binary)) {
  console.error('RUX executable not installed. Reinstall with: npm install -g rux-lang');
  process.exit(1);
}
const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit', windowsHide: false });
if (result.error) {
  console.error('Failed to start RUX:', result.error.message);
  process.exit(1);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
} else {
  process.exit(result.status ?? 1);
}
