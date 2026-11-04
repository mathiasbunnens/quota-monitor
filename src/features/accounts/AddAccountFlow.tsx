import type { RefObject } from "react";
import "./add-account.scss";

/** Absolute path on macOS/Linux (/…) or Windows (C:\… or \\server). */
export const ABSOLUTE_PATH = /^(\/|[A-Za-z]:[\\/]|\\\\)/;
export const ABSOLUTE_PATH_ERROR = "Chemin absolu requis (il commence par /).";

type Props = {
  provider: "Claude" | "Codex";
  path: string;
  error: string | null;
  busy: boolean;
  /** Native folder chooser available (macOS). The path field is always there. */
  canChoose: boolean;
  inputRef?: RefObject<HTMLInputElement | null>;
  onPath: (path: string) => void;
  onChoose: () => void;
  onSubmit: () => void;
  onClose: () => void;
};

/** The one add-account flow, opened by the + of Settings and of the Dashboard:
 *  folder chooser AND typed absolute path in the same small inline panel
 *  (no extra window). Controlled: the caller owns state and commands. */
export default function AddAccountFlow({ provider, path, error, busy, canChoose, inputRef, onPath, onChoose, onSubmit, onClose }: Props) {
  const codex = provider === "Codex";
  return <div className="add-account__flow" role="group" aria-label={"Ajouter un compte " + provider}
    onKeyDown={(event) => { if (event.key === "Escape") { event.stopPropagation(); onClose(); } }}>
    <form className="add-account__manual" onSubmit={(event) => { event.preventDefault(); onSubmit(); }}>
      {canChoose && <button type="button" className="add-account__choose" aria-label="Choisir un dossier" title="Choisir un dossier" disabled={busy} onClick={onChoose}>
        <svg width="16" height="16" viewBox="0 0 24 24" className="add-account__glyph" aria-hidden="true"><path d="M3 7V5a1 1 0 0 1 1-1h5l2 3h9a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1Z" /></svg>
      </button>}
      <input ref={inputRef} value={path} disabled={busy} spellCheck={false} aria-invalid={error ? true : undefined}
        placeholder={codex ? "/Users/nom/.codex-travail" : "/Users/nom/.claude-travail"}
        aria-label={codex ? "Chemin absolu d’un dossier CODEX_HOME" : "Chemin absolu d’un profil Claude Code"}
        onChange={(event) => onPath(event.target.value)} />
      <button type="submit" disabled={busy || !path}>Ajouter</button>
    </form>
    {error && <p role="alert" className="add-account__error">{error}</p>}
  </div>;
}
