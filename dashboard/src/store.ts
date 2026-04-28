import { create } from "zustand";
import { SearchEngine } from "./core/search";
import type { SearchResult } from "./core/search";
import type {
  GraphEdge,
  GraphNode,
  KnowledgeGraph,
  Layer,
  ProjectMeta,
  TourStep,
} from "./core/types";
import { validateGraph } from "./core/schema";
import type { ReactFlowInstance } from "@xyflow/react";

// ---------------------------------------------------------------------------
// API client
// ---------------------------------------------------------------------------
//
// The Rust `understandable` server exposes a JSON REST API under `/api/*`
// (see `crates/ua-server/src/routes/api.rs`). The dashboard talks to that
// server in production *and* in dev (Vite proxies `/api` to it — see
// `vite.config.ts`).
//
// We keep a graceful fallback to the legacy static `knowledge-graph.json`
// blob bundled into `dashboard/public/`. That fallback is what lets the
// `dist/` build serve usefully when somebody opens `index.html` from the
// filesystem or hosts it without the Rust server attached. The fallback
// is *only* exercised when `/api/health` does not respond — once we know
// the server is alive we always use the live endpoints.

/** Graph kind selector matching the Rust server's `GraphKind` enum. */
export type GraphKind = "codebase" | "domain" | "knowledge";

export const DEFAULT_GRAPH_KIND: GraphKind = "codebase";

/**
 * Resolve the graph kind from the URL's `?kind=` query parameter. Falls
 * back to `codebase` when the value is missing or unrecognised.
 */
export function readGraphKindFromUrl(): GraphKind {
  if (typeof window === "undefined") return DEFAULT_GRAPH_KIND;
  try {
    const raw = new URL(window.location.href).searchParams
      .get("kind")
      ?.toLowerCase();
    if (raw === "codebase" || raw === "domain" || raw === "knowledge") {
      return raw;
    }
  } catch {
    /* ignore malformed URLs */
  }
  return DEFAULT_GRAPH_KIND;
}

interface HealthResponse {
  status: string;
  version: string;
}

/**
 * Probe the server's health endpoint with a short timeout. Returns
 * `null` when the probe fails for any reason (no server, file:// origin,
 * timeout, malformed JSON). Logs a single line so the dev console makes
 * the active mode obvious.
 */
async function probeHealth(): Promise<HealthResponse | null> {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), 1500);
  try {
    const res = await fetch("/api/health", { signal: ctrl.signal });
    if (!res.ok) {
      console.warn(
        `[ua-api] /api/health returned ${res.status}; falling back to static graph`,
      );
      return null;
    }
    const json = (await res.json()) as HealthResponse;
    console.info(`[ua-api] connected to understandable server v${json.version}`);
    return json;
  } catch (err) {
    console.warn(
      `[ua-api] /api/health unreachable (${err instanceof Error ? err.message : String(err)}); falling back to /knowledge-graph.json`,
    );
    return null;
  } finally {
    clearTimeout(timer);
  }
}

async function fetchJson<T>(path: string): Promise<T> {
  const res = await fetch(path);
  if (!res.ok) {
    throw new Error(`GET ${path} -> ${res.status} ${res.statusText}`);
  }
  return (await res.json()) as T;
}

/** Append `?kind=…` to a path that already has no query string. */
function withKind(path: string, kind: GraphKind): string {
  const sep = path.includes("?") ? "&" : "?";
  return `${path}${sep}kind=${encodeURIComponent(kind)}`;
}

export interface NodePage {
  items: GraphNode[];
  total: number;
  limit: number;
  offset: number;
}

export interface EdgePage {
  items: GraphEdge[];
  total: number;
  limit: number;
  offset: number;
}

export interface SearchHit {
  id: string;
  score: number;
}

export interface NeighborhoodResponse {
  center: GraphNode;
  neighbors: GraphNode[];
  edges: GraphEdge[];
}

export interface DiffOverlay {
  changedNodeIds: string[];
  affectedNodeIds: string[];
}

