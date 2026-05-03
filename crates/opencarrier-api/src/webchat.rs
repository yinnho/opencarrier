//! Embedded WebChat UI served as static HTML.
//!
//! The production dashboard is assembled at compile time from separate
//! HTML/CSS/JS files under `static/` using `include_str!()`. This keeps
//! single-binary deployment while allowing organized source files.
//!
//! Features:
//! - Alpine.js SPA with hash-based routing (10 panels)
//! - Dark/light theme toggle with system preference detection
//! - Responsive layout with collapsible sidebar
//! - Markdown rendering + syntax highlighting (bundled locally)
//! - WebSocket real-time chat with HTTP fallback
//! - Agent management, memory browser, audit log, and more

use axum::http::header;
use axum::response::IntoResponse;

/// Compile-time ETag based on the crate version.
const ETAG: &str = concat!("\"opencarrier-", env!("CARGO_PKG_VERSION"), "\"");

/// Embedded logo PNG for single-binary deployment.
const LOGO_PNG: &[u8] = include_bytes!("../static/logo.png");

/// Embedded favicon ICO for browser tabs.
const FAVICON_ICO: &[u8] = include_bytes!("../static/favicon.ico");

/// GET /logo.png — Serve the OpenCarrier logo.
pub async fn logo_png() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        LOGO_PNG,
    )
}

/// GET /favicon.ico — Serve the OpenCarrier favicon.
pub async fn favicon_ico() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/x-icon"),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        FAVICON_ICO,
    )
}

/// Embedded PWA manifest for installable web app support.
const MANIFEST_JSON: &str = include_str!("../static/manifest.json");

/// Embedded service worker for PWA support.
const SW_JS: &str = include_str!("../static/sw.js");

/// GET /manifest.json — Serve the PWA web app manifest.
pub async fn manifest_json() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/manifest+json"),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        MANIFEST_JSON,
    )
}

/// Embedded KaTeX font files — all .woff2 variants bundled at compile time.
const KATEX_FONT_AMS_REGULAR: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_AMS-Regular.woff2");
const KATEX_FONT_CALIGRAPHIC_BOLD: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Caligraphic-Bold.woff2");
const KATEX_FONT_CALIGRAPHIC_REGULAR: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Caligraphic-Regular.woff2");
const KATEX_FONT_FRAKTUR_BOLD: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Fraktur-Bold.woff2");
const KATEX_FONT_FRAKTUR_REGULAR: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Fraktur-Regular.woff2");
const KATEX_FONT_MAIN_BOLD: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Main-Bold.woff2");
const KATEX_FONT_MAIN_BOLDITALIC: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Main-BoldItalic.woff2");
const KATEX_FONT_MAIN_ITALIC: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Main-Italic.woff2");
const KATEX_FONT_MAIN_REGULAR: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Main-Regular.woff2");
const KATEX_FONT_MATH_BOLDITALIC: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Math-BoldItalic.woff2");
const KATEX_FONT_MATH_ITALIC: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Math-Italic.woff2");
const KATEX_FONT_SANSSERIF_BOLD: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_SansSerif-Bold.woff2");
const KATEX_FONT_SANSSERIF_ITALIC: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_SansSerif-Italic.woff2");
const KATEX_FONT_SANSSERIF_REGULAR: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_SansSerif-Regular.woff2");
const KATEX_FONT_SCRIPT_REGULAR: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Script-Regular.woff2");
const KATEX_FONT_SIZE1_REGULAR: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Size1-Regular.woff2");
const KATEX_FONT_SIZE2_REGULAR: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Size2-Regular.woff2");
const KATEX_FONT_SIZE3_REGULAR: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Size3-Regular.woff2");
const KATEX_FONT_SIZE4_REGULAR: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Size4-Regular.woff2");
const KATEX_FONT_TYPEWRITER_REGULAR: &[u8] =
    include_bytes!("../static/vendor/katex-fonts/KaTeX_Typewriter-Regular.woff2");

/// GET /sw.js — Serve the PWA service worker.
pub async fn sw_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        SW_JS,
    )
}

