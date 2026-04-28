import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { GraphNode, NodeType } from "../core/types";
import { api, useDashboardStore } from "../store";

const PAGE_SIZE = 200;

/**
 * Modal browser for the full node list, paginated server-side via
 * `GET /api/graph/nodes?limit=…&offset=…`. Lets the user filter by
 * type, layer, or substring, and click into any row to select it.
 *
 * This is an alternate surface to the in-canvas selection UI — it keeps
 * working even on multi-thousand-node graphs where the React Flow
 * fast-path bows out.
 */
export default function BrowsePanel() {
  const open = useDashboardStore((s) => s.browsePanelOpen);
  const setBrowsePanelOpen = useDashboardStore((s) => s.setBrowsePanelOpen);
  const close = useCallback(() => setBrowsePanelOpen(false), [setBrowsePanelOpen]);
  const kind = useDashboardStore((s) => s.graphKind);
  const graph = useDashboardStore((s) => s.graph);
  const navigateToNode = useDashboardStore((s) => s.navigateToNode);

  const [items, setItems] = useState<GraphNode[]>([]);
  const [total, setTotal] = useState<number>(0);
  const [offset, setOffset] = useState<number>(0);
  const [type, setType] = useState<string>("");
  const [layer, setLayer] = useState<string>("");
  const [q, setQ] = useState<string>("");
  const [debouncedQ, setDebouncedQ] = useState<string>("");
  const [loading, setLoading] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  // Debounce q updates so we don't fire one request per keystroke.
  useEffect(() => {
    const t = setTimeout(() => setDebouncedQ(q.trim()), 200);
    return () => clearTimeout(t);
  }, [q]);

  // Reset offset whenever a filter changes.
  useEffect(() => {
    setOffset(0);
  }, [type, layer, debouncedQ, kind]);

  // Track in-flight request id so stale responses can't clobber state.
  const seqRef = useRef(0);

  const reqArgs = useMemo(
    () => ({
      kind,
      type: type || undefined,
      layer: layer || undefined,
      q: debouncedQ || undefined,
      limit: PAGE_SIZE,
      offset,
    }),
    [kind, type, layer, debouncedQ, offset],
  );

  useEffect(() => {
    if (!open) return;
    const id = ++seqRef.current;
    setLoading(true);
    setError(null);
    api
      .listNodes(reqArgs)
      .then((page) => {
        if (id !== seqRef.current) return;
        setItems(page.items);
        setTotal(page.total);
      })
      .catch((err: unknown) => {
        if (id !== seqRef.current) return;
        const msg = err instanceof Error ? err.message : String(err);
        setError(msg);
        setItems([]);
        setTotal(0);
      })
      .finally(() => {
        if (id !== seqRef.current) return;
        setLoading(false);
      });
  }, [open, reqArgs]);

  // Build a unique sorted list of types and layers from the local graph
  // for filter chips. Falls back to empty arrays when the graph isn't
  // loaded yet (the modal can still browse via search).
  const knownTypes = useMemo<string[]>(() => {
    if (!graph) return [];
    const seen = new Set<string>();
    for (const n of graph.nodes) seen.add(n.type);
    return [...seen].sort();
  }, [graph]);
  const knownLayers = useMemo<{ id: string; name: string }[]>(() => {
    if (!graph) return [];
    return graph.layers.map((l) => ({ id: l.id, name: l.name }));
  }, [graph]);

  const onPick = useCallback(
    (node: GraphNode) => {
      navigateToNode(node.id);
      close();
    },
    [navigateToNode, close],
  );

  if (!open) return null;

  const pageStart = offset + 1;
  const pageEnd = Math.min(offset + items.length, total);
  const hasPrev = offset > 0;
  const hasNext = offset + items.length < total;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div className="bg-surface border border-border-subtle rounded-lg shadow-xl w-[min(900px,92vw)] max-h-[85vh] flex flex-col">
        {/* Header */}
        <div className="flex items-center gap-3 px-4 py-3 border-b border-border-subtle">
          <h2 className="text-sm font-serif text-text-primary">Browse nodes</h2>
          <span className="text-[11px] text-text-muted">
            {loading
              ? "loading…"
              : total === 0
                ? "no results"
                : `${pageStart}–${pageEnd} of ${total}`}
          </span>
          <button
            onClick={close}
            className="ml-auto text-text-muted hover:text-text-primary text-xs"
          >
            Close
          </button>
        </div>

        {/* Filters */}
        <div className="px-4 py-3 border-b border-border-subtle space-y-2">
          <input
            type="search"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Filter by name or summary…"
            className="w-full bg-elevated border border-border-subtle rounded px-3 py-1.5 text-sm text-text-primary placeholder:text-text-muted focus:border-accent outline-none"
          />
          <div className="flex flex-wrap gap-1.5">
            <button
              onClick={() => setType("")}
              className={`text-[10px] uppercase px-2 py-0.5 rounded border transition-colors ${
                type === ""
                  ? "border-accent text-accent bg-accent/10"
                  : "border-border-subtle text-text-muted hover:text-text-secondary"
              }`}
            >
              All types
            </button>
            {knownTypes.map((t) => (
              <button
                key={t}
                onClick={() => setType(t === type ? "" : t)}
                className={`text-[10px] uppercase px-2 py-0.5 rounded border transition-colors ${
                  type === t
                    ? "border-accent text-accent bg-accent/10"
                    : "border-border-subtle text-text-muted hover:text-text-secondary"
                }`}
              >
                {t}
              </button>
            ))}
          </div>
          {knownLayers.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              <button
                onClick={() => setLayer("")}
                className={`text-[10px] uppercase px-2 py-0.5 rounded border transition-colors ${
                  layer === ""
                    ? "border-gold text-gold bg-gold/10"
                    : "border-border-subtle text-text-muted hover:text-text-secondary"
                }`}
              >
                All layers
              </button>
              {knownLayers.map((l) => (
                <button
                  key={l.id}
                  onClick={() => setLayer(l.id === layer ? "" : l.id)}
                  className={`text-[10px] uppercase px-2 py-0.5 rounded border transition-colors ${
                    layer === l.id
                      ? "border-gold text-gold bg-gold/10"
                      : "border-border-subtle text-text-muted hover:text-text-secondary"
                  }`}
                  title={l.id}
                >
                  {l.name}
                </button>
              ))}
            </div>
          )}
        </div>

        {/* List */}
        <div className="flex-1 overflow-auto">
          {error && (
            <div className="px-4 py-3 text-xs text-red-300 bg-red-900/20 border-b border-red-700/40">
              {error}
            </div>
          )}
          {!error && items.length === 0 && !loading && (
            <div className="p-6 text-center text-text-muted text-sm">
              No nodes match.
            </div>
          )}
          <ul>
            {items.map((node) => (
              <li key={node.id}>
                <button
                  type="button"
                  onClick={() => onPick(node)}
                  className="w-full text-left flex items-center gap-3 px-4 py-2 hover:bg-elevated/60 border-b border-border-subtle/40 transition-colors"
                >
                  <span className="text-[10px] font-semibold uppercase tracking-wider px-1.5 py-0.5 rounded border border-border-subtle text-text-muted shrink-0">
                    {node.type as NodeType}
                  </span>
                  <span className="text-sm text-text-primary truncate">
                    {node.name}
                  </span>
                  {node.filePath && (
                    <span className="text-[11px] font-mono text-text-muted truncate ml-auto max-w-[40%]">
                      {node.filePath}
                    </span>
                  )}
                </button>
              </li>
            ))}
          </ul>
        </div>

        {/* Pagination footer */}
        <div className="flex items-center gap-2 px-4 py-2 border-t border-border-subtle">
          <button
            disabled={!hasPrev || loading}
            onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}
            className="text-xs px-2 py-1 rounded bg-elevated text-text-secondary hover:text-text-primary disabled:opacity-40 disabled:cursor-not-allowed"
          >
            Prev
          </button>
          <button
            disabled={!hasNext || loading}
            onClick={() => setOffset(offset + PAGE_SIZE)}
            className="text-xs px-2 py-1 rounded bg-elevated text-text-secondary hover:text-text-primary disabled:opacity-40 disabled:cursor-not-allowed"
          >
            Next
          </button>
          <span className="text-[11px] text-text-muted ml-auto">
            page size {PAGE_SIZE}
          </span>
        </div>
      </div>
    </div>
  );
}