export interface UnderstandableApi {
  health(): Promise<HealthResponse | null>;
  fullGraph(kind?: GraphKind): Promise<KnowledgeGraph>;
  listNodes(opts?: {
    kind?: GraphKind;
    type?: string;
    layer?: string;
    q?: string;
    limit?: number;
    offset?: number;
  }): Promise<NodePage>;
  listEdges(opts?: {
    kind?: GraphKind;
    source?: string;
    target?: string;
    type?: string;
    limit?: number;
    offset?: number;
  }): Promise<EdgePage>;
  project(): Promise<ProjectMeta>;
  layers(kind?: GraphKind): Promise<Layer[]>;
  tour(kind?: GraphKind): Promise<TourStep[]>;
  search(q: string, type?: string, limit?: number): Promise<SearchHit[]>;
  node(id: string): Promise<GraphNode>;
  neighbors(id: string, depth?: number): Promise<NeighborhoodResponse>;
  source(path: string, start?: number, end?: number): Promise<string>;
  diff(): Promise<DiffOverlay | null>;
}

function buildQuery(params: Record<string, string | number | undefined>): string {
  const entries = Object.entries(params).filter(
    ([, v]) => v !== undefined && v !== "",
  );
  if (entries.length === 0) return "";
  const search = new URLSearchParams();
  for (const [k, v] of entries) search.set(k, String(v));
  return `?${search.toString()}`;
}

/**
 * Tiny client for the Rust `/api/*` routes. Each method maps 1:1 to a
 * route in `crates/ua-server/src/routes/api.rs`.
 */
export const api: UnderstandableApi = {
  health: probeHealth,
  fullGraph: (kind = DEFAULT_GRAPH_KIND) =>
    fetchJson<KnowledgeGraph>(withKind("/api/graph", kind)),
  listNodes: ({
    kind = DEFAULT_GRAPH_KIND,
    type,
    layer,
    q,
    limit,
    offset,
  } = {}) =>
    fetchJson<NodePage>(
      `/api/graph/nodes${buildQuery({ kind, type, layer, q, limit, offset })}`,
    ),
  listEdges: ({
    kind = DEFAULT_GRAPH_KIND,
    source,
    target,
    type,
    limit,
    offset,
  } = {}) =>
    fetchJson<EdgePage>(
      `/api/graph/edges${buildQuery({ kind, source, target, type, limit, offset })}`,
    ),
  project: () => fetchJson<ProjectMeta>("/api/project"),
  layers: (kind = DEFAULT_GRAPH_KIND) =>
    fetchJson<Layer[]>(withKind("/api/layers", kind)),
  tour: (kind = DEFAULT_GRAPH_KIND) =>
    fetchJson<TourStep[]>(withKind("/api/tour", kind)),
  search: (q, type, limit) =>
    fetchJson<SearchHit[]>(`/api/search${buildQuery({ q, type, limit })}`),
  node: (id) => fetchJson<GraphNode>(`/api/node${buildQuery({ id })}`),
  neighbors: (id, depth) =>
    fetchJson<NeighborhoodResponse>(
      `/api/neighbors${buildQuery({ id, depth })}`,
    ),
  source: async (path, start, end) => {
    const url = `/api/source${buildQuery({ path, start, end })}`;
    const res = await fetch(url);
    if (!res.ok) {
      throw new Error(
        `GET ${url} -> ${res.status} ${res.statusText}: ${await res.text()}`,
      );
    }
    return await res.text();
  },
  diff: async () => {
    const res = await fetch("/api/diff");
    if (res.status === 204) return null;
    if (!res.ok) {
      throw new Error(`GET /api/diff -> ${res.status} ${res.statusText}`);
    }
    return (await res.json()) as DiffOverlay;
  },
};

/**
 * Resolve the boot-time graph for the dashboard. Tries `/api/health`
 * first; if it succeeds, fetches `/api/graph?kind=…`. On any failure
 * (no server, network error, validation error) we fall back to the
 * legacy bundled `knowledge-graph.json`.
 *
 * Returns the graph + a flag telling the caller which mode is in use,
 * so the UI can surface that information if needed.
 */
export async function loadInitialGraph(
  kind: GraphKind = readGraphKindFromUrl(),
): Promise<{
  graph: KnowledgeGraph;
  mode: "api" | "static";
} | null> {
  const health = await probeHealth();

  if (health) {
    try {
      const graph = await api.fullGraph(kind);
      const result = validateGraph(graph);
      if (result.success && result.data) {
        return { graph: result.data, mode: "api" };
      }
      console.error(
        "[ua-api] /api/graph returned data that failed schema validation:",
        result.fatal,
      );
    } catch (err) {
      console.error(
        `[ua-api] failed to load /api/graph?kind=${kind}:`,
        err,
      );
    }
  }

  // Fallback to static JSON for dev / file:// / standalone preview.
  try {
    const res = await fetch("/knowledge-graph.json");
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const json: unknown = await res.json();
    const result = validateGraph(json);
    if (result.success && result.data) {
      console.info(
        "[ua-api] using static /knowledge-graph.json (development fallback)",
      );
      return { graph: result.data, mode: "static" };
    }
    console.error(
      "[ua-api] static /knowledge-graph.json failed validation:",
      result.fatal,
    );
  } catch (err) {
    console.error("[ua-api] static knowledge-graph.json fallback failed:", err);
  }

  return null;
}

