import type { GraphKind } from "../store";

/**
 * Small graph-kind dropdown. The parent owns the kind value + change
 * handler so this component can stay dumb. It is intentionally keyed
 * by the URL `?kind=` param via the parent in `App.tsx`.
 *
 * `kindMissing` is populated lazily as the user attempts to switch to
 * kinds that haven't been built yet. Options are still selectable so the
 * store can show a toast and revert — we just annotate them visually.
 */
export function KindSwitcher({
  kind,
  kindMissing = {},
  onChange,
  disabled = false,
}: {
  kind: GraphKind;
  kindMissing?: Partial<Record<GraphKind, boolean>>;
  onChange: (k: GraphKind) => void;
  disabled?: boolean;
}) {
  function label(k: GraphKind, display: string): string {
    return kindMissing[k] ? `${display} (not built)` : display;
  }

  return (
    <label className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider text-text-muted">
      <span className="font-semibold">Kind</span>
      <select
        value={kind}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value as GraphKind)}
        className="bg-elevated text-text-primary border border-border-subtle rounded px-1.5 py-0.5 text-[11px] hover:border-accent/40 focus:border-accent transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
        title="Switch graph kind (codebase / domain / knowledge)"
      >
        <option value="codebase">{label("codebase", "Codebase")}</option>
        <option value="domain">{label("domain", "Domain")}</option>
        <option value="knowledge">{label("knowledge", "Knowledge")}</option>
      </select>
    </label>
  );
}

export default KindSwitcher;
