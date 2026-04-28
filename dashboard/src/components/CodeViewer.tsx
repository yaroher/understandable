import { useEffect, useState } from "react";
import { api, useDashboardStore } from "../store";

/**
 * Hard cap on the number of lines we render in the browser. The server
 * already enforces a byte cap on `/api/source`, but we re-clip on the
 * client as a defense-in-depth measure: any future server bug can't
 * accidentally lock the viewer up by streaming megabytes of source.
 */
const MAX_LINES = 500;

interface SourceState {
  status: "idle" | "loading" | "ok" | "error";
  /** Captured source slice (already truncated to MAX_LINES). */
  text: string;
  /** True when we hit the local 500-line cap during display. */
  truncated: boolean;
  error: string | null;
}

const INITIAL: SourceState = {
  status: "idle",
  text: "",
  truncated: false,
  error: null,
};

export default function CodeViewer() {
  const graph = useDashboardStore((s) => s.graph);
  const codeViewerNodeId = useDashboardStore((s) => s.codeViewerNodeId);
  const closeCodeViewer = useDashboardStore((s) => s.closeCodeViewer);

  const node = graph?.nodes.find((n) => n.id === codeViewerNodeId) ?? null;

  const [source, setSource] = useState<SourceState>(INITIAL);
  const [reloadToken, setReloadToken] = useState(0);

  const filePath = node?.filePath ?? null;
  const start = node?.lineRange?.[0];
  const end = node?.lineRange?.[1];

  useEffect(() => {
    // Reset when the active node changes — prevents flashing stale text.
    setSource(INITIAL);
    if (!filePath) return;
    let cancelled = false;
    setSource({ ...INITIAL, status: "loading" });
    api
      .source(filePath, start, end)
      .then((text) => {
        if (cancelled) return;
        const lines = text.split(/\r?\n/);
        const truncated = lines.length > MAX_LINES;
        const clipped = truncated ? lines.slice(0, MAX_LINES).join("\n") : text;
        setSource({
          status: "ok",
          text: clipped,
          truncated,
          error: null,
        });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        const msg = err instanceof Error ? err.message : String(err);
        setSource({ status: "error", text: "", truncated: false, error: msg });
      });
    return () => {
      cancelled = true;
    };
  }, [filePath, start, end, reloadToken]);

  if (!node) {
    return (
      <div className="h-full w-full flex items-center justify-center bg-surface">
        <p className="text-text-muted text-sm">No file selected</p>
      </div>
    );
  }

  const lineInfo = node.lineRange
    ? `Lines ${node.lineRange[0]}–${node.lineRange[1]}`
    : "Full file";

  // First-line offset for the gutter when a slice was requested.
  const firstLineNo = node.lineRange?.[0] ?? 1;

  return (
    <div className="h-full w-full flex flex-col bg-surface overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-2.5 bg-elevated border-b border-border-subtle shrink-0">
        <span
          className="text-[10px] font-semibold uppercase tracking-wider px-2 py-0.5 rounded border"
          style={{
            color: "var(--color-node-file)",
            borderColor: "color-mix(in srgb, var(--color-node-file) 30%, transparent)",
            backgroundColor: "color-mix(in srgb, var(--color-node-file) 10%, transparent)",
          }}
        >
          {node.type}
        </span>
        <span className="text-sm font-serif text-text-primary truncate">
          {node.name}
        </span>
        {node.filePath && (
          <span className="text-xs font-mono text-text-muted truncate ml-auto">
            {node.filePath}
          </span>
        )}
        <span className="text-[10px] text-text-muted">{lineInfo}</span>
        <button
          onClick={closeCodeViewer}
          className="text-text-muted hover:text-text-primary ml-2 transition-colors"
          aria-label="Close"
        >
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      {/* Body */}
      <div className="flex-1 overflow-auto p-5">
        {/* Summary */}
        <div className="mb-4">
          <h4 className="text-[11px] font-semibold text-accent uppercase tracking-wider mb-2">Summary</h4>
          <p className="text-sm text-text-secondary leading-relaxed">{node.summary}</p>
        </div>

        {/* Language notes callout */}
        {node.languageNotes && (
          <div className="mb-4 bg-accent/5 border border-accent/20 rounded-lg p-3">
            <h4 className="text-[11px] font-semibold text-accent uppercase tracking-wider mb-1.5">Language Notes</h4>
            <p className="text-sm text-text-secondary leading-relaxed">{node.languageNotes}</p>
          </div>
        )}

        {/* Tags */}
        {node.tags.length > 0 && (
          <div className="mb-4">
            <h4 className="text-[11px] font-semibold text-accent uppercase tracking-wider mb-2">Tags</h4>
            <div className="flex flex-wrap gap-1.5">
              {node.tags.map((tag) => (
                <span key={tag} className="text-[11px] glass text-text-secondary px-2.5 py-1 rounded-full">
                  {tag}
                </span>
              ))}
            </div>
          </div>
        )}

        {/* Source code panel */}
        {node.filePath ? (
          <div className="mb-2">
            <div className="flex items-center gap-2 mb-2">
              <h4 className="text-[11px] font-semibold text-accent uppercase tracking-wider">Source</h4>
              {source.status === "ok" && source.truncated && (
                <span className="text-[10px] text-text-muted italic">
                  truncated to {MAX_LINES} lines
                </span>
              )}
            </div>

            {source.status === "loading" && (
              <div className="bg-elevated border border-border-subtle rounded p-3 text-[11px] text-text-muted">
                Loading source…
              </div>
            )}

            {source.status === "error" && (
              <div className="bg-red-900/20 border border-red-700/40 rounded p-3 text-[11px] text-red-200 flex items-center justify-between gap-3">
                <span className="font-mono truncate">
                  {source.error ?? "Failed to load source"}
                </span>
                <button
                  type="button"
                  onClick={() => setReloadToken((t) => t + 1)}
                  className="shrink-0 text-[10px] uppercase font-semibold px-2 py-1 rounded border border-red-500/50 hover:bg-red-500/20"
                >
                  Try again
                </button>
              </div>
            )}

            {source.status === "ok" && (
              // TODO: hook in syntax highlighting (Shiki or PrismJS) once the
              // dashboard's bundle budget can absorb it. For now the viewer
              // intentionally renders plain monospaced text — the goal is
              // correctness over polish.
              <pre className="bg-elevated border border-border-subtle rounded p-3 text-[11px] leading-snug text-text-secondary overflow-x-auto font-mono whitespace-pre">
                <code>
                  {source.text.split("\n").map((line, i) => (
                    <div key={i} className="flex gap-3">
                      <span className="text-text-muted/50 select-none w-10 text-right shrink-0">
                        {firstLineNo + i}
                      </span>
                      <span className="flex-1">{line || " "}</span>
                    </div>
                  ))}
                </code>
              </pre>
            )}
          </div>
        ) : (
          <div className="text-[11px] text-text-muted italic">
            This node has no associated source file.
          </div>
        )}
      </div>
    </div>
  );
}