export type Persona = "non-technical" | "junior" | "experienced";
export type NavigationLevel = "overview" | "layer-detail";
export type NodeType = "file" | "function" | "class" | "module" | "concept" | "config" | "document" | "service" | "table" | "endpoint" | "pipeline" | "schema" | "resource" | "domain" | "flow" | "step" | "article" | "entity" | "topic" | "claim" | "source";
export type Complexity = "simple" | "moderate" | "complex";
export type EdgeCategory = "structural" | "behavioral" | "data-flow" | "dependencies" | "semantic" | "infrastructure" | "domain" | "knowledge";
export type ViewMode = "structural" | "domain" | "knowledge";

export interface FilterState {
  nodeTypes: Set<NodeType>;
  complexities: Set<Complexity>;
  layerIds: Set<string>;
  edgeCategories: Set<EdgeCategory>;
}

export const ALL_NODE_TYPES: NodeType[] = ["file", "function", "class", "module", "concept", "config", "document", "service", "table", "endpoint", "pipeline", "schema", "resource", "domain", "flow", "step", "article", "entity", "topic", "claim", "source"];
export const ALL_COMPLEXITIES: Complexity[] = ["simple", "moderate", "complex"];
export const ALL_EDGE_CATEGORIES: EdgeCategory[] = ["structural", "behavioral", "data-flow", "dependencies", "semantic", "infrastructure", "domain", "knowledge"];

export const EDGE_CATEGORY_MAP: Record<EdgeCategory, string[]> = {
  structural: ["imports", "exports", "contains", "inherits", "implements"],
  behavioral: ["calls", "subscribes", "publishes", "middleware"],
  "data-flow": ["reads_from", "writes_to", "transforms", "validates"],
  dependencies: ["depends_on", "tested_by", "configures"],
  semantic: ["related", "similar_to"],
  infrastructure: ["deploys", "serves", "provisions", "triggers", "migrates", "documents", "routes", "defines_schema"],
  domain: ["contains_flow", "flow_step", "cross_domain"],
  knowledge: ["cites", "contradicts", "builds_on", "exemplifies", "categorized_under", "authored_by"],
};

export const DOMAIN_EDGE_TYPES = EDGE_CATEGORY_MAP.domain;

const DEFAULT_FILTERS: FilterState = {
  nodeTypes: new Set<NodeType>(ALL_NODE_TYPES),
  complexities: new Set<Complexity>(ALL_COMPLEXITIES),
  layerIds: new Set<string>(),
  edgeCategories: new Set<EdgeCategory>(ALL_EDGE_CATEGORIES),
};

/** Categories used for node type filter toggles. Single source of truth for NodeCategory. */
export type NodeCategory = "code" | "config" | "docs" | "infra" | "data" | "domain" | "knowledge";

/** Find which layer a node belongs to. Returns layerId or null. */
function findNodeLayer(graph: KnowledgeGraph, nodeId: string): string | null {
  for (const layer of graph.layers) {
    if (layer.nodeIds.includes(nodeId)) return layer.id;
  }
  return null;
}

/** Maximum number of entries in the sidebar navigation history. */
const MAX_HISTORY = 50;

/** Result row coming back from `api.search` (server-side hits). */
export interface RemoteSearchHit {
  id: string;
  score: number;
  node?: GraphNode;
}

