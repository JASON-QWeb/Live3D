use crate::{api, state::App};
use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use subtle::ConstantTimeEq;

async fn guard(State((app, port)): State<(App, u16)>, req: Request, next: Next) -> Response {
    let h = req.headers();
    let host = h
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let allowed = host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}");
    if !allowed {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(origin) = h.get(header::ORIGIN)
        && origin.as_bytes() != format!("http://{host}").as_bytes()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let path = req.uri().path();
    if path.starts_with("/api/") {
        let candidate = h
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        let secret = match path {
            p if p == "/api/output"
                || p.starts_with("/api/frame/")
                || p.starts_with("/api/test-audio/")
                || p.starts_with("/api/stage/") =>
            {
                &app.tokens.stage
            }
            "/api/ingest" => &app.tokens.ingest,
            _ => &app.tokens.admin,
        };
        if !bool::from(candidate.as_bytes().ct_eq(secret.as_bytes())) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    } else if req.method() != Method::GET && req.method() != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let mut r = next.run(req).await;
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    r
}
fn js(s: &'static str) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        s,
    )
}
pub fn router(app: App, port: u16) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../web/index.html")) }),
        )
        .route(
            "/stage",
            get(|| async { Html(include_str!("../web/stage.html")) }),
        )
        .route(
            "/online-bgm.js",
            get(|| async { js(include_str!("../web/online-bgm.js")) }),
        )
        .route(
            "/studio.js",
            get(|| async { js(include_str!("../web/studio.js")) }),
        )
        .route(
            "/settings.js",
            get(|| async { js(include_str!("../web/settings.js")) }),
        )
        .route(
            "/i18n.js",
            get(|| async { js(include_str!("../web/i18n.js")) }),
        )
        .route(
            "/i18n-en.json",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                    include_str!("../web/i18n-en.json"),
                )
            }),
        )
        .route(
            "/stage-dance.js",
            get(|| async { js(include_str!("../web/stage-dance.js")) }),
        )
        .route(
            "/studio-dance.js",
            get(|| async { js(include_str!("../web/studio-dance.js")) }),
        )
        .route(
            "/stage.js",
            get(|| async { js(include_str!("../web/stage.js")) }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../web/style.css"),
                )
            }),
        )
        .route(
            "/mark.svg",
            get(|| async { axum::response::Redirect::temporary("/mark.png") }),
        )
        .route(
            "/mark.png",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/png")],
                    include_bytes!("../web/mark.png").as_slice(),
                )
            }),
        )
        .route(
            "/health",
            get(|| async {
                (
                    [("x-vorela-version", env!("CARGO_PKG_VERSION"))],
                    "vorela",
                )
            }),
        )
        .merge(api::routes())
        .layer(middleware::from_fn_with_state((app.clone(), port), guard))
        .with_state(app)
}
