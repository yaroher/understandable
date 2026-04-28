//! Graph construction, layer detection, tour generation, normalization,
//! domain extraction, and Karpathy-wiki ingest.
//!
//! Ports the original `analyzer/` TypeScript modules + the deterministic
//! halves of the `domain-analyzer` and `article-analyzer` agents. LLM
//! reasoning lives in `ua-llm`; everything in this crate is pure.

pub mod change_classifier;
pub mod domain;
pub mod graph_builder;
pub mod knowledge;
pub mod language_lesson;
pub mod layer_detector;
pub mod normalize;
pub mod tour_generator;

pub use change_classifier::{classify_change, classify_change_with, ChangeLevel};
pub use domain::build_domain_graph;
pub use graph_builder::{
    FileMeta, FileWithAnalysisMeta, GraphBuilder, NonCodeFileAnalysisMeta, NonCodeFileMeta,
};
pub use knowledge::{build_knowledge_graph, parse_wiki, ParsedArticle};
pub use language_lesson::{language_lesson_for, LanguageLesson};
pub use layer_detector::{
    apply_llm_layers, build_layer_detection_prompt, detect_layers,
    parse_layer_detection_response, LlmLayerResponse,
};
pub use normalize::{
    normalize_batch_output, normalize_complexity, normalize_node_id, DroppedEdge,
    DropReason, NormalizationStats, NormalizeBatchResult, RawNode, RawEdge,
};
pub use tour_generator::generate_heuristic_tour;
