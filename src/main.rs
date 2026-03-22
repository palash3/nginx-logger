use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    extract::{Form, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use tokio::signal::unix::{SignalKind, signal};
use tracing::{Level, debug, error, info};
use tracing_subscriber::{filter::LevelFilter, prelude::*, reload};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("store mutex was poisoned")]
    StorePoisoned,

    #[error("failed to register signal handler: {0}")]
    SignalSetup(std::io::Error),

    #[error("failed to update log level filter: {0}")]
    LevelReload(tracing_subscriber::reload::Error),

    #[error("failed to bind listener: {0}")]
    Bind(std::io::Error),

    #[error("server error: {0}")]
    Serve(std::io::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        error!(error = %self, "request handler error");
        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()
    }
}

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

type Store = Arc<Mutex<HashMap<String, u32>>>;

#[derive(Clone)]
struct AppState {
    store: Store,
}

// ---------------------------------------------------------------------------
// HTML – the one-page form served at GET /
// ---------------------------------------------------------------------------

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <title>nginx-logger demo</title>
  <style>
    body { font-family: sans-serif; max-width: 480px; margin: 3rem auto; }
    label { display: block; margin: .6rem 0; }
    input  { margin-left: .4rem; }
    button { margin-top: .8rem; padding: .4rem 1.2rem; }
  </style>
</head>
<body>
  <h1>Add Entry</h1>
  <form method="POST" action="/add">
    <label>Name: <input type="text"   name="name" required /></label>
    <label>Age:  <input type="number" name="age"  required min="0" /></label>
    <button type="submit">Save</button>
  </form>
</body>
</html>"#;

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn index() -> Html<&'static str> {
    debug!("serving index page");
    Html(INDEX_HTML)
}

#[derive(Deserialize)]
struct AddForm {
    name: String,
    age: u32,
}

async fn add(
    State(state): State<AppState>,
    Form(form): Form<AddForm>,
) -> Result<Html<String>, AppError> {
    state
        .store
        .lock()
        .map_err(|_| AppError::StorePoisoned)?
        .insert(form.name.clone(), form.age);

    info!(name = %form.name, age = form.age, "entry stored");

    Ok(Html(format!(
        r#"<!DOCTYPE html><html><body>
          <p>✓ Saved <strong>{}</strong> (age {}).</p>
          <a href="/">← Back</a>
        </body></html>"#,
        form.name, form.age
    )))
}

// ---------------------------------------------------------------------------
// Nginx-style request logging middleware
//
//   DEBUG level → every request (method + path + status)
//   INFO  level → only successful (2xx) responses
// ---------------------------------------------------------------------------

async fn log_request(request: axum::extract::Request, next: Next) -> axum::response::Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    debug!(method = %method, path = %path, "→ request received");

    let response = next.run(request).await;
    let status = response.status();

    if status.is_success() {
        info!(method = %method, path = %path, status = %status.as_u16(), "← response sent");
    } else {
        debug!(method = %method, path = %path, status = %status.as_u16(), "← response sent");
    }

    response
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), AppError> {
    // Build a reload-capable level filter so we can toggle it via signal.
    let (filter, reload_handle) = reload::Layer::new(LevelFilter::DEBUG);

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Shared in-memory store.
    let state = AppState {
        store: Arc::new(Mutex::new(HashMap::new())),
    };

    // Register the SIGUSR1 stream before spawning so setup errors surface here.
    // Send with: kill -USR1 <pid>
    let mut sig_stream = signal(SignalKind::user_defined1()).map_err(AppError::SignalSetup)?;

    tokio::spawn(async move {
        let mut current = Level::DEBUG;
        loop {
            sig_stream.recv().await;
            current = if current == Level::DEBUG {
                Level::INFO
            } else {
                Level::DEBUG
            };
            if let Err(e) = reload_handle.modify(|f| *f = LevelFilter::from_level(current)) {
                error!(error = %AppError::LevelReload(e), "failed to update log level");
            } else {
                info!(level = %current, "log level changed via SIGUSR1");
            }
        }
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/add", post(add))
        .layer(middleware::from_fn(log_request))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .map_err(AppError::Bind)?;

    info!(pid = std::process::id(), "listening on http://0.0.0.0:3000");
    axum::serve(listener, app).await.map_err(AppError::Serve)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt; // for `.oneshot()`

    fn test_app() -> Router {
        let state = AppState {
            store: Arc::new(Mutex::new(HashMap::new())),
        };
        Router::new()
            .route("/", get(index))
            .route("/add", post(add))
            .with_state(state)
    }

    fn test_app_with_store(store: Store) -> Router {
        let state = AppState { store };
        Router::new()
            .route("/", get(index))
            .route("/add", post(add))
            .with_state(state)
    }

    #[tokio::test]
    async fn get_index_returns_200_with_form() {
        let app = test_app();
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains("<form"), "response should contain a <form>");
        assert!(
            html.contains(r#"name="name""#),
            "form should have a name field"
        );
        assert!(
            html.contains(r#"name="age""#),
            "form should have an age field"
        );
    }

    #[tokio::test]
    async fn post_add_stores_entry_and_returns_200() {
        let store: Store = Arc::new(Mutex::new(HashMap::new()));
        let app = test_app_with_store(Arc::clone(&store));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/add")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("name=Alice&age=30"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Verify the entry was persisted in the shared store.
        let map = store.lock().unwrap();
        assert_eq!(map.get("Alice"), Some(&30));
    }

    #[tokio::test]
    async fn post_add_confirmation_mentions_name_and_age() {
        let app = test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/add")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("name=Bob&age=25"))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains("Bob"), "confirmation should mention the name");
        assert!(html.contains("25"), "confirmation should mention the age");
    }

    #[tokio::test]
    async fn post_add_missing_fields_returns_422() {
        let app = test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/add")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("name=OnlyName"))
                    .unwrap(),
            )
            .await
            .unwrap();

        // axum returns 422 Unprocessable Entity when form fields are missing.
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
