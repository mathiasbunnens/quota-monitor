# Quota refill alerts and optional Claude Fable quota

## Refill alerts (replaces exhaustion alerts)

Automatic ntfy alerts now fire ONLY on an observed nonfull → full transition:
raw consumption greater than zero → exactly zero (100% remaining). There is no
80/90% consumption warning and no exhaustion alert. Never use a rounded display:
0.4% consumed is 99.6% remaining, not a refill. Missing/invalid/stale data and
provider errors are not zero. Codex, Claude and optional Fable use the same rule.

First discovery at full is silent, as are repeated full polls and restarts. The
local `ntfy-attempts.json` ledger records full/nonfull, last observation time, reset,
attempt and mute state per counter. Old exhaustion objects and number/null entries
migrate on their first valid sample to a silent refill baseline; no preferences,
ntfy server/topic, credentials or provider data are changed. State is saved before
delivery, so failures/interruption cannot repeatedly send the same refill.

For known reset windows, at most one attempt is allowed until the prior reset has
elapsed; a changed future reset date or small within-cycle fluctuation cannot
rearm it. An elapsed timestamp alone never sends: a nonfull → full edge is still
required. Temporarily missing metadata preserves a known future reset. Without a
reset timestamp, repeated full polls remain silent and a new exact nonfull → full
edge is the only cycle evidence available; the app cannot independently prove an
upstream refill versus provider jitter in that case. Older observations are ignored.
All new refills in one refresh are grouped, independently deduplicated per counter.

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
It uses the normal seven-day metric/reset rendering and refill notification
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
baseline; subsequent nonfull → full transitions notify normally. Existing
ledger entries without the new muted flag remain readable.

The Fable row is fully opaque (no dimming or dot); the word Fable keeps the normal
quota label typography (React 13px, native 12pt) and uses the exact same dynamic colour as its bar,
including warning/low states and light/dark variants. The healthy fill remains
teal; other quotas and alert colours are unchanged. The Settings switch keeps
its accessible label without a help sentence.
The display name is exactly **Limite Fable**, even when a reset is known; the real
reset remains on the reset line. A matching general weekly row (`claude:seven_day`)
is replaced in place by the explicit model-scoped Fable row, removing the extra
Fable display. Matching requires available, equal raw utilization, equal duration
and the same known reset; rounding is never used. This is a presentation-only
coalescing rule for the duplicate reported by the user, not a claim that all weekly
buckets are Fable. Distinct values/resets, unknown resets and unavailable data keep
the general row separately. Missing Fable never relabels a general quota.

The raw provider snapshot and notification keys/labels are unchanged, so renaming
or coalescing cannot rearm or replay notifications. Disabling Fable hides only its
row and restores the general weekly row; the persisted mute behavior is unchanged.
No separate 5h/30h Fable buckets or reset durations are invented.

Clarification before any further relabelling: **30h = durée souhaitée ou autre
libellé ?** The request's token `30h` is intentionally not replaced with `7j`.
No live account response, running-app restart, installation, release or real
notification was used for this follow-up. React markup, native panel labels,
snapshot filtering, preference migration and muted-ledger transitions are tested
offline; interactive installed-app UI remains a manual review check.

Unit fixtures are synthetic test values following the inspected official schema,
not claimed captured account responses. Tests cover present/absent/null/invalid
Fable, real zero consumption, exact refill, reset, unchanged old quotas, grouped alerts,
first observation at full, restart, legacy ledger migration and rearming rules.
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
