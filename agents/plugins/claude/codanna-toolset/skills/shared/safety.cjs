// Context-specific encoding for repository-derived data. No shell is involved.
'use strict';
const { randomBytes } = require('node:crypto');
const { execFileSync } = require('node:child_process');
const { pathToFileURL } = require('node:url');

function jsonForScript(value) {
  return JSON.stringify(value).replace(/[<>&\u2028\u2029]/g, c =>
    '\\u' + c.charCodeAt(0).toString(16).padStart(4, '0'));
}

function escapeHtml(value) {
  return String(value ?? '').replace(/[&<>"']/g, c => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
  }[c]));
}

// Apply only after all trusted vendor scripts are inlined. Script-aware matches
// avoid touching '<script' strings inside library code. This is defense in depth;
// repository data must already be encoded for its HTML or JavaScript context.
function secureHtml(html) {
  const nonce = randomBytes(18).toString('base64');
  const policy = "default-src 'none'; script-src 'self' blob: 'nonce-" + nonce +
    "'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; " +
    "connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
  return html.replace(/<script\b([^>]*)>([\s\S]*?)<\/script\s*>/gi,
    (_, attributes, body) => `<script nonce="${nonce}"${attributes}>${body}</script>`)
    .replace(/<head(?:\s[^>]*)?>/i, head => head +
      `<meta http-equiv="Content-Security-Policy" content="${escapeHtml(policy)}">`);
}

function openBrowser(target) {
  // On Windows 'start' is a shell built-in. Do not invoke cmd.exe with a path.
  // Leave a printable URL for manual opening rather than risk shell expansion.
  const url = /^(https?|file):/i.test(target) ? target : pathToFileURL(target).href;
  if (process.platform === 'win32') { console.log(`Open in your browser: ${url}`); return; }
  try {
    execFileSync(process.platform === 'darwin' ? 'open' : 'xdg-open', [url], { stdio: 'ignore' });
  } catch { /* Headless environments still retain the generated artifact. */ }
}

module.exports = { jsonForScript, escapeHtml, secureHtml, openBrowser };
