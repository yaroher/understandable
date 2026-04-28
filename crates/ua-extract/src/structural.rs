//! Deterministic structural fingerprint of a parsed file.
//!
//! Layered on top of [`StructuralAnalysis`] (the language-agnostic shape
//! every analyzer plugin emits) so the same canonicalisation works for
//! tree-sitter languages and the line-oriented `parsers/` family alike.
//!
//! ## Why a second hash?
//!
//! The byte-level blake3 in [`ua_persist::Fingerprint`] flips on every
//! whitespace edit, comment tweak and statement-level change. The
//! change classifier needs to tell those apart from real signature
//! edits — adding a parameter, renaming a function, exporting a symbol —
//! so it knows when to re-run the LLM and when to skip.
//!
//! Mirrors the canonical signature format from the upstream TS port at
//! `Understand-Anything/understand-anything-plugin/packages/core/src/fingerprint.ts:75-294`
//! with one tweak: the Rust `FunctionDecl` doesn't carry `is_async` or
//! `is_exported` flags, so we substitute "function name appears in the
//! sorted exports list" via the surrounding [`StructuralAnalysis`].
//!
//! ## Determinism contract
//!
//! Two analyses with the same structural elements (regardless of order
//! in the source file or whitespace between them) produce the same hash.
//! Any change that flips a top-level signature — new function, renamed
//! class, added parameter, removed export, new import source — flips
//! the hash. Reordering top-level declarations does *not* flip it
//! (everything sorted before hashing).

use ua_core::StructuralAnalysis;

