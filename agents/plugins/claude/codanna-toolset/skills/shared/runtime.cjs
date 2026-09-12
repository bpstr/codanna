'use strict';
// Non-installing prerequisite checks, shared by both visualization entrypoints.
const { execFileSync } = require('node:child_process');
const MIN_NODE = '22.16.0';
const MIN_CODANNA = '0.16.0';

function atLeast(version, minimum) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) return false;
  const actual = version.split('.').map(Number);
  const required = minimum.split('.').map(Number);
  for (let i = 0; i < required.length; i++) {
    if (actual[i] !== required[i]) return actual[i] > required[i];
  }
  return true;
}

function assertNode(version = process.versions.node) {
  if (!atLeast(version, MIN_NODE)) {
    throw new Error(`Codanna visualizations require Node >= ${MIN_NODE}; found ${version}. Select a supported Node runtime, then rerun. Nothing was installed.`);
  }
}

function assertBinary(binary = 'codanna', workingDir = process.cwd()) {
  assertNode();
  let output;
  try {
    output = execFileSync(binary, ['--version'], {
      cwd: workingDir, encoding: 'utf8', timeout: 5000, maxBuffer: 16384,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
  } catch (error) {
    throw new Error(`Cannot check Codanna executable (${error.code || error.status || 'failed'}). Install/update your trusted Codanna build and select it with --binary PATH. Nothing was installed.`);
  }
  const match = output.trim().match(/^codanna (\d+\.\d+\.\d+)(?=\s|$)/);
  if (!match || !atLeast(match[1], MIN_CODANNA)) {
    throw new Error(`Visualizations require a release Codanna >= ${MIN_CODANNA}. Check codanna --version or select --binary PATH. Nothing was installed.`);
  }
  return match[1];
}

module.exports = { assertNode, assertBinary, atLeast, MIN_NODE, MIN_CODANNA };

if (require.main === module) {
  try {
    const args = process.argv.slice(2);
    assertNode();
    if (args.length === 1 && args[0] === '--node-only') {
      console.log(`Node ${process.versions.node}: supported`);
    } else if (args.length === 0 || (args.length === 2 && args[0] === '--binary')) {
      const version = assertBinary(args[1] || 'codanna');
      console.log(`Node ${process.versions.node}; Codanna ${version}: supported`);
    } else {
      throw new Error('Usage: node runtime.cjs [--binary PATH | --node-only]');
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
