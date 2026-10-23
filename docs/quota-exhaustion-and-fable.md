# Exhaustion alerts and optional Claude Fable quota

## Exhaustion alerts

Automatic ntfy alerts use the provider's raw consumption, never a rounded display
percentage. A counter first observed below 100% arms its next exhaustion alert.
A counter first observed at 100% is recorded silently. Polling, indicator selection,
missing/invalid/stale data, provider errors and restarting the application do not
rearm an exhausted counter. A real return below 100%, a changed known reset window,
or an elapsed known reset followed by fresh data establishes a new cycle.

The local `ntfy-attempts.json` ledger now records both the reset and exhausted state.
Existing number/null entries remain readable as already exhausted counters. State
is saved before attempting delivery; a network failure is not retried until rearmed.
All new exhaustions collected in one worker refresh are combined into one message,
with independent state per provider/counter (including Fable).

## Fable field provenance

The installed official Claude Code executable, version **2.1.299**, includes the
`get_usage` response schema and its projection implementation. Read-only inspection
of its distributed program code (not account configuration, caches or credentials)
confirms the optional field:

    rate_limits.model_scoped[]
        display_name: string (schema example: "Fable")
        utilization: number | null
        resets_at: string | null (ISO 8601)

The schema describes these as per-model weekly windows projected from the server's
`limits[]` and filtered by Claude Code's overage-included-models allowlist. An absent
field may mean cached data or a hidden bucket; an empty array means the endpoint
reported no scoped window. The production transport already requests `get_usage`;
no new endpoint, credential access or inference request is introduced.

Only a bucket explicitly labelled `Fable` (case insensitive) with valid numeric
utilization is added as `claude:weekly_fable`, labelled "Limite hebdomadaire Fable".
It uses the normal seven-day metric/reset rendering and exhaustion notification
path. Missing/null/invalid buckets produce no Fable row, not an invented 0%.
Old responses without `model_scoped` keep the existing five-hour and weekly metrics.
Expired windows use the existing unavailable state; a reported percentage without
a reset remains available with reset "non communiquée".

Do not infer a field named `seven_day_fable`: it is not in the inspected control
schema. The public status-line feature request is separate from this transport:
https://github.com/anthropics/claude-code/issues/88137
General official CLI reference: https://code.claude.com/docs/en/cli-reference

## Verification and limitations

### Fable display preference

Settings → Fournisseurs → Claude now includes an explicit Fable quota switch.
`fableEnabled` is persisted with the existing atomic `prefs.json` writer and defaults
to true for existing installations. It controls Quota Monitor only, not Anthropic
model access or a subscription. Turning Claude off preserves the Fable preference.
Disabled Fable is removed from frontend snapshots, the native macOS panel and the
fallback menu; other quotas remain unchanged. No additional provider call is made.

The notification ledger persists a muted baseline, including when delivery is
globally disabled. Hidden samples and missing/invalid data cannot replay a hidden
crossing on reactivation. The first valid enabled sample establishes a silent
baseline; subsequent real below-100 → 100 transitions notify normally. Existing
ledger entries without the new muted flag remain readable.

The Fable row is fully opaque (no dimming); it is set apart in React and in the
native macOS panel by a small teal dot before its label and a teal bar fill while
healthy (system teal, light/dark variants) — warning and low states keep the
regular orange/red so alert colours are never altered. The Settings switch keeps
its accessible label without a help sentence. The native countdown label now
retains that word instead of dropping it
when a reset date is available. Its countdown (potentially 5h or 30h remaining)
is distinct from the bucket's weekly duration. The distributed schema documented
above and the fixtures supply no separate 5h/30h Fable buckets: none are invented,
and existing general Claude 5h/weekly quotas are not relabelled as Fable.

Clarification before any further relabelling: **30h = durée souhaitée ou autre
libellé ?** The request's token `30h` is intentionally not replaced with `7j`.
No live account response, running-app restart, installation, release or real
notification was used for this follow-up. React markup, native panel labels,
snapshot filtering, preference migration and muted-ledger transitions are tested
offline; interactive installed-app UI remains a manual review check.

Unit fixtures are synthetic test values following the inspected official schema,
not claimed captured account responses. Tests cover present/absent/null/invalid
Fable, real zero, exact exhaustion, reset, unchanged old quotas, grouped alerts,
first observation at 100%, restart, legacy ledger migration and rearming rules.
Existing transport tests use a fake local CLI; HTTP delivery tests use loopback
servers only. No real ntfy notification or model-generation request is needed.

Local checks (Node 24 for Vite 8 and the TypeScript-importing Node tests):

    cargo test --manifest-path src-tauri/Cargo.toml
    cargo build --manifest-path src-tauri/Cargo.toml
    node --test scripts/quota-providers.test.mjs
    npm run typecheck
    npm run build
    rustfmt --edition 2021 --check src-tauri/src/{ntfy,claude,claude_cli}.rs
    git diff --check

The control response is explicitly experimental in Claude Code's schema; future
versions can change it. No authenticated live account response or installed-app UI
was exercised for this change. Fable is shown only when the existing transport
actually supplies the supported bucket; no fallback to credentials or raw OAuth
endpoints is attempted. No release, installation or running-app restart is part
of this change.