interface DashboardStore {
  graph: KnowledgeGraph | null;
  /** Nodes fetched ad-hoc via `api.node(id)` for ids missing from `graph`. */
  singleNodes: Map<string, GraphNode>;
  /** Edges fetched ad-hoc via `api.listEdges` for ids missing from `graph`. */
  extraEdges: Map<string, GraphEdge[]>;
  /** Current graph kind (URL `?kind=`). */
  graphKind: GraphKind;
  /** Browse modal toggle. */
  browsePanelOpen: boolean;
  /** Last neighbor expansion result for the keyboard shortcut. */
  lastNeighborhood: NeighborhoodResponse | null;
  /** Server-side search hits, populated by `searchRemote`. */
  remoteSearchResults: RemoteSearchHit[];
  /** True while a remote search is in flight. */
  remoteSearchLoading: boolean;
  selectedNodeId: string | null;
  searchQuery: string;
  searchResults: SearchResult[];
  searchEngine: SearchEngine | null;
  searchMode: "fuzzy" | "semantic";
  setSearchMode: (mode: "fuzzy" | "semantic") => void;
  setBrowsePanelOpen: (open: boolean) => void;
  toggleBrowsePanel: () => void;
  setGraphKind: (kind: GraphKind) => Promise<void>;
  searchRemote: (q: string, type?: string) => void;
  setNeighbors: (nodeId: string, depth?: number) => Promise<NeighborhoodResponse | null>;
  ensureNode: (id: string) => Promise<GraphNode | null>;
  /**
   * Pull a page of edges that touch the given node from the server. Merged
   * into the `extraEdges` cache so callers can render connection rows for
   * nodes that aren't in the loaded graph slice.
   */
  loadEdgesForNode: (id: string, opts?: { limit?: number; offset?: number }) => Promise<GraphEdge[]>;

  // Lens navigation
  navigationLevel: NavigationLevel;
  activeLayerId: string | null;

  codeViewerOpen: boolean;
  codeViewerNodeId: string | null;

  tourActive: boolean;
  currentTourStep: number;
  tourHighlightedNodeIds: string[];

  persona: Persona;

  diffMode: boolean;
  changedNodeIds: Set<string>;
  affectedNodeIds: Set<string>;

  // Focus mode: isolate a node's 1-hop neighborhood
  focusNodeId: string | null;

  // Sidebar navigation history (stack of visited node IDs)
  nodeHistory: string[];

  // Filter & Export features
  filters: FilterState;
  filterPanelOpen: boolean;
  exportMenuOpen: boolean;
  pathFinderOpen: boolean;
  reactFlowInstance: ReactFlowInstance | null;

  // Node type category filters
  nodeTypeFilters: Record<NodeCategory, boolean>;
  toggleNodeTypeFilter: (category: NodeCategory) => void;

  setGraph: (graph: KnowledgeGraph) => void;
  selectNode: (nodeId: string | null) => void;
  navigateToNode: (nodeId: string) => void;
  navigateToNodeInLayer: (nodeId: string) => void;
  navigateToHistoryIndex: (index: number) => void;
  goBackNode: () => void;
  drillIntoLayer: (layerId: string) => void;
  navigateToOverview: () => void;
  setFocusNode: (nodeId: string | null) => void;
  setSearchQuery: (query: string) => void;
  setPersona: (persona: Persona) => void;
  openCodeViewer: (nodeId: string) => void;
  closeCodeViewer: () => void;

  setDiffOverlay: (changed: string[], affected: string[]) => void;
  toggleDiffMode: () => void;
  clearDiffOverlay: () => void;

  toggleFilterPanel: () => void;
  toggleExportMenu: () => void;
  togglePathFinder: () => void;
  setReactFlowInstance: (instance: ReactFlowInstance | null) => void;
  setFilters: (filters: Partial<FilterState>) => void;
  resetFilters: () => void;
  hasActiveFilters: () => boolean;

  startTour: () => void;
  stopTour: () => void;
  setTourStep: (step: number) => void;
  nextTourStep: () => void;
  prevTourStep: () => void;

  // View mode
  viewMode: ViewMode;
  isKnowledgeGraph: boolean;
  domainGraph: KnowledgeGraph | null;
  activeDomainId: string | null;

  setDomainGraph: (graph: KnowledgeGraph) => void;
  setViewMode: (mode: ViewMode) => void;
  setIsKnowledgeGraph: (value: boolean) => void;
  navigateToDomain: (domainId: string) => void;
  clearActiveDomain: () => void;
}

function getSortedTour(graph: KnowledgeGraph): TourStep[] {
  const tour = graph.tour ?? [];
  return [...tour].sort((a, b) => a.order - b.order);
}

/** Navigate tour step to the correct layer for the first highlighted node. */
function navigateTourToLayer(
  graph: KnowledgeGraph,
  nodeIds: string[],
): Partial<DashboardStore> {
  if (nodeIds.length === 0) return {};
  const layerId = findNodeLayer(graph, nodeIds[0]);
  if (layerId) {
    return {
      navigationLevel: "layer-detail" as const,
      activeLayerId: layerId,
    };
  }
  return {};
}

// Module-scoped debounce timer for `searchRemote`. Persists across calls.
let _remoteSearchTimer: ReturnType<typeof setTimeout> | null = null;
let _remoteSearchSeq = 0;

