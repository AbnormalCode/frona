# Group M scoping — harness / sandbox / PKM extraction

Status: scoping spike, 2026-11-20. Read-only analysis of `fronalabs/frona`
`main` @ `05332fd0` against this fork's tree (no code ported). Same method as
[`GROUP_C_PROVIDER_SCOPING.md`](GROUP_C_PROVIDER_SCOPING.md). Companion to
[`UPSTREAM_SYNC_PLAN.md`](UPSTREAM_SYNC_PLAN.md), Groups M and N.

## What upstream did

Between 2026-11-05 and 2026-11-12 upstream pulled three subsystems out of
`frona-server` into publishable crates and rebuilt the server on top of them:

| Crate | Size | What moved |
|---|---|---|
| `frona-harness` | 70 files, ~21k lines | Model execution, provider adapters, history/compaction/budget, sessions, retry, tool dispatch, JSONL replay, structured refinement. Brings Rig 0.46, `rig-bedrock` and the AWS SDK. |
| `frona-pkm` | new | Portable PKM reasoning phases (server `memory/pkm/*` shrinks by ~36k lines). |
| `frona-sandbox` | new, 10 files | Portable sandbox execution with host hooks (`f7774c1e`, 60 files touched). |

The fork has none of the three. It does have `frona-text` and
`frona-model-catalog` (Groups L and C).

## Where the fork diverges (the collision surface)

Measured against upstream's `f6bb78f` baseline, the fork has its own changes in
263 server source files and upstream's range changes 299; **120 files are
touched on both sides**. By area, fork insertions vs baseline: `inference`
5.1k, `tool` 5.8k, `core` 1.8k, `agent` 1.5k, `chat` 1.1k, `memory/pkm` 0.9k.

Fork-only code with no upstream equivalent, and what happens to it:

| Fork code | Lines | Upstream counterpart | Risk |
|---|---|---|---|
| OpenRouter routing / prompt caching param handling (`inference/protocol/hooks.rs`) | 286 | Harness OpenRouter adapter is 82 lines and has none of it | **High.** Needs a host extension point in the harness, or the fork keeps its own request path. |
| OpenRouter cost accounting and `billing` (`inference/usage/*`, `cost_analysis` tool, `config`, cost route) | several files | Harness reports usage through an `InferenceObserver` trait; no billing | Medium. Probably fits the observer (`usage`/`attempt` callbacks), needs a check. |
| Vision (`inference/vision.rs`) and voice-note transcription (`transcription.rs`) | 439 + 458 | None | Medium. Sit beside inference; must be rehomed against the new `inference` module. |
| Extra providers: huggingface, hyperbolic, perplexity adapters; byteplus/zai/venice/minimax/llamafile recipes | ~400 + recipes | Harness adapters: azure, bedrock, cohere, copilot, gemini, openrouter, together (+ discovery) | Medium. `platform.rs` (1378 lines) moves into the harness. |
| `metadata/{catalog,loader}.rs` | — | Replaced upstream by `frona-model-catalog` | Low. Likely deletable. |
| Inbound voice, push, PWA, MCP SSE client, PDF render | — | Not in any extracted crate | **None** from this work, beyond the files they share (`tool/`, `chat/`). |

The Group G port (compaction/history/usage rework, 2026-11) already took the
upstream shape for compaction and replay, which *shrinks* the chat-side
conflict versus before. The harness's own `compaction.rs`, `history*.rs` and
`budget.rs` are the next iteration of that same code.

## What is separable (and cheap)

1. **Rig 0.42 → 0.46** (`2c56acbd`, 119 files upstream). Rig is used in 60
   fork files (inference, adapters, tests, chat, memory/basic). It does not
   require the harness; upstream bundled it with the migration. It can land
   first, alone, and any breakage is the same breakage the harness would
   cause later.
2. **Adding `frona-harness` as an unwired workspace member.** Build and test it
   standalone (its own ~21k lines of tests included) without touching the
   server. Zero runtime risk, and it gives the fork Bedrock support
   (currently a deliberate stub, `UPSTREAM_SYNC_PLAN.md` Group B caveat) once
   wired.

