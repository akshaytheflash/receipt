#!/usr/bin/env node
"use strict";

const { spawn } = require("child_process");
const path = require("path");

const exe = path.join(__dirname, "receipt.exe");
const child = spawn(exe, process.argv.slice(2), { stdio: "inherit" });

child.on("error", (err) => {
  console.error(`receipt: could not start ${exe}: ${err.message}`);
  process.exit(1);
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code === null ? 1 : code);
});
