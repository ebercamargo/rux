'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const pkg = require('../package.json');
test('package exposes rux CLI', () => {
  assert.equal(pkg.name, 'rux-lang');
  assert.equal(pkg.bin.rux, 'bin/rux.cjs');
  assert.equal(pkg.scripts.postinstall, 'node scripts/install.cjs');
});
