# Fork customisation inventory and OpenRouter spike

Status: 2026-11-20. Input to the decision on rebuilding the fork from
`fronalabs/frona` `main` (with real upstream history) and re-applying this
fork's customisations, instead of continuing the content-port plan in
[`UPSTREAM_SYNC_PLAN.md`](UPSTREAM_SYNC_PLAN.md) /
[`GROUP_M_HARNESS_SCOPING.md`](GROUP_M_HARNESS_SCOPING.md).

Method: the README "Fork Enhancements" list (16 sections) as the feature set;
for each, file locations in this tree and a keyword search of
`upstream/main` @ `05332fd0` to see whether upstream now has an equivalent.
Keyword searches are a first pass, not proof; rows marked *verify* need a
code read before work starts. Nothing here was built or run.

Fork size for scale: 182 commits (116 authored by Claude, 54 by the fork
owner), about 280 paths in `crates/`, `web/`, `resources/`, `build/` that do
not exist upstream (many of those are upstream's own relocations — sandbox
drivers, PKM, `core/template.rs` — not fork features).

## Class A — additive (new files, light touch on shared code)

Re-apply by copying the module and wiring registration points.

| Feature | Where it lives | Notes |
|---|---|---|
| Inbound voice, ElevenLabs TTS, caller allowlist | `api/routes/voice/{inbound,allowlist}.rs`, `tool/voice.rs`, `web/.../voice-section.tsx` | Upstream: no inbound/ElevenLabs/allowlist matches. Streaming speech touches the harness loop (see class C). |
| Web Push (VAPID) + PWA | `notification/*` (5 files), `api/routes/push.rs`, `db/repo/push_subscriptions.rs`, `web/public/{sw.js,manifest.webmanifest,icons}`, `web/src/lib/{use-push-notifications,sw-register}.ts` | Upstream: none. Self-contained. |
| Agent / chat sharing | `agent/share/*`, `chat/share/*`, `db/repo/{agent,chat}_shares.rs`, web share modals | Upstream: none (hits are unrelated `is_shared` flags). Touches read paths in chat/agent services. |
| Password reset, self-service change, admin unlock, break-glass CLI | `auth/password_reset/*`, `db/repo/password_reset_tokens.rs`, `web/(auth)/{forgot,reset}-password`, `mail-section.tsx` | Upstream: none. Login-lockout core already exists upstream (ported in Group F). |
| Trigger tokens | `api/routes/triggers.rs`, `triggers-section.tsx` | Upstream: none. |
| Skills: manual add, `search_skills`/`add_skill` | `tool/skills.rs` | *verify* — one upstream file matches `add_skill`. |
| MCP SSE client + failure diagnosis | `tool/mcp/{sse_transport,diagnosis}.rs`, `mcp-log-viewer.tsx` | Upstream MCP moved to a direct backend (`888dd91`); *verify* overlap. |
| File preview, inline media, range requests | `api/routes/files/range.rs`, `web/.../preview/*`, `media-utils.ts` | Upstream: none found. |
| Browser live view | `api/routes/browser/live.rs`, `live-view.tsx` | Upstream added its own debugger/live routes (`c09cf49`, not ported). *verify*. |
| Image tool / analyze_image | `tool/image.rs`, `tool/files/analyze_image.rs` | Uses vision model (see class B). |
| Cost analyst agent, cost reports, admin usage APIs | `cost/*`, `api/routes/cost.rs`, `tool/cost_analysis.rs`, `cost-reports-section.tsx`, built-in agent `groups:`/`cron:` | Upstream: none. Reads the usage tables; depends on `billing` (class B). |
| Log stream, error boundary, toasts | `core/log_stream.rs`, `web/.../{error-boundary,toast}` | Small. |
| Build/release (GHCR, example tag bump, Makefile, CI) | `.github/workflows/*`, `build/*`, `Makefile` | Re-apply last; upstream has no `.github/`. |

## Class B — touches the inference layer (the risky set)

| Feature | Fork code | Upstream state | Re-apply approach |
|---|---|---|---|
| OpenRouter routing (`provider_routing`, `route`, `only`, `max_price`, `data_collection`, `zdr`) | `core/config/types.rs` (`OpenRouterParams`), `inference/protocol/hooks.rs` | Harness `ProviderModel::OpenRouter` carries only OpenAI-compat params, but request bodies accept `extra_params` | Host lowers fork fields into `extra_params` (see spike). No harness patch. |
| OpenRouter prompt caching | `platform.rs` `openrouter_prompt_caching` → Rig `with_prompt_caching()` | Rig 0.46 / harness builds OpenRouter on the OpenAI chat wire; auto-caching exists only for direct Anthropic (`cache_control`, `CacheRetention::Short`) | **Open.** See spike. |
| Cache-write pricing, aggregator id → catalog resolution | `inference/metadata/catalog.rs` | **Upstream has it**: `cache_write` pricing and `lookup_prefix` for vendor-namespaced ids in `frona-model-catalog` | Drop fork code. |
| BytePlus → volcengine catalog alias | `metadata/catalog.rs` | No upstream match | Small patch (host or catalog crate). |
| Provider billing model (`metered`/`subscription`/`self_hosted`) | `ModelProviderConfig.billing`, `inference/usage/*`, `cost/*`, `db/repo/inference_usage.rs` | None. Harness exposes `UsageObservation { model, usage, fallback_index, ... }` to an `InferenceObserver`; server's `UsageService` implements it | Host-side: billing keyed by provider handle at record time. Fits the observer. |
| Extra providers: generic, Z.ai, Venice, MiniMax, llamafile, BytePlus | `provider/platform.rs` recipes, `adapter/{huggingface,hyperbolic,perplexity}.rs` | Upstream has dynamic OpenAI-compatible path; none of the named five | One recipe entry each (as in Group C); hyperbolic/huggingface/perplexity exist upstream as brands. |
| Azure OpenAI | `adapter/azure.rs` | **Upstream has it** (17 files) | Drop fork code; check `api_version` and key-vs-Entra behaviour parity. |
| Vision | `inference/vision.rs` (439 lines; uses Rig `Message`/`UserContent`, `ModelGroup`, `UsageService`) | Not in harness | Rework onto the harness message/model types. Moderate. |
| Transcription | `inference/transcription.rs` (458 lines; plain HTTP, no Rig) | n/a | Copy as is; depends only on config, `Handle`, `Attachment`. |
| Request hooks (`openai` `max_completion_tokens`, `ollama`) | `protocol/hooks.rs` | Harness handles per-dialect request shaping natively | Likely drop. |

