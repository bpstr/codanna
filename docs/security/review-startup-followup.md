# PR #23 — final CI startup follow-up

This supplements `review-20260912.md` and preserves its 38-finding disposition.
Source fix: `e187a3ab4901e59710946e515cd380c7b0863360`.

## Failure and fix

Normal PR run `34718357006` passed default-feature tests but failed the all-feature
CLI suite at `serve_stdio_legacy_lane_sends_no_custom_notifications`. It observed
no standard resource update after the first file edit. The earlier successful
workbench run therefore was not sufficient to call the normal PR gates green.

Source inspection identified a real admission race: the unified watcher was
spawned, then asynchronously loaded handler paths and installed native watches,
while the MCP server could already answer initialization. A client's immediate
edit could occur before native registration and leave no event to process.

`UnifiedWatcher::prepare` now completes handler initialization and native
registration before stdio, HTTP, or HTTPS admits clients. Successful preparation
is idempotent; ordinary `watch()` callers still prepare automatically. Startup
handler/native-watch failures return errors instead of advertising readiness or
quietly continuing without requested watching. **Fix invalid/missing watch roots
before starting a watching server; failed preparation is not a successful degraded
watch mode.** Callers must discard a watcher instance whose preparation failed.

On macOS, a logical registry entry for an indexed file's parent no longer prevents
installing that handler's recursive native root watch. The old CLI advice to
unlink an active serve-lock file was also removed.

The legacy wire fixture now completes initialization in protocol order and waits
for a successful ping round trip before the immediate edit. Its positive resource
notification assertion and six-second observation window remain unchanged. No
sleep, repeated file rewrite, ignored test, or relaxed assertion hides the race.

Three new production-preparation regressions cover a deliberately delayed handler,
idempotence, handler failure, and native registration failure. The delayed-handler
fixture verifies readiness cannot complete before the handler is released.
The permanent review regression script now includes the actual legacy wire test,
not only handler unit tests. Full CI still executes every default/all-feature target.

## Executed validation

Successful Actions run `34719092187` validated the exact source patch before it
was appended to the existing review branch without force-pushing:

- Default features: **1,820 Rust tests passed**, zero failed, 59 ignored.
- All features: **1,822 Rust tests passed**, zero failed, 59 ignored.
- The legacy immediate-edit notification test additionally passed **five out of
  five** separate invocations. Each filter was checked to execute exactly one test.
- Strict all-target/all-feature Clippy, the no-default-features check, and public
  documentation with warnings denied passed.

Patch SHA-256: `ef9531c7af0102818b7ca9be3fc53f75e21d4d85cae318496dc10f2fe723bc04`.
Feature-suite totals overlap and must not be added as distinct tests. The five
extra invocations are repetitions of one existing test, not five new tests.
The three added library regressions explain the increase from 1,819/1,817 in the
previous implementation run. Ignored tests remain explicitly unexecuted.

The source-patch run is evidence for this fix, not a substitute for the final PR
checks. Consult the current PR head for Full Test Suite, Quick Check, Hardening,
and the permanent security/browser workflow outcomes. No paid inference was used;
source fixtures disable semantic indexing and the tests use local transports.

## Server-observed SSE reconnect regression

After the startup correction, normal runs `34719527440` (Hardening) and
`34719527436` (Full Test Suite) exposed an intermittent HTTP reconnect-fixture
failure. The default suite and macOS/Linux watcher witnesses passed. The failed
assertion was the first session's notification after dropping and recreating its
SSE response, not a regression in watcher preparation.

The old fixture treated dropping a client `Response` as proof that Hyper had
already dropped the server's stream. Those are different lifecycle events. The
locked rmcp 3.1.4 transport intentionally creates an idle shadow GET while an
older common stream is active, so racing a new GET against unobserved server
closure did not reliably test reconnection. Its substring-only notification
check could also mistake an inclusively replayed old event for fresh delivery.

Commit `193f379029cd38f6768efb253d0447752ed2d3c0` changes only the test module and
adds the already-locked `http-body` package as a direct development dependency:

- A test-only body observer forwards every production body frame unchanged and
  acknowledges closure only after dropping the inner server response body.
- The fixture awaits that actual closure before reconnecting, while asserting
  both Codanna session listeners remain alive for reconnection.
- The reconnect supplies `Last-Event-ID`. A buffered SSE parser checks complete
  JSON-RPC events and strictly increasing event identities. A replayed previous
  notification cannot satisfy the next fresh-delivery assertion.
- Both clients still must receive each new event; deletion and injected-clock
  expiry must still remove the correct sessions and enforce ownership.

The five-second bounds remain unchanged. The test does not force the production
transport closed, sleep before reconnection, resend an event until one arrives,
or relax/ignore the positive delivery assertions. Production router and session
implementation bytes are unchanged by this fixture correction.

Successful source run `34720293333` validated the corrected fixture before it was
appended to the same review branch:

- **Ten out of ten parallel HTTP/TLS pairs passed** (20 executions of two existing
  tests, not 20 distinct new tests).
- The normal-parallelism hardening command passed **102 tests**, zero failures.
- Full default suite: **1,820 passed; zero failed; 59 ignored**.
- Full all-feature suite: **1,822 passed; zero failed; 59 ignored**.
- Strict Clippy, no-default-features checking and warning-free public docs passed.

Exact tested fixture patch SHA-256:
`9776583cf21dab5120b72449afd803d974bb60337420a577560dfe7ac6d3b849`.
These counts overlap with each other and earlier validation. A first candidate
correctly failed strict Clippy on an unnecessary trait import; the import was
removed, the lint stayed enabled, and the entire validation above was rerun.

The permanent PR workflows continue to execute both full feature suites, the
hardening tests, the review Rust/Node gate and the real Chromium example checks.
Check the PR's current-head results for final gate status; historical source-run
success alone is not a substitute. Nothing was merged or force-pushed.