## What is not separable

Wiring the server onto the harness (`5ea2e93e`, `1e11256c`, `db675e10`,
`0f419c28`, then the Rig-execution unification `66793b75`, `a0ab8cf5`,
`58c6a9fe`) replaces the server's `inference/`, `chat/` and `tool/` execution
paths. This is where the 120 overlapping files and the fork-only inference code
above collide. It cannot be done incrementally without a bridge: each of these
commits assumes the previous one, and each assumes upstream's server shape.
`576be457` (bounded tool-batch concurrency) and the sandbox commits
(`f7774c1e`, `2fc32f41`) have the same dependency.

## Recommendation

Go for a staged port with a hard gate before the risky step:

- **M0 — Rig 0.46 on its own.** Do now. Verify with the full backend suite in
  CI (the sandbox here is too small for `cargo test --workspace`).
- **M1 — `frona-harness` unwired.** Do next. Review whether it builds with the
  fork's workspace pins (rustls, hyper, reqwest, AWS SDK) and passes its own
  tests; fork-pinned git deps (presage, curve25519, rusqlite) are not in its
  graph.
- **Gate — extension-point design.** Before M2, write a short design for how
  the fork's OpenRouter routing/caching, billing, vision and transcription
  attach to the harness (observer callbacks and request hooks exist; a
  request-param hook for provider-specific bodies may need an upstream-style
  addition). If that cannot be done without carrying a fork patch inside
  `frona-harness`, stop at M1 and freeze the server on the current shape.
- **M2 — wire the server** only if the gate passes, as a multi-PR effort in
  the order listed above, with `sdk_bridge`-style adapters where the fork's
  call sites differ.
- **Group N (PKM) and the sandbox crate** after M2, since both are written
  against the wired harness.

Fallback if M2 is declined: stay on M0/M1, keep taking server-side fixes by
content (as Groups F–P did), and record the divergence at the top of
`UPSTREAM_SYNC_PLAN.md`. Cost of waiting is real but bounded: each upstream
commit that lands inside the new crates becomes unportable by diff and has to be
read and re-applied by hand.

## Open questions

- Does the harness have (or can it take, upstream-style) a hook for
  provider-specific request bodies, so OpenRouter `provider`/caching fields do
  not need a fork patch?
- Is `rig-bedrock`/AWS SDK weight acceptable for the fork's image size, given
  Bedrock is not used today? (The harness pulls them unconditionally.)
- Does the fork want `frona-pkm`'s extraction at all, given fork PKM changes
  are small (0.9k lines) but the replaced tree is ~36k?

## M0 trial run — 2026-11-20 (correction to "cheap")

Bumped `rig-core` to 0.46 (`features = ["reqwest"]`, as upstream) on a scratch
tree and ran `cargo check -p frona-server` (needs Rust >= 1.97.1; the 1.97.0
sandbox toolchain was too old, 1.98.1 worked). Result: **95 compile errors in
the first pass, across 16 files**, concentrated in `inference/provider/*`
(adapter construction), `inference/error.rs`, `tool_loop.rs`,
`credential/managed/integration/*` and `chat/message/error.rs`. Rig 0.46
reorganised its module paths (`client::CompletionClient`, `ModelListingClient`,
`completion::CompletionModel`, per-provider `Client`/`CompletionsClient`/
auth types, `ToolCallId`), so the first-pass count understates the cascade.

Upstream's own migration (`2c56acbd`) touched about 50 non-harness server
files plus tests (`chat_compaction`, `tool_signature_replay`,
`usage_service_e2e`, ...) and added a 713-line `history_compat.rs`: **Rig 0.46
changed the serialised message shapes** (tool-call ids), so histories stored
by 0.42-era builds must be decoded through a legacy path. That is a data
compatibility requirement for this fork's existing deployments, not just a
compile fix; it needs tests against real stored histories.

Revised view: M0 is separable from the harness but is **not** a quick step. It
should be its own multi-PR effort (provider construction, then streaming/tool
loop, then history compat, then pricing/usage), validated by the full backend
suite in CI — the local sandbox cannot run it. No code from the trial was kept.
