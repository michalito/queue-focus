use crate::{Database, Error};
use axum::{
    extract::{DefaultBodyLimit, Extension, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use qf_core::sync::{ExchangeRequest, Rejection, MAX_HTTP_BYTES};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone)]
struct App {
    database: Arc<Mutex<Database>>,
    slots: Arc<Semaphore>,
}

pub fn router(database: Database) -> Router {
    let app = App {
        database: Arc::new(Mutex::new(database)),
        slots: Arc::new(Semaphore::new(8)),
    };
    Router::new()
        .route("/health", get(health))
        .route("/v1/exchange", post(exchange))
        .layer(DefaultBodyLimit::max(MAX_HTTP_BYTES))
        .layer(middleware::from_fn_with_state(app.clone(), admission))
        .with_state(app)
}

// Bound requests before buffering JSON. Keep a cloned permit in the blocking
// worker: cancellation of an HTTP request does not cancel a SQLite transaction.
async fn admission(State(app): State<App>, mut request: Request, next: Next) -> Response {
    let permit = match app.slots.clone().try_acquire_owned() {
        Ok(p) => Arc::new(p),
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Server is busy. Retry the same command.",
            )
                .into_response()
        }
    };
    request.extensions_mut().insert(permit.clone());
    match tokio::time::timeout(std::time::Duration::from_secs(30), next.run(request)).await {
        Ok(response) => response,
        Err(_) => (
            StatusCode::REQUEST_TIMEOUT,
            "Request timed out. Retry the same command.",
        )
            .into_response(),
    }
}

async fn health(
    State(app): State<App>,
    Extension(permit): Extension<Arc<OwnedSemaphorePermit>>,
) -> StatusCode {
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        app.database.lock().map(|db| db.healthy()).unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    if result {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn exchange(
    State(app): State<App>,
    Extension(permit): Extension<Arc<OwnedSemaphorePermit>>,
    headers: HeaderMap,
    Json(request): Json<ExchangeRequest>,
) -> Response {
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("")
        .to_owned();
    if token.is_empty() || token.len() > 256 {
        return response_error(Error::Unauthorized);
    }
    let Some(device) = headers
        .get("x-queue-focus-device")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<qf_core::sync::Id>().ok())
        .filter(|id| !id.is_nil())
    else {
        return response_error(Error::Unauthorized);
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut database = app
            .database
            .lock()
            .map_err(|_| Error::Storage("database worker failed".into()))?;
        database.exchange_for_device(&token, device, &request, qf_core::unix_now())
    })
    .await;
    match result {
        Ok(Ok(response)) => Json(response).into_response(),
        Ok(Err(error)) => response_error(error),
        Err(_) => response_error(Error::Storage("database worker stopped".into())),
    }
}

#[derive(Serialize)]
struct Failure {
    message: String,
    reason: Option<Rejection>,
}

fn response_error(error: Error) -> Response {
    let (status, message, reason) = match error {
        Error::Unauthorized => (
            StatusCode::UNAUTHORIZED,
            Error::Unauthorized.to_string(),
            None,
        ),
        Error::ReceiptMismatch => (
            StatusCode::CONFLICT,
            Error::ReceiptMismatch.to_string(),
            None,
        ),
        Error::Protocol(reason) => {
            let status = match reason {
                Rejection::WrongDataset | Rejection::HistoryMismatch => StatusCode::CONFLICT,
                _ => StatusCode::BAD_REQUEST,
            };
            (status, reason.to_string(), Some(reason))
        }
        Error::Invalid(message) => (StatusCode::BAD_REQUEST, message, None),
        Error::Storage(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Sync storage is unavailable. Retry the same command.".into(),
            None,
        ),
    };
    (status, Json(Failure { message, reason })).into_response()
}
