#!/usr/bin/env node
// Real Chromium/React regression checks, without downloading a test framework.
// Run against the locally started example app; only loopback targets are allowed.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, mkdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const origin = new URL(process.env.REVIEW_BROWSER_URL || 'http://127.0.0.1:3040');
assert.equal(origin.protocol, 'http:');
assert.ok(['127.0.0.1', 'localhost', '[::1]'].includes(origin.hostname));
const output = process.env.REVIEW_BROWSER_OUTPUT || join(tmpdir(), 'codanna-browser');
await mkdir(output, { recursive: true });
const profile = await mkdtemp(join(tmpdir(), 'codanna-chrome-'));
const chrome = spawn(process.env.CHROME_BIN || '/usr/bin/google-chrome', [
  '--headless=new', '--no-sandbox', '--disable-dev-shm-usage',
  '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--remote-debugging-port=0',
  `--user-data-dir=${profile}`, 'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'] });
let chromeError = null;
chrome.on('error', error => { chromeError = error; });
let diagnostics = '';
chrome.stderr.on('data', chunk => { diagnostics = (diagnostics + chunk).slice(-32768); });
let ws;
let sequence = 0;
const pending = new Map();
const exceptions = [];
function call(method, params = {}) {
  return new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 15000);
    pending.set(id, { resolve, reject, timer });
    ws.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(expression) {
  const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  assert.ok(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
  return result.result.value;
}
async function until(fn, description, ms = 30000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (chromeError) throw chromeError;
    if (chrome.exitCode !== null) throw new Error(`Chrome exited: ${diagnostics}`);
    const value = await fn();
    if (value) return value;
    await sleep(100);
  }
  throw new Error(`Timed out: ${description}`);
}
async function key(key, code, virtualKey) {
  const text = key === 'Enter' ? '\r' : key.length === 1 ? key : '';
  await call('Input.dispatchKeyEvent', { type: text ? 'keyDown' : 'rawKeyDown', key, code, windowsVirtualKeyCode: virtualKey, text, unmodifiedText: text });
  await call('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: virtualKey });
}
const visible = `el => !!el && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== 'hidden'`;
async function focus(selector) {
  const expression = `(() => { const e = [...document.querySelectorAll(${JSON.stringify(selector)})].find(${visible}); if (!e) return false; e.scrollIntoView({block:'center'}); e.focus(); return document.activeElement === e; })()`;
  await until(() => evaluate(expression), `focus ${selector}`);
}
async function screenshot(name) {
  const data = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
  await writeFile(join(output, `${name}.png`), Buffer.from(data.data, 'base64'));
}
try {
  const port = await until(async () => {
    try { return Number((await readFile(join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]); }
    catch (error) { if (error.code === 'ENOENT') return null; throw error; }
  }, 'Chrome debugging endpoint');
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const page = targets.find(target => target.type === 'page');
  assert.ok(page, 'Chrome must expose a page target');
  ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.addEventListener('open', resolve, { once: true });
    ws.addEventListener('error', reject, { once: true });
  });
  ws.addEventListener('message', event => {
    const message = JSON.parse(event.data);
    if (message.id) {
      const promise = pending.get(message.id);
      if (!promise) return;
      clearTimeout(promise.timer); pending.delete(message.id);
      if (message.error) promise.reject(new Error(JSON.stringify(message.error)));
      else promise.resolve(message.result);
    } else if (message.method === 'Runtime.exceptionThrown') {
      exceptions.push(message.params.exceptionDetails);
    }
  });
  await call('Page.enable'); await call('Runtime.enable'); await call('Accessibility.enable');
  // 320 CSS pixels also witnesses the reflow width of a 1280px viewport at 400% zoom.
  await call('Emulation.setDeviceMetricsOverride', { width: 320, height: 800, deviceScaleFactor: 1, mobile: false });
  for (const route of ['/examples/mail', '/demo']) {
    const result = await call('Page.navigate', { url: new URL(route, origin).href });
    assert.ok(!result.errorText, result.errorText);
    const inbox = 'section[aria-label="Inbox"]';
    const buttons = `${inbox} button[aria-pressed]`;
    await until(() => evaluate(`document.querySelectorAll(${JSON.stringify(buttons)}).length >= 2`), `${route} interactive mail`, 90000);
    await until(() => evaluate("document.readyState === 'complete'"), 'page resources loaded');
    await evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))');
    // Assert observable visibility before sending actual keyboard events.
    await until(() => evaluate(`!!document.querySelector(${JSON.stringify(inbox)}) && [...document.querySelectorAll(${JSON.stringify(buttons)})].some(${visible})`), 'visible mobile inbox');
    const count = await evaluate(`document.querySelectorAll(${JSON.stringify(buttons)}).length`);
    const sender = await evaluate(`(() => { const e = document.querySelectorAll(${JSON.stringify(buttons)})[1]; e.scrollIntoView({block:'center'}); e.focus(); return e.querySelector('.font-semibold').textContent; })()`);
    await key('Enter', 'Enter', 13);
    await until(() => evaluate(`document.querySelectorAll(${JSON.stringify(buttons)})[1].getAttribute('aria-pressed') === 'true'`), 'keyboard selects message');
    await until(() => evaluate(`document.querySelector('section[aria-label="Selected message"]').textContent.includes(${JSON.stringify(sender)})`), 'selected message renders');
    const reply = `section[aria-label="Selected message"] textarea`;
    await focus(reply);
    assert.equal(await evaluate(`document.activeElement.getAttribute('aria-label')`), `Reply to ${sender}`);
    await call('Input.insertText', { text: 'Keyboard reply fixture' });
    assert.equal(await evaluate('document.activeElement.value'), 'Keyboard reply fixture');
    await focus(`${inbox} input[aria-label="Search mail"]`);
    await call('Input.insertText', { text: sender });
    await until(() => evaluate(`document.querySelectorAll(${JSON.stringify(buttons)}).length > 0 && document.querySelectorAll(${JSON.stringify(buttons)}).length < ${count}`), 'controlled search filters mail');
    await screenshot(route === '/demo' ? 'demo-320' : 'mail-320');
    const sizes = await evaluate(`(() => { const e = document.querySelector(${JSON.stringify(inbox)}); return {width:e.getBoundingClientRect().width, viewport:innerWidth}; })()`);
    assert.ok(sizes.width > 0 && sizes.width <= sizes.viewport + 1, `inbox must reflow: ${JSON.stringify(sizes)}`);
    // Clearing by keyboard exercises the real controlled input handler.
    await call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'a', code: 'KeyA', windowsVirtualKeyCode: 65, modifiers: 2 });
    await call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'a', code: 'KeyA', windowsVirtualKeyCode: 65, modifiers: 2 });
    await key('Backspace', 'Backspace', 8);
    await until(() => evaluate(`document.querySelectorAll(${JSON.stringify(buttons)}).length === ${count}`), 'clearing search restores messages');
    // Radix tabs support keyboard activation; verify Unread is not a dead control.
    await evaluate(`(() => { const e = [...document.querySelectorAll('${inbox} [role="tab"]')].find(e => e.textContent === 'Unread'); e.focus(); })()`);
    await key('Enter', 'Enter', 13);
    await until(() => evaluate(`document.querySelectorAll(${JSON.stringify(buttons)}).length > 0 && document.querySelectorAll(${JSON.stringify(buttons)}).length < ${count}`), 'Unread filters messages');
    console.log(`PASS ${route}: 320px interactive list, keyboard selection, reply, search and unread filter`);
  }
  // The More trigger opens actual theme controls; do not emulate a component.
  await evaluate(`(() => { const e = [...document.querySelectorAll('button')].find(e => e.textContent.trim() === 'More' && (${visible})(e)); if (!e) throw Error('Missing theme trigger'); e.scrollIntoView({block:'center'}); e.focus(); })()`);
  await key('Enter', 'Enter', 13);
  const swatches = 'button[aria-label$="duo-tone preset"]';
  await until(() => evaluate(`document.querySelectorAll(${JSON.stringify(swatches)}).length === 8`), 'all labeled swatches');
  const names = await evaluate(`[...document.querySelectorAll(${JSON.stringify(swatches)})].map(e => e.getAttribute('aria-label'))`);
  assert.equal(new Set(names).size, 8);
  await focus('button[aria-label="Green duo-tone preset"]'); await key('Enter', 'Enter', 13);
  await until(() => evaluate(`(() => { const e=document.querySelector('button[aria-label="Green duo-tone preset"]'); return e.getAttribute('aria-pressed')==='true' && e.textContent.includes('✓'); })()`), 'selected preset exposes state and non-color indicator');
  const ax = await call('Accessibility.getFullAXTree');
  assert.ok(ax.nodes.some(n => !n.ignored && n.name?.value === 'Green duo-tone preset' && n.role?.value === 'button' && n.properties?.some(p => p.name === 'pressed' && String(p.value.value) === 'true')), 'real accessibility tree must expose named pressed button');
  await screenshot('theme-320');
  assert.deepEqual(exceptions, [], 'browser must not report uncaught runtime exceptions');
  console.log('PASS theme: unique accessible names, keyboard selection, pressed state and non-color indicator');
} catch (error) {
  if (ws?.readyState === WebSocket.OPEN) {
    await screenshot('failure').catch(() => {});
    await writeFile(join(output, 'dom.txt'), String(await evaluate('document.documentElement.outerHTML').catch(() => 'unavailable')));
  }
  console.error(error); process.exitCode = 1;
} finally {
  ws?.close();
  for (const request of pending.values()) clearTimeout(request.timer);
  chrome.kill('SIGTERM');
  await writeFile(join(output, 'chrome.log'), diagnostics);
  await rm(profile, { recursive: true, force: true, maxRetries: 5 }).catch(() => {});
}
