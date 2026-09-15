# Workspace root changes must precede request dispatch

The local `codanna workspace serve` entry resolves an implicit request from one
connection's client roots, or from its launch directory when roots are not
supported. It must never reuse a cached previous workspace for a request received
after `notifications/roots/list_changed` on that connection.

## Why an asynchronous callback is insufficient

The pinned RMCP 3.1.4 service loop dispatches requests and notifications in
independently scheduled tasks. Receiving a roots-change notification first does
not establish that `on_roots_list_changed` finishes before the following
`call_tool` starts. The old implementation could therefore return the former
workspace's valid result, not merely an indexing delay. Retrying until the desired
result appears, increasing a timeout, or sleeping after a notification would hide
the isolation failure rather than fix it.

## Receive-order invalidation

The workspace entry wraps the SDK's decoded stdio transport in `ScopeTransport`.
On receiving a roots-change notification, it advances the connection-local root
generation synchronously, before returning the message to the SDK. It introduces
no await between message consumption and invalidation. A later message cannot be
dispatched before this invalidation, and cancellation cannot discard an already
consumed message at that boundary.

The existing generation checks reject cached roots and continuations from older
revisions. Cache entries remain bounded and may be reused until a real root
change; no repository walk, unconditional roots round trip, or additional global
lock is added to a warm request. The asynchronous roots callback is removed so a
delayed duplicate invalidation cannot invalidate freshly selected roots.

Framing, protocol negotiation, outgoing messages, cancellation notifications, and
transport closure remain delegated to RMCP. The bare workspace handler is private;
the public CLI entry assembles the handler and transport together. Each stdio
connection keeps its own resolver and worker pool. This is not a machine-wide
shared HTTP service.

This rule applies to messages received after the notification. Already executing
requests retain their independently selected workspace; it does not introduce
cross-session cancellation or a global active workspace. Explicit selectors keep
their existing per-request semantics and cannot authorize arbitrary new indexing.

## Regression coverage

`hardening_workspace_transport_invalidates_roots_before_dispatch` writes the
notification and following tool request together to a duplex transport. It runs
no notification handler and asserts that the generation has already advanced
before the request can be delivered. Unrelated cancellation notifications must
still be passed through without changing the generation.

`hardening_workspace_mcp_queued_root_changes_preserve_session_isolation` runs two
real subprocess clients against synthetic projects, tests both supported roots
protocol paths, immediately switches one client's roots repeatedly, checks both
clients' results, preserves warm-cache reuse, and checks the surviving client
after the other disconnects. No project paths appear in MCP configuration and no
explicit workspace selector is sent.

`hardening_workspace_mcp_root_change_rejects_queued_old_continuation` checks that
an old multi-round-trip roots response cannot bootstrap a previous root after a
notification. The existing immediate-switch regression remains unchanged.

Run the transport unit test and complete workspace MCP suite, followed by the
repository's quick/full checks. Added test source is not a claim of a passing run;
record validation against the actual final PR commit.
