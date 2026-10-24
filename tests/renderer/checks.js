// Run in the offline fixture page console: await (await import('./checks.js')).checkUpdater()
export async function checkUpdater() {
  const fixture = window.updateFixture;
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const cases = [
    ["checking", null, "Recherche d’une nouvelle version…"],
    [
      "available",
      null,
      "La version test-current est installée. Quota Monitor vtest-next est disponible.",
    ],
    ["downloading", 0, "Téléchargement de Quota Monitor vtest-next…"],
    ["downloading", 42, "Téléchargement de Quota Monitor vtest-next…"],
    ["downloading", null, "Téléchargement de Quota Monitor vtest-next…"],
    ["installing", null, "Téléchargement reçu. Vérification et installation…"],
    [
      "ready",
      100,
      "Quota Monitor vtest-next est installé et prêt à être relancé.",
    ],
    [
      "error",
      null,
      "Vérifie ta connexion Internet puis réessaie.\n\nDétail : " +
        "Erreur synthétique /offline-fixture/ ".repeat(80),
    ],
    [
      "up-to-date",
      null,
      "La version test-current est la dernière version disponible.",
    ],
  ];
  const results = [];
  for (const [phase, progress, message] of cases) {
    await fixture.set({ phase, progress, message });
    const root = document.querySelector("main").getBoundingClientRect();
    const bar = document.querySelector("progress");
    const heading = document
      .querySelector(".update-popup__progress-heading")
      ?.getBoundingClientRect();
    const percent = document.querySelector(".update-popup__percent");
    const br = bar?.getBoundingClientRect();
    assert(
      document.documentElement.scrollWidth <= innerWidth,
      `Horizontal overflow: ${phase}`,
    );
    if (br) {
      assert(br.top >= heading.bottom, `Progress overlaps heading: ${phase}`);
      assert(
        Math.abs(br.right - percent.getBoundingClientRect().right) < 1,
        `Percent alignment: ${phase}`,
      );
      assert(br.width > innerWidth - 90, `Progress too narrow: ${phase}`);
      assert(
        bar.matches(":indeterminate") === (progress === null),
        `Indeterminate: ${phase}`,
      );
    }
    const count = fixture.calls.filter(
      (call) => call.command === "fit_update_window",
    ).length;
    await new Promise((resolve) => setTimeout(resolve, 150));
    assert(
      count ===
        fixture.calls.filter((call) => call.command === "fit_update_window")
          .length,
      `Resize loop: ${phase}`,
    );
    results.push({
      phase,
      progress,
      height: root.height,
      fit: fixture.calls
        .filter((call) => call.command === "fit_update_window")
        .at(-1)?.height,
      percent: percent?.textContent,
      indeterminate: bar?.matches(":indeterminate"),
    });
  }
  for (const [phase, command] of [
    ["available", "start_update_install"],
    ["error", "start_update_check"],
    ["ready", "restart_application"],
  ]) {
    await fixture.set({ phase, message: "Offline interaction fixture" });
    document.querySelector(".update-popup__primary").click();
    await new Promise((resolve) => setTimeout(resolve, 50));
    assert(fixture.calls.at(-1).command === command, `Wrong action: ${phase}`);
  }
  fixture.fail("start_update_install");
  await fixture.set({ phase: "available" });
  document.querySelector(".update-popup__primary").click();
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert(
    document
      .querySelector('[role="alert"]')
      .textContent.includes("IPC failure"),
    "IPC rejection missing",
  );
  fixture.fail(null);
  document.querySelector(".update-popup__actions button").click();
  assert(
    fixture.calls.at(-1).command === "dismiss_update_window",
    "Close failed",
  );
  // Return to a useful screenshot state, also checking that errors clear.
  await fixture.set({
    phase: "downloading",
    progress: 42,
    message: "Téléchargement de Quota Monitor vtest-next…",
  });
  assert(!document.querySelector('[role="alert"]'), "Stale command error");
  return {
    width: innerWidth,
    theme: matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light",
    cases: results,
    interactions: "install/retry/restart/close/rejected IPC passed",
  };
}
