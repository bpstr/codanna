'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const vm = require('node:vm');
const { execFileSync } = require('node:child_process');
const { pathToFileURL } = require('node:url');
const skills = path.resolve(__dirname, '../../agents/plugins/claude/codanna-toolset/skills');
const safety = require(path.join(skills, 'shared/safety.cjs'));
const attack = '</script><script>globalThis.INJECTED=1</script><img src=x onerror="globalThis.INJECTED=1">\u2028\u2029&';

function assertNoInjectedMarkup(html) {
  assert.ok(!html.includes(attack), 'repository text must not remain raw in markup');
  assert.ok(!html.includes('<script>globalThis.INJECTED'), 'script-end breakout survived');
  assert.ok(!html.includes('<img src=x onerror='), 'HTML text-context injection survived');
  // Parse the renderer's actual JSON-bearing script, not a reimplemented encoder.
  const match = html.match(/const (?:graphData|data) = (.*);/);
  assert.ok(match, 'expected renderer data assignment');
  assert.ok(JSON.stringify(JSON.parse(match[1])).includes('INJECTED'), 'fixture was silently discarded');
}

test('script JSON round-trips hostile text without creating an HTML delimiter', () => {
  const encoded = safety.jsonForScript({ attack });
  assert.ok(!/[<>&\u2028\u2029]/u.test(encoded));
  assert.deepEqual(JSON.parse(encoded), { attack });
  const context = {};
  vm.runInNewContext('value = ' + encoded, context);
  assert.equal(context.value.attack, attack);
  assert.equal(context.INJECTED, undefined);
});

test('all x-ray renderers encode repository names, titles, paths and signatures', () => {
  const node = { id: 1, name: attack, kind: attack, file: attack, signature: attack, level: 0, group: 0 };
  const graph = { nodes: [node], links: [] };
  const hierarchy = { name: attack, children: [{ ...node, count: 1 }] };
  const opts = { title: attack, subtitle: attack, workingDir: attack, legendKinds: [attack], details: { 1: { name: attack, signature: attack } } };
  const renderers = [
    ['render', 'generateHTML', graph],
    ['dag-render', 'generateDAGHTML', graph],
    ['tree-render', 'generateTreeHTML', hierarchy],
    ['collapse-render', 'generateCollapseHTML', hierarchy],
  ];
  for (const [file, name, data] of renderers) {
    assertNoInjectedMarkup(require(path.join(skills, 'x-ray/graph', file))[name](data, opts));
  }
  const { generateBundleHTML } = require(path.join(skills, 'x-ray/graph/bundle-render'));
  assertNoInjectedMarkup(generateBundleHTML(hierarchy, [], opts));
});

test('published page has restrictive CSP and nonces without rewriting script string literals', () => {
  const script = 'const literal = "<script should-not-change>";';
  const html = safety.secureHtml('<html><head></head><body><script>' + script + '</script></body></html>');
  assert.match(html, /Content-Security-Policy/);
  assert.match(html, /connect-src &#39;none&#39;/);
  assert.match(html, /<script nonce="[A-Za-z0-9+/]+"/);
  assert.ok(html.includes(script));
});

test('both dump readers execute a metacharacter-containing binary path as one argv item', { skip: process.platform === 'win32' }, async () => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'codanna-review-'));
  try {
    const binary = path.join(temp, 'codanna ; printf injected > marker #');
    fs.writeFileSync(binary, '#!' + process.execPath + '\n' +
      'if (process.argv[2] === "--version") { console.log("codanna 0.16.0"); process.exit(0); }\n' +
      'if (process.argv.slice(2).join() !== "dump") process.exit(7);\n' +
      'console.log(JSON.stringify({type:"summary", data:{fixture:true}}));\n', { mode: 0o700 });
    const cjs = require(path.join(skills, 'x-ray/graph/dump.js'));
    const esm = await import(pathToFileURL(path.join(skills, 'graph/lib/dump.mjs')).href);
    for (const reader of [cjs.readDump, esm.readDump]) {
      assert.equal(reader({ binary, workingDir: temp }).summary.fixture, true);
      assert.equal(fs.existsSync(path.join(temp, 'marker')), false);
    }
  } finally { fs.rmSync(temp, { recursive: true, force: true }); }
});

test('disc CLI safely embeds hostile dump data in the final published HTML', () => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'codanna-disc-review-'));
  try {
    const dump = path.join(temp, 'fixture.jsonl');
    const output = path.join(temp, 'graph.html');
    fs.writeFileSync(dump, [
      { type: 'result', meta: { entity_type: 'symbol' }, data: { id: 1, name: attack, kind: 'Function', file_path: 'src/example.rs', module_path: attack, signature: attack, language_id: 'rust', range: { start_line: 1, end_line: 2 } } },
      { type: 'summary', data: { symbols: 1, relationships: 0 } },
    ].map(x => JSON.stringify(x)).join('\n'));
    execFileSync(process.execPath, [path.join(skills, 'graph/graph.mjs'), '--from', dump, '--dates', 'none', '--no-open', '--out', output, '--name', attack], {
      cwd: temp, env: { ...process.env, CLAUDE_PROJECT_DIR: temp }, stdio: 'pipe', timeout: 30000,
    });
    const html = fs.readFileSync(output, 'utf8');
    assert.ok(!html.includes(attack));
    assert.ok(!html.includes('<script>globalThis.INJECTED'));
    assert.match(html, /Content-Security-Policy/);
    const match = html.match(/window.VAULT_DATA=(.*);<\/script>/);
    assert.ok(match, 'published disc must retain data');
    assert.ok(JSON.stringify(JSON.parse(match[1])).includes('INJECTED'));
  } finally { fs.rmSync(temp, { recursive: true, force: true }); }
});


test('runtime prerequisites reject unsupported Node and accept tested minima', () => {
  const runtime = require(path.join(skills, 'shared/runtime.cjs'));
  for (const version of ['20.19.0', '22.15.9', '22.16.0-rc.1', 'invalid']) {
    assert.throws(() => runtime.assertNode(version), /Node >= 22\.16\.0/);
  }
  for (const version of ['22.16.0', '22.17.0', '24.0.0']) runtime.assertNode(version);
});

test('runtime binary preflight is bounded, non-installing and rejects incompatible output', { skip: process.platform === 'win32' }, () => {
  const runtime = require(path.join(skills, 'shared/runtime.cjs'));
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'codanna-runtime-'));
  try {
    const binary = path.join(temp, 'trusted binary ; literal');
    for (const [version, valid] of [['codanna 0.15.9', false], ['codanna 0.16.0-rc.1', false], ['codanna 0.16.0 (fixture)', true], ['unexpected output', false]]) {
      fs.writeFileSync(binary, '#!' + process.execPath + '\n' +
        'if (process.argv.slice(2).join() !== "--version") process.exit(7);\n' +
        'console.log(' + JSON.stringify(version) + ');\n', { mode: 0o700 });
      if (valid) assert.equal(runtime.assertBinary(binary, temp), '0.16.0');
      else assert.throws(() => runtime.assertBinary(binary, temp), /require a release Codanna/);
    }
    assert.throws(() => runtime.assertBinary(path.join(temp, 'missing'), temp), /Nothing was installed/);
  } finally { fs.rmSync(temp, { recursive: true, force: true }); }
});