/// GET /katex-fonts/:name — Serve a KaTeX font file (.woff2 only).
pub async fn katex_font(
    axum::extract::Path(name): axum::extract::Path<String>,
) -> axum::response::Response<axum::body::Body> {
    let (data, content_type) = match name.as_str() {
        "KaTeX_AMS-Regular.woff2" => (KATEX_FONT_AMS_REGULAR, "font/woff2"),
        "KaTeX_Caligraphic-Bold.woff2" => (KATEX_FONT_CALIGRAPHIC_BOLD, "font/woff2"),
        "KaTeX_Caligraphic-Regular.woff2" => (KATEX_FONT_CALIGRAPHIC_REGULAR, "font/woff2"),
        "KaTeX_Fraktur-Bold.woff2" => (KATEX_FONT_FRAKTUR_BOLD, "font/woff2"),
        "KaTeX_Fraktur-Regular.woff2" => (KATEX_FONT_FRAKTUR_REGULAR, "font/woff2"),
        "KaTeX_Main-Bold.woff2" => (KATEX_FONT_MAIN_BOLD, "font/woff2"),
        "KaTeX_Main-BoldItalic.woff2" => (KATEX_FONT_MAIN_BOLDITALIC, "font/woff2"),
        "KaTeX_Main-Italic.woff2" => (KATEX_FONT_MAIN_ITALIC, "font/woff2"),
        "KaTeX_Main-Regular.woff2" => (KATEX_FONT_MAIN_REGULAR, "font/woff2"),
        "KaTeX_Math-BoldItalic.woff2" => (KATEX_FONT_MATH_BOLDITALIC, "font/woff2"),
        "KaTeX_Math-Italic.woff2" => (KATEX_FONT_MATH_ITALIC, "font/woff2"),
        "KaTeX_SansSerif-Bold.woff2" => (KATEX_FONT_SANSSERIF_BOLD, "font/woff2"),
        "KaTeX_SansSerif-Italic.woff2" => (KATEX_FONT_SANSSERIF_ITALIC, "font/woff2"),
        "KaTeX_SansSerif-Regular.woff2" => (KATEX_FONT_SANSSERIF_REGULAR, "font/woff2"),
        "KaTeX_Script-Regular.woff2" => (KATEX_FONT_SCRIPT_REGULAR, "font/woff2"),
        "KaTeX_Size1-Regular.woff2" => (KATEX_FONT_SIZE1_REGULAR, "font/woff2"),
        "KaTeX_Size2-Regular.woff2" => (KATEX_FONT_SIZE2_REGULAR, "font/woff2"),
        "KaTeX_Size3-Regular.woff2" => (KATEX_FONT_SIZE3_REGULAR, "font/woff2"),
        "KaTeX_Size4-Regular.woff2" => (KATEX_FONT_SIZE4_REGULAR, "font/woff2"),
        "KaTeX_Typewriter-Regular.woff2" => (KATEX_FONT_TYPEWRITER_REGULAR, "font/woff2"),
        _ => {
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::NOT_FOUND)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(axum::body::Body::from("font not found"))
                .unwrap();
        }
    };
    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(axum::body::Body::from(data))
        .unwrap()
}

/// GET / — Serve the OpenCarrier Dashboard single-page application.
///
/// Returns the full SPA with ETag header based on package version for caching.
pub async fn webchat_page() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::ETAG, ETAG),
            (
                header::CACHE_CONTROL,
                "public, max-age=3600, must-revalidate",
            ),
        ],
        WEBCHAT_HTML,
    )
}

/// Serve the public share/onboarding page.
pub async fn share_page() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        SHARE_HTML,
    )
}

/// The embedded HTML/CSS/JS for the OpenCarrier Dashboard.
///
/// Assembled at compile time from organized static files.
/// All vendor libraries (Alpine.js, marked.js, highlight.js) are bundled
/// locally — no CDN dependency. Alpine.js is included LAST because it
/// immediately processes x-data directives and fires alpine:init on load.
const WEBCHAT_HTML: &str = concat!(
    include_str!("../static/index_head.html"),
    "<style>\n",
    include_str!("../static/css/theme.css"),
    "\n",
    include_str!("../static/css/layout.css"),
    "\n",
    include_str!("../static/css/components.css"),
    "\n",
    include_str!("../static/css/bots.css"),
    "\n",
    include_str!("../static/vendor/github-dark.min.css"),
    "\n",
    include_str!("../static/vendor/katex.min.css"),
    "\n</style>\n",
    include_str!("../static/index_body.html"),
    // Vendor libs: marked + highlight first (used by app.js), then Chart.js
    "<script>\n",
    include_str!("../static/vendor/marked.min.js"),
    "\n</script>\n",
    "<script>\n",
    include_str!("../static/vendor/highlight.min.js"),
    "\n</script>\n",
    "<script>\n",
    include_str!("../static/vendor/chart.umd.min.js"),
    "\n</script>\n",
    // KaTeX math rendering (bundled locally, zero CDN dependency)
    "<script>\n",
    include_str!("../static/vendor/katex.min.js"),
    "\n</script>\n",
    "<script>\n",
    include_str!("../static/vendor/katex-auto-render.min.js"),
    "\n</script>\n",
    // App code
    "<script>\n",
    include_str!("../static/vendor/qrcode.min.js"),
    "\n",
    include_str!("../static/js/api.js"),
    "\n",
    include_str!("../static/js/app.js"),
    "\n",
    include_str!("../static/js/pages/overview.js"),
    "\n",
    include_str!("../static/js/pages/chat.js"),
    "\n",
    include_str!("../static/js/pages/agents.js"),
    "\n",
    include_str!("../static/js/pages/bots.js"),
    "\n",
    include_str!("../static/js/pages/scheduler.js"),
    "\n",
    include_str!("../static/js/pages/brain.js"),
    "\n",
    include_str!("../static/js/pages/security.js"),
    "\n",
    include_str!("../static/js/pages/sessions.js"),
    "\n",
    include_str!("../static/js/pages/logs.js"),
    "\n",
    include_str!("../static/js/pages/comms.js"),
    "\n",
    include_str!("../static/js/pages/mcp.js"),
    "\n",
    include_str!("../static/js/pages/tenants.js"),
    "\n</script>\n",
    // Alpine.js MUST be last — it processes x-data and fires alpine:init
    "<script>\n",
    include_str!("../static/vendor/alpine.min.js"),
    "\n</script>\n",
    "</body></html>"
);

/// Share page HTML — assembled at compile time with QR code library inlined.
const SHARE_HTML: &str = concat!(
    include_str!("../static/share.html"),
    "<script>\n",
    include_str!("../static/vendor/qrcode.min.js"),
    "\n</script>\n",
    "</body>\n</html>"
);
