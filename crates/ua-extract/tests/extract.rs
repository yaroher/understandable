//! Sanity tests against tiny real-world snippets per language.

use std::path::Path;
use ua_extract::{default_registry, LanguageRegistry};

fn registry() -> ua_extract::PluginRegistry {
    default_registry()
}

#[test]
fn ts_extracts_function_class_import_export() {
    let src = r#"
import { foo, bar as baz } from "./util";
import * as ns from "./ns";

export function add(a: number, b: number): number {
    return a + b;
}

export class Counter {
    count: number = 0;
    increment(by: number): void {
        this.count += by;
    }
}

const square = (n: number) => n * n;
"#;
    let r = registry();
    let a = r.analyze_file("typescript", "x.ts", src).unwrap();
    let fn_names: Vec<&str> = a.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(fn_names.contains(&"add"), "got {:?}", fn_names);
    assert!(fn_names.contains(&"square"), "got {:?}", fn_names);
    assert!(fn_names.contains(&"increment"), "got {:?}", fn_names);

    let cls_names: Vec<&str> = a.classes.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(cls_names, vec!["Counter"]);
    let counter = a.classes.iter().find(|c| c.name == "Counter").unwrap();
    assert!(counter.methods.contains(&"increment".to_string()));
    assert!(counter.properties.contains(&"count".to_string()));

    let import_sources: Vec<&str> = a.imports.iter().map(|i| i.source.as_str()).collect();
    assert_eq!(import_sources, vec!["./util", "./ns"]);

    let export_names: Vec<&str> = a.exports.iter().map(|e| e.name.as_str()).collect();
    assert!(export_names.contains(&"add"));
    assert!(export_names.contains(&"Counter"));
}

#[test]
fn ts_call_graph_has_caller() {
    let src = r#"
function helper() {
    return 1;
}

function entry() {
    helper();
    helper();
}
"#;
    let r = registry();
    let calls = r.extract_call_graph("typescript", "x.ts", src).unwrap();
    let from_entry: Vec<&str> = calls
        .iter()
        .filter(|c| c.caller == "entry")
        .map(|c| c.callee.as_str())
        .collect();
    assert_eq!(from_entry, vec!["helper", "helper"]);
}

#[test]
fn python_extracts_function_class_import() {
    let src = r#"
import os
from typing import List

def add(a, b):
    return a + b

class Counter:
    def __init__(self):
        self.count = 0
    def increment(self, by):
        self.count += by
"#;
    let r = registry();
    let a = r.analyze_file("python", "x.py", src).unwrap();

    let fn_names: Vec<&str> = a.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(fn_names.contains(&"add"), "got {:?}", fn_names);
    assert!(fn_names.contains(&"increment"), "got {:?}", fn_names);
    assert!(fn_names.contains(&"__init__"), "got {:?}", fn_names);

    let cls_names: Vec<&str> = a.classes.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(cls_names, vec!["Counter"]);

    let import_sources: Vec<&str> = a.imports.iter().map(|i| i.source.as_str()).collect();
    assert!(import_sources.contains(&"os"));
    assert!(import_sources.contains(&"typing"));
}

#[test]
fn rust_extracts_fn_struct_use() {
    let src = r#"
use std::collections::HashMap;
use serde::Serialize;

pub fn entry() -> i32 {
    42
}

pub struct Counter {
    pub count: i32,
}

impl Counter {
    pub fn increment(&mut self, by: i32) {
        self.count += by;
    }
}
"#;
    let r = registry();
    let a = r.analyze_file("rust", "x.rs", src).unwrap();
    let fn_names: Vec<&str> = a.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(fn_names.contains(&"entry"), "got {:?}", fn_names);
    assert!(fn_names.contains(&"increment"), "got {:?}", fn_names);

    let cls_names: Vec<&str> = a.classes.iter().map(|c| c.name.as_str()).collect();
    assert!(cls_names.contains(&"Counter"));

    assert_eq!(a.imports.len(), 2);
}

#[test]
fn go_extracts_func_method_struct() {
    let src = r#"
package main

import "fmt"

type Counter struct {
    count int
}

func (c *Counter) Increment(by int) {
    c.count += by
}

func main() {
    fmt.Println("hello")
}
"#;
    let r = registry();
    let a = r.analyze_file("go", "x.go", src).unwrap();
    let fn_names: Vec<&str> = a.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(fn_names.contains(&"main"), "got {:?}", fn_names);
    assert!(fn_names.contains(&"Increment"), "got {:?}", fn_names);
    assert!(a.classes.iter().any(|c| c.name == "Counter"));
}

#[test]
fn unknown_language_errors() {
    let r = registry();
    let err = r.analyze_file("brainfuck", "x.bf", "+++.").unwrap_err();
    assert!(
        matches!(&err, ua_extract::Error::Plugin(s) if s.contains("brainfuck")),
        "expected `no plugin for language brainfuck` error, got: {err:?}"
    );
}

#[test]
fn language_registry_routes_path() {
    let lr = LanguageRegistry::default_registry();
    assert_eq!(lr.for_path(Path::new("a.ts")).unwrap().id, "typescript");
    assert_eq!(lr.for_path(Path::new("a.py")).unwrap().id, "python");
    assert_eq!(lr.for_path(Path::new("a.rs")).unwrap().id, "rust");
}
