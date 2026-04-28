use std::path::Path;
use ua_extract::{detect_frameworks, FrameworkRegistry};

#[test]
fn react_detected_in_package_json() {
    let r = FrameworkRegistry::default_registry();
    let pkg = r#"{"dependencies": {"react": "^19", "react-dom": "^19"}}"#;
    let manifests: Vec<(&Path, &str)> = vec![(Path::new("package.json"), pkg)];
    let imports: Vec<&str> = vec![];
    let found = detect_frameworks(&r, &manifests, &imports);
    assert!(found.contains(&"React".to_string()), "got {:?}", found);
}

#[test]
fn fastapi_detected_via_imports() {
    let r = FrameworkRegistry::default_registry();
    let imports = vec!["fastapi", "from fastapi"];
    let found = detect_frameworks(&r, &[], &imports);
    assert!(found.contains(&"FastAPI".to_string()), "got {:?}", found);
}

#[test]
fn axum_detected_via_cargo_toml() {
    let r = FrameworkRegistry::default_registry();
    let cargo = r#"[dependencies]
axum = "0.7"
tokio = { version = "1", features = ["full"] }
"#;
    let manifests: Vec<(&Path, &str)> = vec![(Path::new("Cargo.toml"), cargo)];
    let found = detect_frameworks(&r, &manifests, &[]);
    assert!(found.contains(&"Axum".to_string()), "got {:?}", found);
}

#[test]
fn nothing_detected_in_empty_project() {
    let r = FrameworkRegistry::default_registry();
    let found = detect_frameworks(&r, &[], &[]);
    assert!(found.is_empty(), "got {:?}", found);
}
