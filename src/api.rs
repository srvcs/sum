use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::{OpenApi, ToSchema};

use crate::client::{self, DepError};

pub const SERVICE: &str = "srvcs-sum";
pub const CONCERN: &str = "aggregate: sum of a list";
pub const DEPENDS_ON: &[&str] = &["srvcs-add"];

/// Dependency endpoints, injected as router state so tests can point them at
/// mock services.
#[derive(Clone)]
pub struct Deps {
    pub add_url: String,
}

#[derive(Serialize, ToSchema)]
pub struct Info {
    pub service: &'static str,
    pub concern: &'static str,
    pub depends_on: Vec<&'static str>,
}

/// `GET /` — service identity (srvcs service standard).
#[utoipa::path(get, path = "/", responses((status = 200, body = Info)))]
pub async fn index() -> Json<Info> {
    Json(Info {
        service: SERVICE,
        concern: CONCERN,
        depends_on: DEPENDS_ON.to_vec(),
    })
}

#[derive(Deserialize, ToSchema)]
pub struct EvalRequest {
    /// The list of integers to sum. An empty list sums to `0`.
    #[schema(value_type = Object)]
    pub values: Vec<Value>,
}

#[derive(Serialize, ToSchema)]
pub struct SumResponse {
    #[schema(value_type = Object)]
    pub values: Vec<Value>,
    pub result: i64,
}

fn ok(values: Vec<Value>, result: i64) -> Response {
    (
        StatusCode::OK,
        Json(json!({ "values": values, "result": result })),
    )
        .into_response()
}

fn degraded(dependency: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "dependency unavailable", "dependency": dependency })),
    )
        .into_response()
}

fn forward(status: u16, body: Value) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    (code, Json(body)).into_response()
}

/// Ask `srvcs-add` to compute `acc + v`, returning the running total.
///
/// Maps the dependency's failures to the response this service should return:
/// `503` if it is unreachable, the forwarded `422` if `add` rejects the element
/// (e.g. a non-integer), and a generic `500` if `add` returns an unusable body.
async fn ask_add(url: &str, acc: i64, v: &Value) -> Result<i64, Response> {
    let body = json!({ "a": acc, "b": v });
    match client::call(url, &body).await {
        Err(DepError::Unreachable) => Err(degraded("srvcs-add")),
        Ok((200, body)) => match body.get("result").and_then(Value::as_i64) {
            Some(sum) => Ok(sum),
            None => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "srvcs-add returned no integer result" })),
            )
                .into_response()),
        },
        // Bad element (e.g. not an integer) — add already judged it; forward it.
        Ok((422, body)) => Err(forward(422, body)),
        Ok(_) => Err(degraded("srvcs-add")),
    }
}

/// `POST /` — sum a list of integers.
///
/// This service does no arithmetic of its own. It folds the list through
/// `srvcs-add`, starting from `0`: `acc = add(acc, v)` for each element. The
/// sum of the empty list is `0` and makes no dependency calls. If `add` rejects
/// an element the `422` is forwarded; if `add` is unreachable this service
/// reports itself degraded rather than guessing.
#[utoipa::path(
    post,
    path = "/",
    request_body = EvalRequest,
    responses(
        (status = 200, body = SumResponse),
        (status = 422, description = "an element is not a valid integer (forwarded from srvcs-add)"),
        (status = 500, description = "srvcs-add returned an unusable response"),
        (status = 503, description = "the srvcs-add dependency is unavailable")
    )
)]
pub async fn evaluate(State(deps): State<Deps>, Json(req): Json<EvalRequest>) -> Response {
    let mut acc: i64 = 0;
    for v in &req.values {
        acc = match ask_add(&deps.add_url, acc, v).await {
            Ok(sum) => sum,
            Err(resp) => return resp,
        };
    }
    ok(req.values, acc)
}

#[derive(OpenApi)]
#[openapi(
    paths(index, evaluate),
    components(schemas(Info, EvalRequest, SumResponse))
)]
pub struct ApiDoc;

/// Serve OpenAPI document
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_documents_routes() {
        let doc = ApiDoc::openapi();
        let root = doc.paths.paths.get("/").expect("path / present");
        assert!(root.get.is_some());
        assert!(root.post.is_some());
    }

    #[tokio::test]
    async fn index_reports_dependency() {
        let Json(info) = index().await;
        assert_eq!(info.service, "srvcs-sum");
        assert_eq!(info.concern, "aggregate: sum of a list");
        assert_eq!(info.depends_on, vec!["srvcs-add"]);
    }
}
