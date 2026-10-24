// Run in the offline compact fixture page console:
//   await (await import('./compact-checks.js')).checkCompact()
// Synthetic data only. Commands are recorded by the fixture, never executed.
export async function checkCompact() {
  const fixture = window.compactFixture;
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const wait = (ms = 80) => new Promise((resolve) => setTimeout(resolve, ms));
  const until = async (condition, ms) => {
    for (const end = Date.now() + ms; Date.now() < end && !condition();) await wait(20);
  };
  const results = [];
  const commands = () => fixture.calls.map((call) => call.command)
    .filter((command) => !["get_compact_panel", "request_quota_refresh"].includes(command));
  const settings = () => document.querySelector('button[aria-label="Réglages"]');
  const update = () => document.querySelector('button[aria-label="Vérifier les mises à jour"]');

  fixture.fail(null);
  await fixture.update(null);
  await fixture.scenario("two-providers");

  // Two distinct icon buttons, grouped in the top-right corner of the header.
  const header = document.querySelector(".compact__header").getBoundingClientRect();
  const icons = [...document.querySelectorAll(".compact__actions button")];
  assert(icons.length === 2, "Exactly two icon buttons");
  assert(icons[0] === settings() && icons[1] === update(), "Settings then update, by accessible name");
  for (const icon of icons) {
    const box = icon.getBoundingClientRect();
    assert(icon.querySelector("svg[aria-hidden='true']") && !icon.textContent.trim(), "Icon-only button");
    assert(box.top >= header.top && box.bottom <= header.bottom, "Icon inside header");
    assert(box.width >= 24 && box.height >= 24, "Icon target at least 24px");
    assert(box.left > innerWidth / 2, "Icon on the right half");
  }
  assert(Math.abs(icons[1].getBoundingClientRect().right - header.right) <= 12, "Update icon flush right");
  assert(settings().title === "Réglages… (⌘,)", "Settings tooltip");
  assert(update().title === "Vérifier les mises à jour…", "Update tooltip when idle");
  assert(!document.querySelector("main").innerText.includes("Vérifier les mises à jour"), "No text update action");
  assert(!document.querySelector("main").innerText.includes("Réglages…"), "No text settings action");
  results.push("icons");

  // Same order and fields as the menu panel model.
  const model = fixture.scenarios["two-providers"];
  const sections = [...document.querySelectorAll(".compact-provider")];
  assert(sections.length === model.providers.length, "Provider count");
  sections.forEach((section, p) => {
    const provider = model.providers[p];
    assert(section.querySelector(".compact-provider__head span").textContent === provider.section, "Section");
    const rows = [...section.querySelectorAll(".compact-row")];
    assert(rows.length === provider.rows.length, "Row count");
    rows.forEach((element, r) => {
      const row = provider.rows[r];
      assert(element.querySelector(".compact-row__label").textContent === row.label, `Label ${row.label}`);
      assert(element.querySelector(".compact-row__value").textContent === row.value, `Value ${row.value}`);
      assert(element.querySelector(".compact-row__reset").textContent === row.reset, `Reset ${row.reset}`);
      assert(element.querySelector(".compact-row__value").classList.contains("is-low") === row.low, "Low colour");
      const track = element.querySelector(".compact-row__track");
      const fill = element.querySelector(".compact-row__fill");
      assert(!!fill === row.fill.visible, `Fill visibility ${row.label}`);
      if (fill) assert(Math.abs(fill.getBoundingClientRect().width / track.getBoundingClientRect().width - row.fill.ratio) < 0.01, "Fill ratio");
      assert(track.getAttribute("role") === "meter" && Number(track.getAttribute("aria-valuenow")) === row.remaining, "Meter value");
      assert(element.getBoundingClientRect().height === 48, "48px rows");
    });
  });
  assert(document.documentElement.scrollWidth <= innerWidth, "No horizontal overflow");
  results.push("parity");

  // Real handlers through mocked IPC.
  fixture.calls.length = 0;
  settings().click(); await wait();
  update().click(); await wait();
  window.dispatchEvent(new KeyboardEvent("keydown", { key: ",", metaKey: true, bubbles: true })); await wait();
  assert(JSON.stringify(commands()) === JSON.stringify(["open_settings", "start_update_check", "open_settings"]), "Dispatch: " + commands());

  // Keyboard: the two icons are reachable in order and activate with Enter/Space natively.
  settings().focus();
  assert(document.activeElement === settings(), "Settings focusable");
  update().focus();
  assert(document.activeElement === update(), "Update focusable");
  results.push("handlers");

  // In-flight lock and rejected IPC.
  fixture.calls.length = 0;
  fixture.hold();
  update().click();
  await until(() => update().getAttribute("aria-disabled") === "true", 5000);
  assert(update().getAttribute("aria-busy") === "true" && update().getAttribute("aria-disabled") === "true", "Pending update busy");
  update().click(); settings().click();
  fixture.release();
  await until(() => !update().hasAttribute("aria-disabled"), 5000);
  assert(JSON.stringify(commands()) === JSON.stringify(["start_update_check"]), "Single in-flight action: " + commands());
  fixture.delay(0);
  fixture.fail("open_settings");
  settings().click();
  await until(() => document.querySelector('[role="alert"]'), 5000);
  assert(document.querySelector('[role="alert"]')?.textContent.includes("open_settings failed"),
    "Rejected IPC shown: " + document.querySelector('[role="alert"]')?.textContent + " / " + commands()
    + " / " + settings().getAttribute("aria-disabled") + " / " + update().getAttribute("aria-busy"));
  fixture.fail(null);
  settings().click();
  await until(() => !document.querySelector('[role="alert"]'), 5000);
  assert(!document.querySelector('[role="alert"]'), "Alert cleared after success");
  results.push("pending/error");

  // Updater states exposed on the update icon.
  const states = [
    [{ phase: "checking" }, "is-busy", "true", "Recherche de mises à jour en cours…"],
    [{ phase: "downloading", progress: 42 }, "is-busy", "true", "Téléchargement de Quota Monitor vfixture-next en cours (42 %)…"],
    [{ phase: "installing" }, "is-busy", "true", "Installation de Quota Monitor vfixture-next en cours…"],
    [{ phase: "available" }, "is-available", "false", "Mise à jour disponible : Quota Monitor vfixture-next."],
    [{ phase: "ready" }, "is-ready", "false", "Quota Monitor vfixture-next est installé : relancez l’application."],
    [{ phase: "error" }, "is-error", "false", "La dernière vérification des mises à jour a échoué."],
    [{ phase: "up-to-date" }, "is-idle", "false", "Quota Monitor est à jour."],
  ];
  for (const [state, cls, busy, description] of states) {
    await fixture.update(state);
    const button = update();
    assert(button.classList.contains(cls), `${state.phase} class`);
    assert(button.getAttribute("aria-busy") === busy, `${state.phase} aria-busy`);
    const described = document.getElementById(button.getAttribute("aria-describedby"));
    assert(described.textContent === (cls === "is-idle" ? "" : description), `${state.phase} description`);
    assert(button.title.includes(description) || cls === "is-idle", `${state.phase} tooltip`);
    assert(!!button.querySelector(".compact-icon__badge") === ["is-available", "is-ready", "is-error"].includes(cls), `${state.phase} badge`);
  }
  await fixture.update(null);
  results.push("update states");

  // Other menu states: notes, connection action, empty.
  await fixture.scenario("codex-error");
  assert(document.querySelector(".compact-card__note.compact-tone--error"), "Error note");
  fixture.calls.length = 0;
  document.querySelector(".compact__connect button").click(); await wait();
  assert(commands()[0] === "connect_codex_account", "Connect action");
  assert(document.documentElement.scrollWidth <= innerWidth, "Long note wraps");
  await fixture.scenario("logging-in");
  assert(document.querySelector(".compact__connect button").disabled, "Logging in disabled");
  await fixture.scenario("none");
  assert(document.querySelectorAll(".compact-provider").length === 0, "No provider");
  assert(document.querySelector(".compact__capsule").textContent.includes("Inactif"), "Inactive capsule");
  assert(settings() && update(), "Settings stays reachable with no provider");
  await fixture.scenario("two-providers");
  results.push("states");
  return { width: innerWidth, dark: matchMedia("(prefers-color-scheme: dark)").matches,
    reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches, results };
}