// ---------------------------------------------------------------------------
// Live-reload via SSE
// ---------------------------------------------------------------------------

let _eventSource: EventSource | null = null;
let _reloadDebounceTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * Open a persistent SSE connection to `/api/events`. On every
 * `graph-reloaded` event, debounce 300 ms then re-fetch the full graph
 * for the current kind. Auto-reconnects after 2 s on any error.
 *
 * Idempotent — safe to call multiple times; only one connection is kept.
 */
export function subscribeToEvents(): void {
  if (_eventSource) return;
  const es = new EventSource("/api/events");
  _eventSource = es;

  es.addEventListener("graph-reloaded", (e) => {
    console.info("[live] graph-reloaded", (e as MessageEvent).data);
    if (_reloadDebounceTimer) clearTimeout(_reloadDebounceTimer);
    _reloadDebounceTimer = setTimeout(() => {
      const kind = useDashboardStore.getState().graphKind;
      void loadInitialGraph(kind).then((loaded) => {
        if (!loaded) return;
        useDashboardStore.getState().setGraph(loaded.graph);
      });
    }, 300);
  });

  es.onerror = () => {
    console.warn("[live] SSE connection lost; reconnecting in 2 s");
    es.close();
    _eventSource = null;
    setTimeout(subscribeToEvents, 2000);
  };
}

/**
 * Close the SSE connection and cancel any pending reload. Call this on
 * component unmount to prevent memory leaks in tests.
 */
export function unsubscribeFromEvents(): void {
  if (_reloadDebounceTimer) {
    clearTimeout(_reloadDebounceTimer);
    _reloadDebounceTimer = null;
  }
  if (_eventSource) {
    _eventSource.close();
    _eventSource = null;
  }
}

