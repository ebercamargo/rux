#!/usr/bin/env node
"use strict";
const { spawnSync } = require("node:child_process");
const path = require("node:path");
const platform = process.platform;
const binary = platform === "win32" ? "rux.exe" : platform === "linux" ? "rux" : null;
if (!binary || process.arch !== "x64") {
  console.error("RUX alpha supports Windows/Linux x64 only.");
  process.exit(1);
}
const executable = path.join(__dirname, "..", "native", platform, binary);
const result = spawnSync(executable, process.argv.slice(2), { stdio: "inherit", windowsHide: false });
if (result.error) {
  console.error("Unable to start RUX native binary:", result.error.message);
  process.exit(1);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
} else {
  process.exit(result.status === null ? 1 : result.status);
}
