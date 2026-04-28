import { useEffect, useState, useMemo, lazy, Suspense } from "react";
import { validateGraph } from "./core/schema";
import type { GraphIssue } from "./core/schema";
import type { ProjectMeta } from "./core/types";
import {
  api,
  loadInitialGraph,
  readGraphKindFromUrl,
  subscribeToEvents,
  unsubscribeFromEvents,
  useDashboardStore,
} from "./store";
import GraphView from "./components/GraphView";
import DomainGraphView from "./components/DomainGraphView";
import KnowledgeGraphView from "./components/KnowledgeGraphView";
import SearchBar from "./components/SearchBar";
import NodeInfo from "./components/NodeInfo";
import LayerLegend from "./components/LayerLegend";
import DiffToggle from "./components/DiffToggle";
import FilterPanel from "./components/FilterPanel";
import ExportMenu from "./components/ExportMenu";
import PersonaSelector from "./components/PersonaSelector";
import ProjectOverview from "./components/ProjectOverview";
import WarningBanner from "./components/WarningBanner";
import TokenGate from "./components/TokenGate";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import type { KeyboardShortcut } from "./hooks/useKeyboardShortcuts";
import { ThemeProvider } from "./themes/index.ts";
import { ThemePicker } from "./components/ThemePicker.tsx";
import type { ThemeConfig } from "./themes/index.ts";
import { KindSwitcher } from "./components/KindSwitcher";

// Lazy-load heavy / optional components so they ship in separate chunks.
const CodeViewer = lazy(() => import("./components/CodeViewer"));
const LearnPanel = lazy(() => import("./components/LearnPanel"));
const PathFinderModal = lazy(() => import("./components/PathFinderModal"));
const KeyboardShortcutsHelp = lazy(
  () => import("./components/KeyboardShortcutsHelp"),
);
const BrowsePanel = lazy(() => import("./components/BrowsePanel"));

// The Rust `understandable` server replaces the original Vite/Node host:
// we talk to its REST API (`/api/*`) when it's reachable, and fall back
// to the bundled `/knowledge-graph.json` blob otherwise. The graph
// kind is read from the URL (`?kind=codebase|domain|knowledge`) so the
// user can switch graphs without rebuilding the dashboard.

function App() {
  // `TokenGate` wraps the dashboard so that, once chat / explain UIs
  // land in this Rust port, they can opt into the gate without any
  // top-level surgery. Right now `required={false}` means it renders
  // children unconditionally.
  // TODO: flip `required` to `true` once chat / explain components are
  // mounted somewhere that needs an Anthropic API key.
  return (
    <TokenGate>
      <Dashboard />
    </TokenGate>
  );
}

