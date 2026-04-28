// Integration tests run via `cargo test --bin understandable`. Since
// ua-cli is a binary crate the builders aren't reachable from `tests/`
// without a lib target — wire them up via a small doctest-style smoke
// suite instead. This file exists so cargo treats the crate as having
// a test target and warns if we forget to add one.

#[test]
fn placeholder() {
    // Real exercising of builders happens through the smoke test in
    // `tests/cli_e2e.rs` (binary integration).
}
