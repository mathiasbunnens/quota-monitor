import assert from "node:assert/strict";
import test from "node:test";
import { claudeRows, formatResetDate } from "../src/features/quota/providers.ts";

// Synthetic snapshot fixtures matching the backend's normal metric contract.
const metric = (id, usedPercent, resetAt = null) => ({
  id, label: id === "claude:weekly_fable" ? "Limite hebdomadaire Fable" : "Limite 5 heures",
  usedPercent, resetAt, availability: "available",
  periodDurationSeconds: id === "claude:weekly_fable" ? 604800 : 18000,
  exhausted: usedPercent >= 100,
});
const snapshot = (metrics) => ({ metrics });

test("optional Fable uses normal remaining-percentage and reset rendering", () => {
  const reset = "2026-10-30T14:00:00Z";
  const rows = claudeRows(snapshot([
    metric("claude:five_hour", 23.5), metric("claude:weekly_fable", 37.5, reset),
  ]));
  assert.equal(rows.length, 2);
  assert.equal(rows[0].remaining, 77);
  assert.deepEqual(rows[1], {
    id: "claude:weekly_fable", label: "Limite hebdomadaire Fable",
    remaining: 63, exhausted: false, missing: "Non communiquée",
    resetText: formatResetDate(reset),
  });
});

test("old snapshots do not fabricate Fable; expired windows remain unavailable", () => {
  assert.equal(claudeRows(snapshot([metric("claude:five_hour", 0)])).length, 1);
  const expired = { ...metric("claude:weekly_fable", null), availability: "unavailable" };
  const [row] = claudeRows(snapshot([expired]));
  assert.equal(row.remaining, null);
  assert.equal(row.exhausted, false);
  assert.equal(row.missing, "Réinitialisée · en attente");
  assert.equal(row.resetText, "non communiquée");
});

test("rounded zero remaining is not actual Fable exhaustion", () => {
  const [rounded, exhausted] = claudeRows(snapshot([
    metric("claude:weekly_fable", 99.6), metric("claude:weekly_fable", 100),
  ]));
  assert.equal(rounded.remaining, 0);
  assert.equal(rounded.exhausted, false);
  assert.equal(exhausted.remaining, 0);
  assert.equal(exhausted.exhausted, true);
});
