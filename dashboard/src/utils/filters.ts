import type { GraphNode, GraphEdge, Layer } from "../core/types";
import type { FilterState, NodeType, Complexity, EdgeCategory } from "../store";
import { EDGE_CATEGORY_MAP } from "../store";

/**
 * Filter nodes based on active filters
 */
export function filterNodes(
  nodes: GraphNode[],
  layers: Layer[],
  filters: FilterState,
): GraphNode[] {
  return nodes.filter((node) => {
    // Filter by node type
    if (!filters.nodeTypes.has(node.type as NodeType)) {
      return false;
    }

    // Filter by complexity
    if (node.complexity && !filters.complexities.has(node.complexity as Complexity)) {
      return false;
    }

    // Filter by layer (if any layers are selected)
    if (filters.layerIds.size > 0) {
      const nodeInSelectedLayer = layers.some(
        (layer) => filters.layerIds.has(layer.id) && layer.nodeIds.includes(node.id)
      );
      if (!nodeInSelectedLayer) {
        return false;
      }
    }

    return true;
  });
}

/**
 * Filter edges based on visible nodes and active edge category filters
 */
export function filterEdges(
  edges: GraphEdge[],
  visibleNodeIds: Set<string>,
  filters: FilterState,
): GraphEdge[] {
  return edges.filter((edge) => {
    // Only keep edges between visible nodes
    if (!visibleNodeIds.has(edge.source) || !visibleNodeIds.has(edge.target)) {
      return false;
    }

    // Filter by edge category
    const edgeCategory = getEdgeCategory(edge.type);
    if (edgeCategory && !filters.edgeCategories.has(edgeCategory)) {
      return false;
    }

    return true;
  });
}

/**
 * Determine which category an edge type belongs to
 */
function getEdgeCategory(edgeType: string): EdgeCategory | null {
  for (const [category, types] of Object.entries(EDGE_CATEGORY_MAP)) {
    if (types.includes(edgeType)) {
      return category as EdgeCategory;
    }
  }
  return null;
}