function Dashboard() {
  const graph = useDashboardStore((s) => s.graph);
  const setGraph = useDashboardStore((s) => s.setGraph);
  const selectedNodeId = useDashboardStore((s) => s.selectedNodeId);
  const tourActive = useDashboardStore((s) => s.tourActive);
  const persona = useDashboardStore((s) => s.persona);
  const codeViewerOpen = useDashboardStore((s) => s.codeViewerOpen);
  const closeCodeViewer = useDashboardStore((s) => s.closeCodeViewer);
  const setDiffOverlay = useDashboardStore((s) => s.setDiffOverlay);
  const pathFinderOpen = useDashboardStore((s) => s.pathFinderOpen);
  const togglePathFinder = useDashboardStore((s) => s.togglePathFinder);
  const nodeTypeFilters = useDashboardStore((s) => s.nodeTypeFilters);
  const toggleNodeTypeFilter = useDashboardStore((s) => s.toggleNodeTypeFilter);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [graphIssues, setGraphIssues] = useState<GraphIssue[]>([]);
  const [showKeyboardHelp, setShowKeyboardHelp] = useState(false);
  const [metaTheme, setMetaTheme] = useState<ThemeConfig | null>(null);
  const viewMode = useDashboardStore((s) => s.viewMode);
  const setViewMode = useDashboardStore((s) => s.setViewMode);
  const isKnowledgeGraph = useDashboardStore((s) => s.isKnowledgeGraph);
  const domainGraph = useDashboardStore((s) => s.domainGraph);
  const setDomainGraph = useDashboardStore((s) => s.setDomainGraph);
  const graphKind = useDashboardStore((s) => s.graphKind);
  const setGraphKind = useDashboardStore((s) => s.setGraphKind);
  const browsePanelOpen = useDashboardStore((s) => s.browsePanelOpen);
  const toggleBrowsePanel = useDashboardStore((s) => s.toggleBrowsePanel);
  const ensureNode = useDashboardStore((s) => s.ensureNode);
  const selectNode = useDashboardStore((s) => s.selectNode);
  const loadEdgesForNode = useDashboardStore((s) => s.loadEdgesForNode);

  useEffect(() => {
    api
      .project()
      .then((meta) => {
        // The Rust `ProjectMeta` doesn't carry a theme today, but the
        // shape stays open-ended so future server builds can add one.
        const themed = meta as ProjectMeta & { theme?: ThemeConfig };
        if (themed.theme) setMetaTheme(themed.theme);
      })
      .catch(() => {
        /* ignore — project meta is optional for theming */
      });
  }, []);

  // Live-reload: open an SSE connection so graph changes made by
  // `understandable analyze` in another terminal are reflected without
  // restarting the server.
  useEffect(() => {
    subscribeToEvents();
    return () => {
      unsubscribeFromEvents();
    };
  }, []);

  // Define keyboard shortcuts
  const shortcuts = useMemo<KeyboardShortcut[]>(
    () => [
      // Help
      {
        key: "?",
        shiftKey: true,
        description: "Show keyboard shortcuts",
        action: () => setShowKeyboardHelp((prev) => !prev),
        category: "General",
      },
      // Navigation
      {
        key: "Escape",
        description: "Close panels and modals / go back to overview",
        action: () => {
          // Read from store at invocation time to avoid stale closures
          const state = useDashboardStore.getState();
          if (state.pathFinderOpen) {
            state.togglePathFinder();
          } else if (state.filterPanelOpen) {
            state.toggleFilterPanel();
          } else if (state.exportMenuOpen) {
            state.toggleExportMenu();
          } else if (state.codeViewerOpen) {
            state.closeCodeViewer();
          } else if (state.selectedNodeId) {
            state.selectNode(null);
          } else if (state.navigationLevel === "layer-detail") {
            state.navigateToOverview();
          } else if (state.tourActive) {
            state.stopTour();
          } else {
            setShowKeyboardHelp(false);
          }
        },
        category: "Navigation",
      },
      {
        key: "/",
        description: "Focus search bar",
        action: () => {
          const searchInput = document.querySelector<HTMLInputElement>(
            'input[placeholder*="Search"]'
          );
          searchInput?.focus();
        },
        category: "Navigation",
      },
      // Tour controls
      {
        key: "ArrowRight",
        description: "Next tour step",
        action: () => {
          const state = useDashboardStore.getState();
          if (state.tourActive) {
            state.nextTourStep();
          }
        },
        category: "Tour",
      },
      {
        key: "ArrowLeft",
        description: "Previous tour step",
        action: () => {
          const state = useDashboardStore.getState();
          if (state.tourActive) {
            state.prevTourStep();
          }
        },
        category: "Tour",
      },
      // View toggles
      {
        key: "d",
        description: "Toggle diff mode",
        action: () => {
          const state = useDashboardStore.getState();
          state.toggleDiffMode();
        },
        category: "View",
      },
      {
        key: "f",
        description: "Toggle filter panel",
        action: () => {
          const state = useDashboardStore.getState();
          state.toggleFilterPanel();
        },
        category: "View",
      },
      {
        key: "e",
        description: "Toggle export menu",
        action: () => {
          const state = useDashboardStore.getState();
          state.toggleExportMenu();
        },
        category: "View",
      },
      {
        key: "p",
        description: "Open path finder",
        action: () => {
          const state = useDashboardStore.getState();
          state.togglePathFinder();
        },
        category: "View",
      },
      {
        key: "b",
        description: "Open node browser",
        action: () => {
          const state = useDashboardStore.getState();
          state.toggleBrowsePanel();
        },
        category: "View",
      },
      {
        key: "n",
        description: "Expand neighbours of selected node",
        action: () => {
          const state = useDashboardStore.getState();
          if (state.selectedNodeId) {
            void state.setNeighbors(state.selectedNodeId, 1);
          }
        },
        category: "Navigation",
      },
    ],
    []
  );

  // Register keyboard shortcuts
  useKeyboardShortcuts(shortcuts);

  useEffect(() => {
    let cancelled = false;
    const kind = readGraphKindFromUrl();
    loadInitialGraph(kind)
      .then((loaded) => {
        if (cancelled) return;
        if (!loaded) {
          setLoadError(
            "Failed to load knowledge graph from /api/graph and the static fallback. Run `understandable analyze` first or check the dev console.",
          );
          return;
        }
        const { graph } = loaded;
        // Re-run validation here so we can capture issue notes even when
        // the loader already validated. Cheap and gives us a structured
        // issue list for the warning banner.
        const result = validateGraph(graph);
        if (result.success && result.data) {
          setGraph(result.data);
          setGraphIssues(result.issues);
          if (graph.kind === "knowledge" || kind === "knowledge") {
            setViewMode("knowledge");
            useDashboardStore.getState().setIsKnowledgeGraph(true);
          }
          for (const issue of result.issues) {
            if (issue.level === "auto-corrected") {
              console.warn(`[graph] auto-corrected: ${issue.message}`);
            } else if (issue.level === "dropped") {
              console.error(`[graph] dropped: ${issue.message}`);
            }
          }
        } else if (result.fatal) {
          console.error("Knowledge graph validation failed:", result.fatal);
          setLoadError(`Invalid knowledge graph: ${result.fatal}`);
        } else {
          console.error("Knowledge graph validation failed: unknown error");
          setLoadError("Invalid knowledge graph: unknown validation error");
        }
      })
      .catch((err) => {
        if (cancelled) return;
        console.error("Failed to load knowledge graph:", err);
        setLoadError(
          `Failed to load knowledge graph: ${err instanceof Error ? err.message : String(err)}`,
        );
      });
    return () => {
      cancelled = true;
    };
  }, [setGraph, setViewMode]);

  useEffect(() => {
    api
      .diff()
      .then((d) => {
        if (d && d.changedNodeIds.length > 0) {
          setDiffOverlay(d.changedNodeIds, d.affectedNodeIds);
        }
      })
      .catch(() => {
        // Silently ignore — diff overlay is optional. Server returns
        // 204 when no overlay is on disk.
      });
  }, [setDiffOverlay]);

  useEffect(() => {
    api
      .fullGraph("domain")
      .then((data) => {
        const result = validateGraph(data);
        if (result.success && result.data) {
          setDomainGraph(result.data);
        } else if (result.fatal) {
          console.warn(`[domain-graph] validation failed: ${result.fatal}`);
        }
      })
      .catch(() => {
        // Silently ignore — domain graph is optional and 404s when the
        // user hasn't run `understandable domain` yet.
      });
  }, [setDomainGraph]);

  // URL-driven single-node fallback: `?node=<id>` selects that node, and
  // if it's not in the loaded graph (e.g. came from a permalink to a
  // node that lives only on the server) we hydrate it via `/api/node`.
  useEffect(() => {
    if (!graph) return;
    let cancelled = false;
    try {
      const id = new URL(window.location.href).searchParams.get("node");
      if (!id) return;
      const known = graph.nodes.find((n) => n.id === id);
      if (known) {
        selectNode(id);
        return;
      }
      void ensureNode(id).then((n) => {
        if (cancelled || !n) return;
        // We deliberately don't navigate into a layer here — the node
        // isn't part of the active graph, so we just surface its id.
        selectNode(n.id);
        // Warm the edge cache so the connections list can render even
        // though the node isn't part of the loaded graph slice.
        void loadEdgesForNode(n.id);
      });
    } catch {
      /* ignore malformed URLs */
    }
    return () => {
      cancelled = true;
    };
  }, [graph, ensureNode, selectNode]);

  // Determine sidebar content
  // NodeInfo always takes priority when a node is selected.
  // Learn mode adds LearnPanel below it; otherwise ProjectOverview shows when idle.
  const isLearnMode = tourActive || persona === "junior";
  const sidebarContent = (
    <>
      {selectedNodeId && <NodeInfo />}
      {isLearnMode && (
        <Suspense fallback={null}>
          <LearnPanel />
        </Suspense>
      )}
      {!selectedNodeId && !isLearnMode && <ProjectOverview />}
    </>
  );

  return (
    <ThemeProvider metaTheme={metaTheme}>
    <div className="h-screen w-screen flex flex-col bg-root text-text-primary noise-overlay">
      {/* Header */}
      <header className="flex items-center px-5 py-3 bg-surface border-b border-border-subtle shrink-0 gap-4">
        {/* Left — fixed */}
        <div className="flex items-center gap-5 shrink-0">
          <h1 className="font-serif text-lg text-text-primary tracking-wide">
            {graph?.project.name ?? "understandable"}
          </h1>
          <div className="w-px h-5 bg-border-subtle" />
          <PersonaSelector />
          <div className="w-px h-5 bg-border-subtle" />
          <KindSwitcher kind={graphKind} onChange={(k) => void setGraphKind(k)} />
          {graph && !isKnowledgeGraph && domainGraph && (
            <>
              <div className="w-px h-5 bg-border-subtle" />
              <div className="flex items-center bg-elevated rounded-lg p-0.5">
                <button
                  type="button"
                  onClick={() => setViewMode("domain")}
                  title="Switch to domain view"
                  className={`px-3 py-1 text-xs font-medium rounded-md transition-colors ${
                    viewMode === "domain"
                      ? "bg-accent/20 text-accent"
                      : "text-text-muted hover:text-text-secondary"
                  }`}
                >
                  Domain
                </button>
                <button
                  type="button"
                  onClick={() => setViewMode("structural")}
                  title="Switch to structural view"
                  className={`px-3 py-1 text-xs font-medium rounded-md transition-colors ${
                    viewMode === "structural"
                      ? "bg-accent/20 text-accent"
                      : "text-text-muted hover:text-text-secondary"
                  }`}
                >
                  Structural
                </button>
              </div>
            </>
          )}
        </div>

        {/* Middle — scrollable legends */}
        <div className="flex-1 min-w-0 overflow-x-auto scrollbar-hide">
          <div className="flex items-center gap-4 w-max">
            <DiffToggle />
            <div className="flex items-center gap-1">
              {(isKnowledgeGraph ? [
                { key: "knowledge" as const, label: "All", color: "var(--color-node-article)" },
              ] : [
                { key: "code" as const, label: "Code", color: "var(--color-node-file)" },
                { key: "config" as const, label: "Config", color: "var(--color-node-config)" },
                { key: "docs" as const, label: "Docs", color: "var(--color-node-document)" },
                { key: "infra" as const, label: "Infra", color: "var(--color-node-service)" },
                { key: "data" as const, label: "Data", color: "var(--color-node-table)" },
                { key: "domain" as const, label: "Domain", color: "var(--color-node-concept)" },
                { key: "knowledge" as const, label: "Knowledge", color: "var(--color-node-article)" },
              ]).map((cat) => (
                <button
                  key={cat.key}
                  onClick={() => toggleNodeTypeFilter(cat.key)}
                  className={`text-[10px] font-semibold uppercase tracking-wider px-2 py-1 rounded border transition-colors flex items-center gap-1.5 whitespace-nowrap ${
                    nodeTypeFilters[cat.key] !== false
                      ? "border-border-medium bg-elevated text-text-secondary hover:text-text-primary"
                      : "border-transparent bg-transparent text-text-muted/40 line-through hover:text-text-muted"
                  }`}
                  title={`${nodeTypeFilters[cat.key] !== false ? "Hide" : "Show"} ${cat.label} nodes`}
                >
                  <span
                    className="w-2 h-2 rounded-full shrink-0"
                    style={{
                      backgroundColor: cat.color,
                      opacity: nodeTypeFilters[cat.key] !== false ? 1 : 0.3,
                    }}
                  />
                  {cat.label}
                </button>
              ))}
            </div>
            <LayerLegend />
          </div>
        </div>

        {/* Right — fixed actions */}
        <div className="flex items-center gap-4 shrink-0">
          <FilterPanel />
          <ExportMenu />
          <button
            onClick={toggleBrowsePanel}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm bg-elevated text-text-secondary hover:text-text-primary transition-colors"
            title="Browse all nodes (B)"
          >
            <svg
              className="w-4 h-4"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M4 6h16M4 12h16M4 18h16"
              />
            </svg>
            Browse
          </button>
          <button
            onClick={togglePathFinder}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm bg-elevated text-text-secondary hover:text-text-primary transition-colors"
            title="Find path between nodes (P)"
          >
            <svg
              className="w-4 h-4"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M13 7h8m0 0v8m0-8l-8 8-4-4-6 6"
              />
            </svg>
            Path
          </button>
          <ThemePicker />
          <button
            onClick={() => setShowKeyboardHelp(true)}
            className="text-text-muted hover:text-accent transition-colors"
            title="Keyboard shortcuts (Shift + ?)"
          >
            <svg
              className="w-5 h-5"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M8.228 9c.549-1.165 2.03-2 3.772-2 2.21 0 4 1.343 4 3 0 1.4-1.278 2.575-3.006 2.907-.542.104-.994.54-.994 1.093m0 3h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
              />
            </svg>
          </button>
        </div>
      </header>

      {/* Search */}
      <SearchBar />

      {/* Validation warning banner */}
      {graphIssues.length > 0 && !loadError && (
        <WarningBanner issues={graphIssues} />
      )}

      {/* Error banner */}
      {loadError && (
        <div className="px-5 py-3 bg-red-900/30 border-b border-red-700 text-red-200 text-sm">
          {loadError}
        </div>
      )}

      {/* Main content: Graph + Sidebar */}
      <div className="flex-1 flex min-h-0 relative">
        {/* Graph area */}
        <div className="flex-1 min-w-0 min-h-0 relative">
          {viewMode === "knowledge" ? (
            <KnowledgeGraphView />
          ) : viewMode === "domain" && domainGraph ? (
            <DomainGraphView />
          ) : (
            <GraphView />
          )}
          <div className="absolute top-3 right-3 text-sm text-text-muted/60 pointer-events-none select-none">
            Press <kbd className="kbd">?</kbd> for keyboard shortcuts
          </div>
        </div>

        {/* Right sidebar */}
        <aside className="w-[360px] shrink-0 bg-surface border-l border-border-subtle overflow-auto">
          {sidebarContent}
        </aside>

        {/* Code viewer overlay */}
        {codeViewerOpen && (
          <div className="absolute bottom-0 left-0 right-0 h-[25vh] bg-surface border-t border-border-subtle animate-slide-up z-20">
            <div className="h-full flex flex-col">
              <div className="flex items-center justify-end px-3 py-1 shrink-0">
                <button
                  onClick={closeCodeViewer}
                  className="text-text-muted hover:text-text-primary text-xs transition-colors"
                >
                  Close
                </button>
              </div>
              <div className="flex-1 min-h-0">
                <Suspense fallback={null}>
                  <CodeViewer />
                </Suspense>
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Keyboard shortcuts help modal */}
      {showKeyboardHelp && (
        <Suspense fallback={null}>
          <KeyboardShortcutsHelp
            shortcuts={shortcuts}
            onClose={() => setShowKeyboardHelp(false)}
          />
        </Suspense>
      )}

      {/* Path Finder Modal — only mounted when open so its chunk is lazy-loaded on demand. */}
      {pathFinderOpen && (
        <Suspense fallback={null}>
          <PathFinderModal isOpen={pathFinderOpen} onClose={togglePathFinder} />
        </Suspense>
      )}

      {/* Browse Panel — mounted only when open so its chunk loads on demand. */}
      {browsePanelOpen && (
        <Suspense fallback={null}>
          <BrowsePanel />
        </Suspense>
      )}
    </div>
    </ThemeProvider>
  );
}

export default App;
