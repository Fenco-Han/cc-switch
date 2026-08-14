use axum::{routing::post, Json, Router};
use cc_switch_lib::{Database, Provider, ProxyService};
use serde_json::json;
use serial_test::serial;
use std::sync::Arc;
use tokio::net::TcpListener;

async fn fake_deepseek_stream() -> axum::response::Response {
    let stream = async_stream::stream! {
        for text in ["HELLO", " FROM", " DEEPSEEK"] {
            yield Ok::<_, std::convert::Infallible>(bytes::Bytes::from(format!(
                "data: {}\n\n",
                json!({
                    "id": "mock",
                    "object": "chat.completion.chunk",
                    "model": "deepseek-chat",
                    "choices": [{
                        "index": 0,
                        "delta": { "content": text },
                        "finish_reason": serde_json::Value::Null,
                    }],
                })
            )));
        }
        yield Ok::<_, std::convert::Infallible>(bytes::Bytes::from(format!(
            "data: {}\n\n",
            json!({
                "id": "mock",
                "object": "chat.completion.chunk",
                "model": "deepseek-chat",
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop",
                }],
                "usage": {
                    "prompt_tokens": 5,
                    "completion_tokens": 3,
                    "total_tokens": 8,
                },
            })
        )));
        yield Ok::<_, std::convert::Infallible>(bytes::Bytes::from_static(b"data: [DONE]\n\n"));
    };

    axum::response::Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .body(axum::body::Body::from_stream(stream))
        .unwrap()
}

async fn fake_deepseek_chat(Json(_body): Json<serde_json::Value>) -> axum::response::Response {
    fake_deepseek_stream().await
}

async fn start_fake_upstream() -> String {
    let app = Router::new().route("/chat/completions", post(fake_deepseek_chat));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn accio_provider(base_url: String) -> Provider {
    Provider::with_id(
        "deepseek".to_string(),
        "DeepSeek".to_string(),
        json!({
            "name": "deepseek",
            "base_url": base_url,
            "api_key": "test-key",
            "api_format": "openai_chat",
            "model": "deepseek-chat",
        }),
        Some("https://platform.deepseek.com".to_string()),
    )
}

#[serial]
#[tokio::test]
async fn accio_adk_stream_translates_openai_chat_completions() {
    std::env::set_var(
        "ACCIO_HOME",
        std::env::temp_dir().join(format!("accio-adk-stream-test-{}", std::process::id())),
    );
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    std::env::set_var("no_proxy", "127.0.0.1,localhost");

    let upstream = start_fake_upstream().await;
    let db = Arc::new(Database::memory().expect("memory db"));
    db.save_provider("accio", &accio_provider(upstream))
        .expect("seed accio provider");

    let mut proxy_config = db.get_proxy_config().await.expect("proxy config");
    proxy_config.listen_port = 0;
    db.update_proxy_config(proxy_config)
        .await
        .expect("ephemeral port");

    let proxy = ProxyService::new(db);
    let info = proxy.start().await.expect("start proxy");

    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/accio/api/adk/llm/generateContent?sg_k=test",
            info.address, info.port
        ))
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .body(
            json!({
                "model": "deepseek-chat",
                "contents": [{"role": "user", "parts": [{"text": "say hello"}]}],
            })
            .to_string(),
        )
        .send()
        .await
        .expect("send ADK request");

    assert!(response.status().is_success());
    let body = response.text().await.expect("response body");
    assert!(body.contains("HELLO"));
    assert!(body.contains("finishReason"));

    proxy.stop().await.ok();
    std::env::remove_var("ACCIO_HOME");
}

#[serial]
#[tokio::test]
async fn accio_model_catalog_returns_db_provider_when_file_is_missing() {
    std::env::set_var("ACCIO_HOME", std::env::temp_dir().join("accio-adk-test"));
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    std::env::set_var("no_proxy", "127.0.0.1,localhost");

    let db = Arc::new(Database::memory().expect("memory db"));
    db.save_provider(
        "accio",
        &accio_provider("https://api.deepseek.com".to_string()),
    )
    .expect("seed accio provider");

    let mut proxy_config = db.get_proxy_config().await.expect("proxy config");
    proxy_config.listen_port = 0;
    db.update_proxy_config(proxy_config)
        .await
        .expect("ephemeral port");

    let proxy = ProxyService::new(db);
    let info = proxy.start().await.expect("start proxy");

    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}:{}/accio/api/llm/config/v2",
            info.address, info.port
        ))
        .json(&json!({ "token": "test-token" }))
        .send()
        .await
        .expect("send catalog request");

    assert!(response.status().is_success());
    let body = response.text().await.expect("catalog body");
    assert!(body.contains("deepseek-chat"));

    proxy.stop().await.ok();
    std::env::remove_var("ACCIO_HOME");
}