## Class C — edits to hot shared code

| Feature | Where | Notes |
|---|---|---|
| Fewer tool calls: batched `read`/memory tools, Tool Economy prompt | `tool/files/*`, `memory/*`, `resources/prompts/*` | Upstream now has bounded tool-batch concurrency (`576be457`); fork's concurrent read-only execution likely subsumed. Batched arguments and prompt text are fork-only. |
| Browser actions return the resulting page | `frona-browser`, tool views | Upstream matched `snapshot`-style diffs in 2 files; *verify*. |
| Private-memory agents | `memory/*`, agent model + backfill migration | Upstream moved PKM to `frona-pkm`; Basic memory still server-side. Re-apply against the new layout. Needs a DB migration for existing rows. |
| Shared-agent workspace-path resolution, Stop-actually-stops fixes, config/SMTP redaction, SSRF/IDOR fixes | scattered | Many already ported to or from upstream in Groups F/G; audit each against upstream before re-applying. |
| Streaming agent speech (voice) | agent loop callbacks | Depends on how the harness exposes streamed text. |

## Spike: can OpenRouter ride on the upstream harness without patching it?

Read-only, against `upstream/main`.

**Routing parameters — yes.** The harness merges a model's
`request_settings.extra_params` into the request body (`merge_json(body,
extra)`) and only rejects keys in a per-dialect *reserved* list. For the OpenAI
chat dialect that list is `model, messages, tools, functions, stream,
stream_options.include_usage`. `provider`, `route` and `cache_control` are not
reserved. So the server can keep the fork's typed config
(`provider_routing`, `route`, `only`, `max_price`, ...) and lower it to
`extra_params` (`provider_routing` → `provider`) when it builds the harness
`ModelConfig`. The fork's `hooks::openrouter` rename moves into that lowering.
The upstream OpenRouter adapter (82 lines) is discovery and key validation
only, so nothing there conflicts.

**Prompt caching — open.** The fork gets caching from Rig's OpenRouter provider
(`with_prompt_caching()`, a `cache_control` breakpoint on the system prompt).
Rig 0.46 no longer exposes that provider type, and the harness builds
OpenRouter on the OpenAI chat wire. The harness's automatic caching is only for
direct Anthropic (top-level `cache_control`). Two options: (a) send a top-level
`cache_control` via `extra_params` for OpenRouter — **unverified** whether
OpenRouter applies a top-level marker for non-Anthropic backends; (b) add a
typed system-prompt breakpoint in the harness (a patch the fork would carry or
propose upstream). This needs a live test with an OpenRouter key and a request
through both Anthropic- and Gemini-backed models; it cannot be settled by
reading code.

**Cost accounting — mostly absorbed upstream.** `frona-model-catalog` now
prices cache writes and resolves vendor-prefixed aggregator ids, which were two
of the fork's OpenRouter fixes. What remains is host-side: the `billing`
classification and the instance-wide aggregations, which hang off the observer
the harness already provides.

**Verdict.** The OpenRouter feature set does not require patching the harness
except possibly the caching breakpoint. That is the single item that could force
a carried patch; everything else is host code.

## Suggested order if the rebuild goes ahead

1. New branch from `upstream/main` (keeps upstream history, so future syncs are
   merges). Build it in CI first; the sandbox here is too small.
2. Class A additive features, one commit group each with its tests, starting
   with build/CI, then push/PWA, auth recovery, sharing, voice, triggers.
3. Class B on top of the harness: transcription, billing/cost, providers,
   OpenRouter lowering, vision. Resolve the caching question with a live test.
4. Class C last, auditing each fix against upstream first.
5. DB/config compatibility run against a copy of a real deployment's data
   (fork tables: shares, push subscriptions, password-reset tokens, usage
   billing columns, `private_memory`) before switching `main`.

## Decision log

- 2026-11-20: owner judged the OpenRouter prompt-caching risk small. Proceed on
  that basis without a prior live test; if caching misbehaves after the
  rebuild, fall back to option (b) (typed breakpoint in the harness, carried
  or proposed upstream). Caching correctness should be checked once against a
  real OpenRouter key after the class B work lands, not before.
