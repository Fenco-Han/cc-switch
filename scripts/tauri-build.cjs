// Direct Tauri CLI runner that bypasses the bin-name detection bug in tauri.js
// (DSH pretends node's argv0 is "DeepSeek Harness.exe", which tauri.js then
// injects into args → `unrecognized subcommand`).
const cli = require('../node_modules/@tauri-apps/cli');

const args = process.argv.slice(2);

cli.run(args, 'cargo tauri').catch((err) => {
  console.error(err && err.message ? err.message : err);
  process.exit(1);
});
