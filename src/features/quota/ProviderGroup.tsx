import type { ReactNode } from "react";

type Props = {
  /** Stable id of the provider heading (also its landmark name). */
  id: string;
  title: string;
  /** The title is the detailed-usage link: `icon` is the external arrow. */
  usage: { label: string; onOpen: () => void; icon: ReactNode };
  /** Several cards side by side (paired layout). */
  columns?: boolean;
  /** The + of this provider, bottom right. */
  footer?: ReactNode;
  children?: ReactNode;
};

/** One provider on the Dashboard: header « Codex ↗ » once, its account cards, then its +. */
export default function ProviderGroup({ id, title, usage, columns = false, footer, children }: Props) {
  return <section className="provider-group" aria-labelledby={id}>
    <header className="provider-group__header">
      <h2 id={id}><button type="button" className="provider-card__usage" aria-label={usage.label} title={usage.label} onClick={usage.onOpen}>{title} {usage.icon}</button></h2>
    </header>
    <div className={"provider-group__cards" + (columns ? " provider-group__cards--columns" : "")}>{children}</div>
    {footer && <footer className="provider-group__footer">{footer}</footer>}
  </section>;
}
