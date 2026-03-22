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
// Signal helpers
// ---------------------------------------------------------------------------

/// Pure toggle: DEBUG → INFO → DEBUG → …
fn toggle_level(current: Level) -> Level {
    if current == Level::DEBUG {
        Level::INFO
    } else {
        Level::DEBUG
    }
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
            current = toggle_level(current);
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
    #[cfg(unix)]
    use libc;
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

    // -----------------------------------------------------------------------
    // Signal / log-level tests
    // -----------------------------------------------------------------------

    #[test]
    fn toggle_level_debug_becomes_info() {
        assert_eq!(toggle_level(Level::DEBUG), Level::INFO);
    }

    #[test]
    fn toggle_level_info_becomes_debug() {
        assert_eq!(toggle_level(Level::INFO), Level::DEBUG);
    }

    #[test]
    fn toggle_level_cycles_back() {
        let l = Level::DEBUG;
        let l = toggle_level(l);
        let l = toggle_level(l);
        assert_eq!(l, Level::DEBUG, "two toggles should return to DEBUG");
    }

    #[tokio::test]
    async fn reload_handle_updates_level_filter() {
        // `_layer` must stay alive: the handle holds a Weak reference to it.
        let (_layer, handle) =
            reload::Layer::<LevelFilter, tracing_subscriber::Registry>::new(LevelFilter::DEBUG);

        handle
            .modify(|f| *f = LevelFilter::INFO)
            .expect("modify should succeed");

        handle
            .with_current(|f| assert_eq!(*f, LevelFilter::INFO))
            .expect("with_current should succeed");
    }

    /// End-to-end: send SIGUSR1 to the current process and verify the reload
    /// handle reflects the toggled level.
    #[tokio::test]
    async fn sigusr1_toggles_reload_handle_level() {
        // `_layer` must stay alive: the handle holds a Weak reference to it.
        let (_layer, handle) =
            reload::Layer::<LevelFilter, tracing_subscriber::Registry>::new(LevelFilter::DEBUG);
        let handle_task = handle.clone();

        // Shared level so the test can inspect what the task recorded.
        let level_seen: Arc<Mutex<Option<Level>>> = Arc::new(Mutex::new(None));
        let level_seen_task = Arc::clone(&level_seen);

        let mut sig =
            signal(SignalKind::user_defined1()).expect("failed to register SIGUSR1 in test");

        tokio::spawn(async move {
            sig.recv().await;
            let new_level = toggle_level(Level::DEBUG);
            *level_seen_task.lock().unwrap() = Some(new_level);
            handle_task
                .modify(|f| *f = LevelFilter::from_level(new_level))
                .unwrap();
        });

        // Send SIGUSR1 to ourselves.
        unsafe { libc::raise(libc::SIGUSR1) };

        // Give the spawned task a moment to process the signal.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // The task should have toggled DEBUG → INFO.
        assert_eq!(
            *level_seen.lock().unwrap(),
            Some(Level::INFO),
            "task should have recorded INFO after first SIGUSR1"
        );

        handle
            .with_current(|f| assert_eq!(*f, LevelFilter::INFO))
            .expect("reload handle should reflect INFO");
    }
}
