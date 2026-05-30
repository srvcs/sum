use axum::body::Body;
use axum::extract::Json as JsonExtract;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router as AxumRouter};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use srvcs_sum::{api::Deps, health, router, telemetry};
use tower::ServiceExt;

const DEAD_URL: &str = "http://127.0.0.1:1";

/// Mock `srvcs-add` that ACTUALLY COMPUTES: it reads `{a, b}` from the request
/// and returns `{"a", "b", "result": a + b}`. This is what makes the fold
/// genuinely testable — the running accumulator is real, not faked.
async fn spawn_computing_add() -> String {
    let app = AxumRouter::new().route(
        "/",
        post(|JsonExtract(req): JsonExtract<Value>| async move {
            let a = req["a"].as_i64().unwrap_or(0);
            let b = req["b"].as_i64().unwrap_or(0);
            Json(json!({ "a": a, "b": b, "result": a + b }))
        }),
    );
    serve(app).await
}

/// Mock `srvcs-add` that always answers with a fixed status + body (used to
/// simulate a `422` rejection of a bad element).
async fn spawn_fixed(status: StatusCode, body: Value) -> String {
    let app = AxumRouter::new().route(
        "/",
        post(move || {
            let body = body.clone();
            async move { (status, Json(body)) }
        }),
    );
    serve(app).await
}

async fn serve(app: AxumRouter) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn app(add_url: &str) -> axum::Router {
    router(
        telemetry::metrics_handle_for_tests(),
        Deps {
            add_url: add_url.to_string(),
        },
    )
}

async fn eval(add_url: &str, values: Value) -> (StatusCode, Value) {
    let res = app(add_url)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "values": values }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn status_of(uri: &str) -> StatusCode {
    app(DEAD_URL)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn healthz_ok() {
    assert_eq!(status_of("/healthz").await, StatusCode::OK);
}

#[tokio::test]
async fn readyz_reflects_state() {
    health::set_ready(true);
    assert_eq!(status_of("/readyz").await, StatusCode::OK);
}

#[tokio::test]
async fn openapi_ok() {
    assert_eq!(status_of("/openapi.json").await, StatusCode::OK);
}

// --- Correctness cases from the spec, exercised against a REAL computing add ---

#[tokio::test]
async fn sums_a_list() {
    let add = spawn_computing_add().await;
    let (status, body) = eval(&add, json!([1, 2, 3, 4])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], 10);
    assert_eq!(body["values"], json!([1, 2, 3, 4]));
}

#[tokio::test]
async fn sum_of_empty_list_is_zero_with_no_calls() {
    // DEAD_URL: if the fold tried to call add at all on an empty list, this
    // would degrade to 503. It must short-circuit to 0 with no calls.
    let (status, body) = eval(DEAD_URL, json!([])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], 0);
    assert_eq!(body["values"], json!([]));
}

#[tokio::test]
async fn sum_of_singleton_is_the_element() {
    let add = spawn_computing_add().await;
    let (status, body) = eval(&add, json!([5])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], 5);
}

#[tokio::test]
async fn sums_negatives() {
    let add = spawn_computing_add().await;
    let (status, body) = eval(&add, json!([10, -3, -7])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], 0);
}

// --- Error / edge cases ---

#[tokio::test]
async fn forwards_422_for_bad_element() {
    let add = spawn_fixed(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "error": "value is not an integer" }),
    )
    .await;
    let (status, body) = eval(&add, json!([1, "nope", 3])).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "value is not an integer");
}

#[tokio::test]
async fn degrades_when_add_is_unreachable() {
    let (status, body) = eval(DEAD_URL, json!([1, 2, 3])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-add");
}
