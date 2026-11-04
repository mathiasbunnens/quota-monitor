import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import AddAccountFlow, { ABSOLUTE_PATH, ABSOLUTE_PATH_ERROR } from "./AddAccountFlow";
import PlusIcon from "./PlusIcon";

type Provider = "Claude" | "Codex";
const commands = {
  Codex: { add: "add_codex_profile", choose: "choose_codex_profile_folder" },
  Claude: { add: "add_claude_profile", choose: "choose_claude_profile_folder" },
} as const;

/** + at the bottom right of a provider (Dashboard): same flow, same commands as
 *  Settings → Fournisseurs. The backend validates, deduplicates and follows. */
export default function AddAccountButton({ provider, onChange }: { provider: Provider; onChange: () => void }) {
  const [open, setOpen] = useState(false);
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [canChoose, setCanChoose] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const command = commands[provider];

  useEffect(() => {
    let disposed = false;
    void invoke<string>("get_platform").then((platform) => { if (!disposed) setCanChoose(platform === "macos"); }).catch(() => undefined);
    return () => { disposed = true; };
  }, []);

  const close = () => { setOpen(false); setError(null); window.requestAnimationFrame(() => button.current?.focus()); };
  const add = async (value: string) => {
    setBusy(true); setError(null);
    try { await invoke(command.add, { path: value }); setPath(""); setOpen(false); onChange(); return true; }
    catch (cause) { setError(String(cause).replace(/^Error: /, "")); return false; }
    finally { setBusy(false); }
  };
  const choose = async () => {
    setBusy(true); setError(null);
    let chosen: string | null = null;
    try { chosen = await invoke<string | null>(command.choose); }
    catch (cause) { setError(String(cause).replace(/^Error: /, "")); }
    finally { setBusy(false); }
    if (chosen) await add(chosen);
  };
  const submit = async () => {
    if (!ABSOLUTE_PATH.test(path)) { setError(ABSOLUTE_PATH_ERROR); return; }
    await add(path);
  };

  return <div className="add-account">
    <button type="button" ref={button} className="add-account__button" disabled={busy}
      aria-label={"Ajouter un compte " + provider} title={"Ajouter un compte " + provider} aria-expanded={open}
      onClick={() => { setOpen((value) => !value); window.requestAnimationFrame(() => input.current?.focus()); }}><PlusIcon /></button>
    {open && <AddAccountFlow provider={provider} path={path} error={error} busy={busy} canChoose={canChoose} inputRef={input}
      onPath={setPath} onChoose={() => void choose()} onSubmit={() => void submit()} onClose={close} />}
  </div>;
}