/// Build a deterministic hash of the structural shape of a file.
///
/// The hash captures, in order, the following sorted lists separated
/// by tagged section markers:
///
///   * `FUNCS` — `name:arity:is_exported` for every top-level function
///     (sorted by name, then by arity to keep overloads stable);
///   * `CLASSES` — `name:method1,method2,...` for every class (methods
///     within a class are sorted before joining);
///   * `IMPORTS` — deduped sorted import sources (the specifier list
///     often shifts on cosmetic edits like reformatting `import { A, B }`
///     to multi-line form, so it's deliberately excluded);
///   * `EXPORTS` — deduped sorted export names;
///   * `SECTIONS` — sorted section names (Markdown, INI, etc.);
///   * `DEFINITIONS` — `kind:name` for parser-emitted definitions
///     (SQL tables, GraphQL types, protobuf messages, …).
///
/// Two files with the same structural hash differ only in:
///   * whitespace, comments, formatting,
///   * function body contents (statement-level edits),
///   * variable names inside functions,
///   * import-specifier lists (the source path still has to match).
///
/// A signature change (param added, function renamed, new export) flips
/// the hash. Reordering top-level declarations does NOT flip it (every
/// list is sorted before hashing).
pub fn structural_hash(analysis: &StructuralAnalysis) -> String {
    let mut hasher = blake3::Hasher::new();

    // Build the exported-name set up front so each function entry can
    // record whether it's part of the public surface — that's the
    // closest stand-in we have for the TS `is_exported` flag.
    let exported_names: std::collections::HashSet<&str> =
        analysis.exports.iter().map(|e| e.name.as_str()).collect();

    // Functions: `name:arity:is_exported`. Sort by `(name, arity)` so
    // overloads (rare in Rust but common in TS/JS/PHP) stay stable.
    hasher.update(b"FUNCS\n");
    let mut funcs: Vec<(&str, usize, bool)> = analysis
        .functions
        .iter()
        .map(|f| {
            let is_exported = exported_names.contains(f.name.as_str());
            (f.name.as_str(), f.params.len(), is_exported)
        })
        .collect();
    funcs.sort_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(&b.1)));
    for (name, arity, is_exported) in funcs {
        hasher.update(name.as_bytes());
        hasher.update(b":");
        hasher.update(&(arity as u32).to_le_bytes());
        hasher.update(b":");
        hasher.update(&[is_exported as u8]);
        hasher.update(b"\n");
    }

    // Classes: `name:method1,method2,…`. Sort classes by name and then
    // sort each class's method list — the textual order of methods in
    // the source is irrelevant to the structural shape.
    hasher.update(b"CLASSES\n");
    let mut classes: Vec<(&str, Vec<&str>)> = analysis
        .classes
        .iter()
        .map(|c| {
            let mut methods: Vec<&str> = c.methods.iter().map(|s| s.as_str()).collect();
            methods.sort();
            (c.name.as_str(), methods)
        })
        .collect();
    classes.sort_by(|a, b| a.0.cmp(b.0));
    for (name, methods) in classes {
        hasher.update(name.as_bytes());
        hasher.update(b":");
        hasher.update(methods.join(",").as_bytes());
        hasher.update(b"\n");
    }

    // Imports: source path only. Specifier lists shift on cosmetic
    // reflows ( `{ A, B }` → `{\n  A,\n  B,\n}` ) so they're excluded;
    // the source string is the part that changes the dependency graph.
    hasher.update(b"IMPORTS\n");
    let mut imports: Vec<&str> = analysis.imports.iter().map(|i| i.source.as_str()).collect();
    imports.sort();
    imports.dedup();
    for src in imports {
        hasher.update(src.as_bytes());
        hasher.update(b"\n");
    }

    // Exports: name only — the line number is positional and would
    // flip on any vertical edit.
    hasher.update(b"EXPORTS\n");
    let mut exports: Vec<&str> = analysis.exports.iter().map(|e| e.name.as_str()).collect();
    exports.sort();
    exports.dedup();
    for name in exports {
        hasher.update(name.as_bytes());
        hasher.update(b"\n");
    }

    // Sections: Markdown headers, INI sections, etc. The line range is
    // dropped — only the named outline matters for "did the structure
    // change".
    if let Some(secs) = analysis.sections.as_ref() {
        hasher.update(b"SECTIONS\n");
        let mut names: Vec<(&str, u32)> = secs.iter().map(|s| (s.name.as_str(), s.level)).collect();
        names.sort();
        for (name, level) in names {
            hasher.update(name.as_bytes());
            hasher.update(b":");
            hasher.update(&level.to_le_bytes());
            hasher.update(b"\n");
        }
    }

    // Definitions: SQL tables, GraphQL types, protobuf messages, …
    // The `kind` qualifier prevents `table:Foo` colliding with
    // `view:Foo`. Fields are joined sorted so column reorders inside
    // a table count as cosmetic.
    if let Some(defs) = analysis.definitions.as_ref() {
        hasher.update(b"DEFINITIONS\n");
        let mut rows: Vec<(&str, &str, Vec<&str>)> = defs
            .iter()
            .map(|d| {
                let mut fields: Vec<&str> = d.fields.iter().map(|s| s.as_str()).collect();
                fields.sort();
                (d.kind.as_str(), d.name.as_str(), fields)
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(b.1)));
        for (kind, name, fields) in rows {
            hasher.update(kind.as_bytes());
            hasher.update(b":");
            hasher.update(name.as_bytes());
            hasher.update(b":");
            hasher.update(fields.join(",").as_bytes());
            hasher.update(b"\n");
        }
    }

    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ua_core::{ClassDecl, ExportDecl, FunctionDecl, ImportDecl, SectionInfo};

    fn fn_decl(name: &str, params: &[&str]) -> FunctionDecl {
        FunctionDecl {
            name: name.to_string(),
            line_range: (0, 0),
            params: params.iter().map(|s| s.to_string()).collect(),
            return_type: None,
        }
    }

    fn class_decl(name: &str, methods: &[&str]) -> ClassDecl {
        ClassDecl {
            name: name.to_string(),
            line_range: (0, 0),
            methods: methods.iter().map(|s| s.to_string()).collect(),
            properties: Vec::new(),
        }
    }

    fn import_decl(source: &str) -> ImportDecl {
        ImportDecl {
            source: source.to_string(),
            specifiers: Vec::new(),
            line_number: 0,
        }
    }

    fn export_decl(name: &str) -> ExportDecl {
        ExportDecl {
            name: name.to_string(),
            line_number: 0,
        }
    }

    #[test]
    fn empty_analysis_is_stable() {
        let a = StructuralAnalysis::default();
        let b = StructuralAnalysis::default();
        assert_eq!(structural_hash(&a), structural_hash(&b));
    }

    #[test]
    fn identical_analyses_match() {
        let mut a = StructuralAnalysis::default();
        a.functions.push(fn_decl("foo", &["x", "y"]));
        a.functions.push(fn_decl("bar", &[]));
        a.classes.push(class_decl("Widget", &["render", "build"]));
        a.imports.push(import_decl("std::fs"));
        a.exports.push(export_decl("foo"));

        let b = a.clone();
        assert_eq!(structural_hash(&a), structural_hash(&b));
    }

    #[test]
    fn order_independent_for_top_level_decls() {
        let mut a = StructuralAnalysis::default();
        a.functions.push(fn_decl("alpha", &[]));
        a.functions.push(fn_decl("beta", &[]));
        a.functions.push(fn_decl("gamma", &[]));

        let mut b = StructuralAnalysis::default();
        b.functions.push(fn_decl("gamma", &[]));
        b.functions.push(fn_decl("alpha", &[]));
        b.functions.push(fn_decl("beta", &[]));

        assert_eq!(structural_hash(&a), structural_hash(&b));
    }

    #[test]
    fn changes_on_function_rename() {
        let mut a = StructuralAnalysis::default();
        a.functions.push(fn_decl("foo", &["x"]));

        let mut b = StructuralAnalysis::default();
        b.functions.push(fn_decl("renamed", &["x"]));

        assert_ne!(structural_hash(&a), structural_hash(&b));
    }

    #[test]
    fn changes_on_param_added() {
        let mut a = StructuralAnalysis::default();
        a.functions.push(fn_decl("foo", &["x"]));

        let mut b = StructuralAnalysis::default();
        b.functions.push(fn_decl("foo", &["x", "y"]));

        assert_ne!(structural_hash(&a), structural_hash(&b));
    }

    #[test]
    fn changes_on_export_added() {
        let mut a = StructuralAnalysis::default();
        a.functions.push(fn_decl("foo", &[]));

        let mut b = a.clone();
        b.exports.push(export_decl("foo"));

        assert_ne!(structural_hash(&a), structural_hash(&b));
    }

    #[test]
    fn import_specifier_changes_dont_flip_hash() {
        let mut a = StructuralAnalysis::default();
        a.imports.push(ImportDecl {
            source: "react".to_string(),
            specifiers: vec!["useState".to_string()],
            line_number: 1,
        });

        let mut b = StructuralAnalysis::default();
        b.imports.push(ImportDecl {
            source: "react".to_string(),
            specifiers: vec!["useState".to_string(), "useEffect".to_string()],
            line_number: 1,
        });

        // Specifier list shifting (e.g. cosmetic reflow) shouldn't flip
        // the structural hash — only the source path matters.
        assert_eq!(structural_hash(&a), structural_hash(&b));
    }

    #[test]
    fn import_source_change_flips_hash() {
        let mut a = StructuralAnalysis::default();
        a.imports.push(import_decl("react"));

        let mut b = StructuralAnalysis::default();
        b.imports.push(import_decl("preact"));

        assert_ne!(structural_hash(&a), structural_hash(&b));
    }

    #[test]
    fn class_method_set_change_flips_hash() {
        let mut a = StructuralAnalysis::default();
        a.classes.push(class_decl("Widget", &["render"]));

        let mut b = StructuralAnalysis::default();
        b.classes.push(class_decl("Widget", &["render", "build"]));

        assert_ne!(structural_hash(&a), structural_hash(&b));
    }

    #[test]
    fn class_method_order_doesnt_flip_hash() {
        let mut a = StructuralAnalysis::default();
        a.classes.push(class_decl("Widget", &["render", "build"]));

        let mut b = StructuralAnalysis::default();
        b.classes.push(class_decl("Widget", &["build", "render"]));

        assert_eq!(structural_hash(&a), structural_hash(&b));
    }

    #[test]
    fn sections_factor_into_hash() {
        let a = StructuralAnalysis {
            sections: Some(vec![SectionInfo {
                name: "Intro".to_string(),
                level: 1,
                line_range: (1, 10),
            }]),
            ..Default::default()
        };

        let b = StructuralAnalysis {
            sections: Some(vec![SectionInfo {
                name: "Outro".to_string(),
                level: 1,
                line_range: (1, 10),
            }]),
            ..Default::default()
        };

        assert_ne!(structural_hash(&a), structural_hash(&b));
    }

    // -----------------------------------------------------------------
    // End-to-end tests through the actual tree-sitter parser. These
    // give us confidence that the canonicalisation survives real
    // parser output (ordering, byte spans, whitespace) rather than
    // just our hand-rolled `StructuralAnalysis` fixtures.
    // -----------------------------------------------------------------

    fn rust_hash(src: &str) -> String {
        let registry = crate::default_registry();
        registry
            .structural_hash_of("rust", "src/lib.rs", src)
            .expect("rust grammar is wired into the default registry")
    }

    #[test]
    fn structural_hash_stable_under_whitespace_edit() {
        let a = "fn foo() { 1 }\nfn bar() { 2 }\n";
        let b = "fn foo() {\n    1\n}\n\n\nfn bar() {\n    2\n}\n";
        assert_eq!(rust_hash(a), rust_hash(b));
    }

    #[test]
    fn structural_hash_stable_under_comment_edit() {
        let a = "fn foo() {}\n";
        let b = "// hello\nfn foo() {}\n// trailing\n";
        assert_eq!(rust_hash(a), rust_hash(b));
    }

    #[test]
    fn structural_hash_changes_on_function_rename() {
        let a = "fn foo() {}\n";
        let b = "fn renamed() {}\n";
        assert_ne!(rust_hash(a), rust_hash(b));
    }

    #[test]
    fn structural_hash_changes_on_param_added() {
        let a = "fn foo(x: i32) {}\n";
        let b = "fn foo(x: i32, y: i32) {}\n";
        assert_ne!(rust_hash(a), rust_hash(b));
    }

    #[test]
    fn structural_hash_independent_of_top_level_decl_order() {
        let a = "fn foo() {}\nfn bar() {}\n";
        let b = "fn bar() {}\nfn foo() {}\n";
        assert_eq!(rust_hash(a), rust_hash(b));
    }

    #[test]
    fn structural_hash_changes_on_export_added() {
        // In Rust we model "export" via `pub` — the registry's parser
        // won't capture it as an `ExportDecl` (Rust queries don't emit
        // `exp.def`), so for this test we use TypeScript where the
        // export concept maps directly.
        let registry = crate::default_registry();
        let ts_hash = |src: &str| -> String {
            registry
                .structural_hash_of("typescript", "src/x.ts", src)
                .expect("typescript grammar wired in")
        };
        let a = "function foo() {}\n";
        let b = "export function foo() {}\n";
        assert_ne!(ts_hash(a), ts_hash(b));
    }

    #[test]
    fn structural_hash_changes_on_function_body_only_is_cosmetic() {
        // Body-statement edits don't change the structural shape — so
        // the hash must match. This is the property the change
        // classifier leans on for `Cosmetic`.
        let a = "fn foo() { 1 + 1 }\n";
        let b = "fn foo() { 2 + 2 }\n";
        assert_eq!(rust_hash(a), rust_hash(b));
    }
}
