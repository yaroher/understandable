//! Verifies the `Authorization` header behaviour of [`OpenAiEmbeddings`].
//!
//! Strict local OpenAI-compatible stacks (LiteLLM, vllm) reject
//! requests carrying an empty `Authorization: Bearer ` header rather
//! than silently ignoring them, so `new_keyless()` must omit the
//! header entirely. Conversely, calls built with a real key MUST
//! send `Authorization: Bearer <key>`.

use ua_llm::OpenAiEmbeddings;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OK_BODY: &str = r#"{
  "data": [
    { "index": 0, "embedding": [0.1, 0.2, 0.3] }
  ]
}"#;

#[tokio::test]
async fn keyless_omits_authorization_header() {
    let server = MockServer::start().await;

    // The mock matches only when the Authorization header is *absent*.
    // wiremock has no built-in `header_absent`, so we register two
    // responders: a 200 for the "no Authorization" case, and a 401
    // catch-all that fires whenever the header is present.
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(401).set_body_string("auth header leaked"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_string(OK_BODY))
        .mount(&server)
        .await;

    let client = OpenAiEmbeddings::new_keyless().with_base_url(server.uri());
    let out = client
        .embed(&["hello"])
        .await
        .expect("embed must succeed without auth header");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0], vec![0.1, 0.2, 0.3]);

    // Sanity: confirm the captured request really had no Authorization.
    let received = server.received_requests().await.expect("requests captured");
    assert_eq!(received.len(), 1);
    assert!(
        received[0].headers.get("authorization").is_none(),
        "Authorization header must not be sent in keyless mode; got {:?}",
        received[0].headers.get("authorization")
    );
}

#[tokio::test]
async fn with_key_sends_bearer_authorization() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .and(header("authorization", "Bearer sk-test-1234"))
        .respond_with(ResponseTemplate::new(200).set_body_string(OK_BODY))
        .mount(&server)
        .await;

    let client = OpenAiEmbeddings::new(Some("sk-test-1234".into()))
        .expect("client built")
        .with_base_url(server.uri());
    let out = client
        .embed(&["hello"])
        .await
        .expect("embed must succeed with auth header");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0], vec![0.1, 0.2, 0.3]);

    let received = server.received_requests().await.expect("requests captured");
    assert_eq!(received.len(), 1);
    let auth = received[0]
        .headers
        .get("authorization")
        .expect("Authorization header must be present when a key is configured")
        .to_str()
        .unwrap();
    assert_eq!(auth, "Bearer sk-test-1234");
}
