#!/usr/bin/env node
const { spawn } = require("child_process");
const { getBinaryPath } = require("./index.js");

const child = spawn(getBinaryPath(), process.argv.slice(2), {
  stdio: "inherit",
});

child.on("close", (code) => process.exit(code ?? 0));
