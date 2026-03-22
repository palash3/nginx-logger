use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    extract::{Form, State},
    middleware::{self, Next},
    response::Html,
    routing::{get, post},
};
use serde::Deserialize;
use tokio::signal::unix::{SignalKind, signal};
use tracing::{Level, debug, info};
use tracing_subscriber::{filter::LevelFilter, prelude::*, reload};

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

async fn add(State(state): State<AppState>, Form(form): Form<AddForm>) -> Html<String> {
    state
        .store
        .lock()
        .expect("store mutex poisoned")
        .insert(form.name.clone(), form.age);

    info!(name = %form.name, age = form.age, "entry stored");

    Html(format!(
        r#"<!DOCTYPE html><html><body>
          <p>✓ Saved <strong>{}</strong> (age {}).</p>
          <a href="/">← Back</a>
        </body></html>"#,
        form.name, form.age
    ))
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
async fn main() {
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

    // Register the SIGUSR1 stream. Send with: kill -USR1 <pid>
    let mut sig_stream =
        signal(SignalKind::user_defined1()).expect("failed to register SIGUSR1 handler");

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
                eprintln!("failed to update log level: {e}");
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
        .expect("failed to bind to 0.0.0.0:3000");

    info!(pid = std::process::id(), "listening on http://0.0.0.0:3000");
    axum::serve(listener, app).await.expect("server error");
}
