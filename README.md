# nginx-logger

A small Axum web server that mimics Nginx-style request logging with a runtime-togglable log level.

## Features

- **Web form** — `GET /` serves an HTML form to submit a name and age
- **In-memory store** — `POST /add` saves entries to a shared `HashMap` and confirms the submission
- **Nginx-style middleware** — every request is logged at `DEBUG`; only successful (2xx) responses are logged at `INFO`
- **Runtime log-level toggle** — send `SIGUSR1` to the process to flip between `DEBUG` and `INFO` without restarting
- **Structured error handling** — typed `AppError` enum via `thiserror`; handler errors return HTTP 500 automatically

## Requirements

- Rust (edition 2024, stable toolchain)
- Unix-like OS (signal handling requires `tokio::signal::unix`)

## Running

```bash
cargo run
```

The server binds to `http://0.0.0.0:3000`. The PID is printed on startup:

```
INFO nginx_logger: listening on http://0.0.0.0:3000 pid=12345
```

## Toggling the log level

Send `SIGUSR1` to the running process to toggle between `DEBUG` and `INFO`:

```bash
# using the PID printed at startup
kill -USR1 <pid>

# or look it up
kill -USR1 $(pgrep nginx-logger)
```

Each signal inverts the current level:

| Before | After |
|--------|-------|
| `DEBUG` (default) | `INFO` |
| `INFO` | `DEBUG` |

At `DEBUG` level every request is logged. At `INFO` level only successful responses appear.

## Log level behaviour

| Event | `DEBUG` level | `INFO` level |
|-------|--------------|-------------|
| Incoming request | ✓ logged | — |
| 2xx response | ✓ logged | ✓ logged |
| Non-2xx response | ✓ logged | — |

## API

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/` | Serves the HTML entry form |
| `POST` | `/add` | Stores `name` + `age` from form body; returns a confirmation page |

`POST /add` expects `application/x-www-form-urlencoded` with `name` (string) and `age` (unsigned integer). Missing fields return `422 Unprocessable Entity`.

## Testing

```bash
cargo test
```

10 tests cover:

- `GET /` returns 200 with a `<form>` containing `name` and `age` fields
- `POST /add` persists the entry in the shared store and returns 200
- `POST /add` confirmation body echoes the submitted name and age
- `POST /add` with missing fields returns 422
- `toggle_level` pure function: `DEBUG → INFO`, `INFO → DEBUG`, and round-trip
- `apply_toggle` with a live reload handle: `DEBUG → INFO`, `INFO → DEBUG`, and round-trip

## Dependencies

| Crate | Purpose |
|-------|---------|
| `axum` | HTTP router and middleware |
| `tokio` | Async runtime and Unix signal handling |
| `tracing` / `tracing-subscriber` | Structured logging with a runtime-reloadable level filter |
| `serde` | Form deserialization |
| `thiserror` | Typed error enum with `Display` derive |