export const useDashboardStore = create<DashboardStore>()((set, get) => ({
  graph: null,
  singleNodes: new Map<string, GraphNode>(),
  extraEdges: new Map<string, GraphEdge[]>(),
  graphKind: readGraphKindFromUrl(),
  browsePanelOpen: false,
  lastNeighborhood: null,
  remoteSearchResults: [],
  remoteSearchLoading: false,
  selectedNodeId: null,
  searchQuery: "",
  searchResults: [],
  searchEngine: null,
  searchMode: "fuzzy",

  setBrowsePanelOpen: (open) => set({ browsePanelOpen: open }),
  toggleBrowsePanel: () => set((s) => ({ browsePanelOpen: !s.browsePanelOpen })),

  setGraphKind: async (kind) => {
    // Update URL `?kind=` without reloading the page.
    if (typeof window !== "undefined") {
      try {
        const url = new URL(window.location.href);
        url.searchParams.set("kind", kind);
        window.history.replaceState({}, "", url.toString());
      } catch {
        /* ignore */
      }
    }
    set({ graphKind: kind });
    try {
      const [graph, layers, tour] = await Promise.all([
        api.fullGraph(kind),
        api.layers(kind).catch(() => [] as Layer[]),
        api.tour(kind).catch(() => [] as TourStep[]),
      ]);
      const result = validateGraph({ ...graph, layers, tour });
      const validated =
        result.success && result.data ? result.data : { ...graph, layers, tour };
      get().setGraph(validated);
      if (kind === "knowledge") {
        set({ viewMode: "knowledge", isKnowledgeGraph: true });
      } else {
        set({ isKnowledgeGraph: false });
      }
    } catch (err) {
      console.error(`[store] setGraphKind(${kind}) failed:`, err);
    }
  },

  searchRemote: (q, type) => {
    if (_remoteSearchTimer) clearTimeout(_remoteSearchTimer);
    const trimmed = q.trim();
    if (trimmed.length < 2) {
      set({ remoteSearchResults: [], remoteSearchLoading: false });
      return;
    }
    set({ remoteSearchLoading: true });
    const seq = ++_remoteSearchSeq;
    _remoteSearchTimer = setTimeout(() => {
      api
        .search(trimmed, type)
        .then((hits) => {
          // Drop stale responses if a newer search already raced ahead.
          if (seq !== _remoteSearchSeq) return;
          const graph = get().graph;
          const enriched: RemoteSearchHit[] = hits.map((h) => {
            const node = graph?.nodes.find((n) => n.id === h.id);
            return node ? { ...h, node } : { ...h };
          });
          set({ remoteSearchResults: enriched, remoteSearchLoading: false });
        })
        .catch((err) => {
          if (seq !== _remoteSearchSeq) return;
          console.warn(`[searchRemote] ${err instanceof Error ? err.message : err}`);
          set({ remoteSearchResults: [], remoteSearchLoading: false });
        });
    }, 200);
  },

  setNeighbors: async (nodeId, depth = 1) => {
    try {
      const resp = await api.neighbors(nodeId, depth);
      set({ lastNeighborhood: resp });
      // Merge any unknown nodes into the singleNodes pocket so callers can
      // resolve names for newly-discovered ids without rebuilding the graph.
      const known = new Set(get().graph?.nodes.map((n) => n.id) ?? []);
      const next = new Map(get().singleNodes);
      for (const n of [resp.center, ...resp.neighbors]) {
        if (!known.has(n.id)) next.set(n.id, n);
      }
      set({ singleNodes: next });
      return resp;
    } catch (err) {
      console.warn(`[setNeighbors] failed for ${nodeId}: ${err instanceof Error ? err.message : err}`);
      return null;
    }
  },

  ensureNode: async (id) => {
    const { graph, singleNodes } = get();
    const existing =
      graph?.nodes.find((n) => n.id === id) ?? singleNodes.get(id) ?? null;
    if (existing) return existing;
    try {
      const node = await api.node(id);
      const next = new Map(get().singleNodes);
      next.set(id, node);
      set({ singleNodes: next });
      return node;
    } catch (err) {
      console.warn(`[ensureNode] ${id} not found: ${err instanceof Error ? err.message : err}`);
      return null;
    }
  },

  loadEdgesForNode: async (id, opts) => {
    const limit = opts?.limit ?? 100;
    const offset = opts?.offset ?? 0;
    const cacheKey = `${id}:${limit}:${offset}`;
    const cached = get().extraEdges.get(cacheKey);
    if (cached) return cached;
    try {
      // Server filters by `source` *or* `target`; we issue both so we
      // pick up incoming as well as outgoing edges.
      const [outgoing, incoming] = await Promise.all([
        api.listEdges({ kind: get().graphKind, source: id, limit, offset }),
        api.listEdges({ kind: get().graphKind, target: id, limit, offset }),
      ]);
      const merged = [...outgoing.items, ...incoming.items];
      const next = new Map(get().extraEdges);
      next.set(cacheKey, merged);
      set({ extraEdges: next });
      return merged;
    } catch (err) {
      console.warn(`[loadEdgesForNode] ${id}: ${err instanceof Error ? err.message : err}`);
      return [];
    }
  },

  navigationLevel: "overview",
  activeLayerId: null,
  codeViewerOpen: false,
  codeViewerNodeId: null,

  tourActive: false,
  currentTourStep: 0,
  tourHighlightedNodeIds: [],

  persona: "junior",

  diffMode: false,
  changedNodeIds: new Set<string>(),
  affectedNodeIds: new Set<string>(),

  focusNodeId: null,
  nodeHistory: [],

  filters: { ...DEFAULT_FILTERS, nodeTypes: new Set(DEFAULT_FILTERS.nodeTypes), complexities: new Set(DEFAULT_FILTERS.complexities), layerIds: new Set(DEFAULT_FILTERS.layerIds), edgeCategories: new Set(DEFAULT_FILTERS.edgeCategories) },
  filterPanelOpen: false,
  exportMenuOpen: false,
  pathFinderOpen: false,
  reactFlowInstance: null,

  nodeTypeFilters: { code: true, config: true, docs: true, infra: true, data: true, domain: true, knowledge: true },

  toggleNodeTypeFilter: (category) =>
    set((state) => ({
      nodeTypeFilters: {
        ...state.nodeTypeFilters,
        [category]: !state.nodeTypeFilters[category],
      },
    })),

  setGraph: (graph) => {
    const searchEngine = new SearchEngine(graph.nodes);
    const query = get().searchQuery;
    const searchResults = query.trim() ? searchEngine.search(query) : [];
    const { viewMode, domainGraph, activeDomainId } = get();
    // Preserve domain view if a domain graph is already loaded
    const keepDomainView = viewMode === "domain" && domainGraph !== null;
    set({
      graph,
      searchEngine,
      searchResults,
      navigationLevel: "overview",
      activeLayerId: null,
      selectedNodeId: null,
      focusNodeId: null,
      nodeHistory: [],
      viewMode: keepDomainView ? "domain" as const : "structural" as const,
      activeDomainId: keepDomainView ? activeDomainId : null,
    });
  },

  selectNode: (nodeId) => {
    const { selectedNodeId, nodeHistory } = get();
    if (nodeId && selectedNodeId && nodeId !== selectedNodeId) {
      // Push current node to history before navigating away
      set({
        selectedNodeId: nodeId,
        nodeHistory: [...nodeHistory, selectedNodeId].slice(-MAX_HISTORY),
      });
    } else {
      set({ selectedNodeId: nodeId });
    }
  },

  navigateToNode: (nodeId) => {
    get().navigateToNodeInLayer(nodeId);
  },

  navigateToNodeInLayer: (nodeId) => {
    const { graph, selectedNodeId, nodeHistory } = get();
    if (!graph) return;
    const layerId = findNodeLayer(graph, nodeId);
    const newHistory =
      selectedNodeId && nodeId !== selectedNodeId
        ? [...nodeHistory, selectedNodeId].slice(-MAX_HISTORY)
        : nodeHistory;
    if (layerId) {
      set({
        navigationLevel: "layer-detail",
        activeLayerId: layerId,
        selectedNodeId: nodeId,
        focusNodeId: null,
        codeViewerOpen: false,
        codeViewerNodeId: null,
        nodeHistory: newHistory,
      });
    } else {
      set({
        selectedNodeId: nodeId,
        nodeHistory: newHistory,
      });
    }
  },

  navigateToHistoryIndex: (index) => {
    const { nodeHistory, graph } = get();
    if (!graph || index < 0 || index >= nodeHistory.length) return;
    const targetId = nodeHistory[index];
    const newHistory = nodeHistory.slice(0, index);
    const layerId = findNodeLayer(graph, targetId);
    set({
      selectedNodeId: targetId,
      nodeHistory: newHistory,
      ...(layerId ? { navigationLevel: "layer-detail" as const, activeLayerId: layerId } : {}),
    });
  },

  goBackNode: () => {
    const { nodeHistory, graph } = get();
    if (nodeHistory.length === 0 || !graph) return;
    const prevNodeId = nodeHistory[nodeHistory.length - 1];
    const newHistory = nodeHistory.slice(0, -1);
    const layerId = findNodeLayer(graph, prevNodeId);
    if (layerId) {
      set({
        navigationLevel: "layer-detail",
        activeLayerId: layerId,
        selectedNodeId: prevNodeId,
        nodeHistory: newHistory,
      });
    } else {
      set({
        selectedNodeId: prevNodeId,
        nodeHistory: newHistory,
      });
    }
  },

  drillIntoLayer: (layerId) =>
    set({
      navigationLevel: "layer-detail",
      activeLayerId: layerId,
      selectedNodeId: null,
      focusNodeId: null,
      codeViewerOpen: false,
      codeViewerNodeId: null,
    }),

  navigateToOverview: () =>
    set({
      navigationLevel: "overview",
      activeLayerId: null,
      selectedNodeId: null,
      focusNodeId: null,
      codeViewerOpen: false,
      codeViewerNodeId: null,
    }),

  setFocusNode: (nodeId) => set({ focusNodeId: nodeId, selectedNodeId: nodeId }),
  setSearchMode: (mode) => set({ searchMode: mode }),
  setSearchQuery: (query) => {
    const engine = get().searchEngine;
    const mode = get().searchMode;
    if (!engine || !query.trim()) {
      set({ searchQuery: query, searchResults: [], remoteSearchResults: [] });
      return;
    }
    // Currently both modes use the same fuzzy engine
    // When embeddings are available, "semantic" mode will use SemanticSearchEngine
    void mode;
    const searchResults = engine.search(query);
    set({ searchQuery: query, searchResults });
    // Hybrid: kick off the server-side index for richer matches once the
    // query is at least 2 chars. Below that we keep things purely local.
    get().searchRemote(query);
  },

  setPersona: (persona) => set({ persona }),

  openCodeViewer: (nodeId) => set({ codeViewerOpen: true, codeViewerNodeId: nodeId }),
  closeCodeViewer: () => set({ codeViewerOpen: false, codeViewerNodeId: null }),

  setDiffOverlay: (changed, affected) =>
    set({
      diffMode: true,
      changedNodeIds: new Set(changed),
      affectedNodeIds: new Set(affected),
    }),

  toggleDiffMode: () => set((state) => ({ diffMode: !state.diffMode })),

  clearDiffOverlay: () =>
    set({
      diffMode: false,
      changedNodeIds: new Set<string>(),
      affectedNodeIds: new Set<string>(),
    }),

  toggleFilterPanel: () => set((state) => ({
    filterPanelOpen: !state.filterPanelOpen,
    exportMenuOpen: false,
  })),

  toggleExportMenu: () => set((state) => ({
    exportMenuOpen: !state.exportMenuOpen,
    filterPanelOpen: false,
  })),

  togglePathFinder: () => set((state) => ({
    pathFinderOpen: !state.pathFinderOpen,
  })),

  setReactFlowInstance: (instance) => set({ reactFlowInstance: instance }),

  setFilters: (newFilters) => set((state) => ({
    filters: { ...state.filters, ...newFilters },
  })),

  resetFilters: () => set({
    filters: {
      nodeTypes: new Set<NodeType>(ALL_NODE_TYPES),
      complexities: new Set<Complexity>(ALL_COMPLEXITIES),
      layerIds: new Set<string>(),
      edgeCategories: new Set<EdgeCategory>(ALL_EDGE_CATEGORIES),
    },
  }),

  hasActiveFilters: () => {
    const { filters } = get();
    return filters.nodeTypes.size !== ALL_NODE_TYPES.length
      || filters.complexities.size !== ALL_COMPLEXITIES.length
      || filters.layerIds.size > 0
      || filters.edgeCategories.size !== ALL_EDGE_CATEGORIES.length;
  },

  startTour: () => {
    const { graph } = get();
    if (!graph || !graph.tour || graph.tour.length === 0) return;
    const sorted = getSortedTour(graph);
    const layerNav = navigateTourToLayer(graph, sorted[0].nodeIds);
    set({
      tourActive: true,
      currentTourStep: 0,
      tourHighlightedNodeIds: sorted[0].nodeIds,
      selectedNodeId: null,
      ...layerNav,
    });
  },

  stopTour: () =>
    set({
      tourActive: false,
      currentTourStep: 0,
      tourHighlightedNodeIds: [],
    }),

  setTourStep: (step) => {
    const { graph } = get();
    if (!graph || !graph.tour || graph.tour.length === 0) return;
    const sorted = getSortedTour(graph);
    if (step < 0 || step >= sorted.length) return;
    const layerNav = navigateTourToLayer(graph, sorted[step].nodeIds);
    set({
      currentTourStep: step,
      tourHighlightedNodeIds: sorted[step].nodeIds,
      ...layerNav,
    });
  },

  nextTourStep: () => {
    const { graph, currentTourStep } = get();
    if (!graph || !graph.tour || graph.tour.length === 0) return;
    const sorted = getSortedTour(graph);
    if (currentTourStep < sorted.length - 1) {
      const next = currentTourStep + 1;
      const layerNav = navigateTourToLayer(graph, sorted[next].nodeIds);
      set({
        currentTourStep: next,
        tourHighlightedNodeIds: sorted[next].nodeIds,
        ...layerNav,
      });
    }
  },

  prevTourStep: () => {
    const { graph, currentTourStep } = get();
    if (!graph || !graph.tour || graph.tour.length === 0) return;
    if (currentTourStep > 0) {
      const sorted = getSortedTour(graph);
      const prev = currentTourStep - 1;
      const layerNav = navigateTourToLayer(graph, sorted[prev].nodeIds);
      set({
        currentTourStep: prev,
        tourHighlightedNodeIds: sorted[prev].nodeIds,
        ...layerNav,
      });
    }
  },

  viewMode: "structural",
  isKnowledgeGraph: false,
  domainGraph: null,
  activeDomainId: null,

  setDomainGraph: (graph) => {
    set({ domainGraph: graph });
  },

  setIsKnowledgeGraph: (value) => {
    set({ isKnowledgeGraph: value });
  },

  setViewMode: (mode) => {
    set({
      viewMode: mode,
      selectedNodeId: null,
      focusNodeId: null,
      codeViewerOpen: false,
      codeViewerNodeId: null,
    });
  },

  navigateToDomain: (domainId) => {
    const { selectedNodeId, nodeHistory } = get();
    const newHistory = selectedNodeId
      ? [...nodeHistory, selectedNodeId].slice(-MAX_HISTORY)
      : nodeHistory;
    set({
      viewMode: "domain" as const,
      activeDomainId: domainId,
      focusNodeId: null,
      nodeHistory: newHistory,
    });
  },

  clearActiveDomain: () => {
    set({
      activeDomainId: null,
      selectedNodeId: null,
      focusNodeId: null,
    });
  },
}));
