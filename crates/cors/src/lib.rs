//! Cross-Origin Resource Sharing (CORS) support for Savlo web server framework.
//!
//! [CORS]: https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS
//!
//! # Example
//!
//! ```
//! use salvo_core::http::Method;
//! use salvo_core::prelude::*;
//! use salvo_cors::Cors;
//!
//! let cors_handler = Cors::new()
//!     .allow_origin("https://salvo.rs")
//!     .allow_methods(vec![Method::GET, Method::POST, Method::DELETE]).into_handler();
//!
//! let router = Router::new().hoop(cors_handler).post(upload_file).options(upload_file);
//! #[handler]
//! async fn upload_file(res: &mut Response) {
//! }
//!
//! ```
//! If you want to allow any router:
//! ```
//! use salvo_core::prelude::*;
//! use salvo_cors::{self as cors, Cors};
//! let cors_handler = Cors::new().allow_origin(cors::Any).into_handler();
//! ```
//!
//! Read more: <https://salvo.rs>
#![doc(html_favicon_url = "https://salvo.rs/favicon-32x32.png")]
#![doc(html_logo_url = "https://salvo.rs/images/logo.svg")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(unreachable_pub)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::future_not_send)]
#![warn(rustdoc::broken_intra_doc_links)]

use bytes::{BufMut, BytesMut};
use salvo_core::http::header::{self, HeaderMap, HeaderName, HeaderValue};
use salvo_core::http::{Method, Request, Response, StatusCode};
use salvo_core::{async_trait, Depot, FlowCtrl, Handler};

mod allow_credentials;
mod allow_headers;
mod allow_methods;
mod allow_origin;
mod expose_headers;
mod max_age;
mod vary;

pub use self::{
    allow_credentials::AllowCredentials, allow_headers::AllowHeaders, allow_methods::AllowMethods,
    allow_origin::AllowOrigin, expose_headers::ExposeHeaders, max_age::MaxAge, vary::Vary,
};

static WILDCARD: HeaderValue = HeaderValue::from_static("*");

/// Represents a wildcard value (`*`) used with some CORS headers such as
/// [`Cors::allow_methods`].
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct Any;

fn separated_by_commas<I>(mut iter: I) -> Option<HeaderValue>
where
    I: Iterator<Item = HeaderValue>,
{
    match iter.next() {
        Some(fst) => {
            let mut result = BytesMut::from(fst.as_bytes());
            for val in iter {
                result.reserve(val.len() + 1);
                result.put_u8(b',');
                result.extend_from_slice(val.as_bytes());
            }

            Some(HeaderValue::from_maybe_shared(result.freeze()).unwrap())
        }
        None => None,
    }
}

/// [`Cors`] middleware which adds headers for [CORS][mdn].
///
/// [mdn]: https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS
#[derive(Clone, Debug)]
pub struct Cors {
    allow_credentials: AllowCredentials,
    allow_headers: AllowHeaders,
    allow_methods: AllowMethods,
    allow_origin: AllowOrigin,
    expose_headers: ExposeHeaders,
    max_age: MaxAge,
    vary: Vary,
}
impl Default for Cors {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Cors {
    /// Create new `Cors`.
    #[inline]
    pub fn new() -> Self {
        Cors {
            allow_credentials: Default::default(),
            allow_headers: Default::default(),
            allow_methods: Default::default(),
            allow_origin: Default::default(),
            expose_headers: Default::default(),
            max_age: Default::default(),
            vary: Default::default(),
        }
    }

    /// A permissive configuration:
    ///
    /// - All request headers allowed.
    /// - All methods allowed.
    /// - All origins allowed.
    /// - All headers exposed.
    pub fn permissive() -> Self {
        Self::new()
            .allow_headers(Any)
            .allow_methods(Any)
            .allow_origin(Any)
            .expose_headers(Any)
    }

    /// A very permissive configuration:
    ///
    /// - **Credentials allowed.**
    /// - The method received in `Access-Control-Request-Method` is sent back
    ///   as an allowed method.
    /// - The origin of the preflight request is sent back as an allowed origin.
    /// - The header names received in `Access-Control-Request-Headers` are sent
    ///   back as allowed headers.
    /// - No headers are currently exposed, but this may change in the future.
    pub fn very_permissive() -> Self {
        Self::new()
            .allow_credentials(true)
            .allow_headers(AllowHeaders::mirror_request())
            .allow_methods(AllowMethods::mirror_request())
            .allow_origin(AllowOrigin::mirror_request())
    }

    /// Sets whether to add the `Access-Control-Allow-Credentials` header.
    #[inline]
    pub fn allow_credentials(mut self, allow_credentials: impl Into<AllowCredentials>) -> Self {
        self.allow_credentials = allow_credentials.into();
        self
    }

    /// Adds multiple headers to the list of allowed request headers.
    ///
    /// **Note**: These should match the values the browser sends via `Access-Control-Request-Headers`, e.g.`content-type`.
    ///
    /// # Panics
    ///
    /// Panics if any of the headers are not a valid `http::header::HeaderName`.
    #[inline]
    pub fn allow_headers(mut self, headers: impl Into<AllowHeaders>) -> Self {
        self.allow_headers = headers.into();
        self
    }

    /// Sets the `Access-Control-Max-Age` header.
    ///
    /// # Example
    ///
    ///
    /// ```
    /// use std::time::Duration;
    /// use salvo_core::prelude::*;
    /// use salvo_cors::Cors;
    ///
    /// let cors = Cors::new().max_age(30); // 30u32 seconds
    /// let cors = Cors::new().max_age(Duration::from_secs(30)); // or a Duration
    /// ```
    #[inline]
    pub fn max_age(mut self, max_age: impl Into<MaxAge>) -> Self {
        self.max_age = max_age.into();
        self
    }

    /// Adds multiple methods to the existing list of allowed request methods.
    ///
    /// # Panics
    ///
    /// Panics if the provided argument is not a valid `http::Method`.
    #[inline]
    pub fn allow_methods<I>(mut self, methods: I) -> Self
    where
        I: Into<AllowMethods>,
    {
        self.allow_methods = methods.into();
        self
    }

    /// Set the value of the [`Access-Control-Allow-Origin`][mdn] header.
    ///
    /// [mdn]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Access-Control-Allow-Origin
    #[inline]
    pub fn allow_origin(mut self, origin: impl Into<AllowOrigin>) -> Self {
        self.allow_origin = origin.into();
        self
    }

    /// Set the value of the [`Access-Control-Expose-Headers`][mdn] header.
    ///
    /// [mdn]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Access-Control-Expose-Headers
    #[inline]
    pub fn expose_headers(mut self, headers: impl Into<ExposeHeaders>) -> Self {
        self.expose_headers = headers.into();
        self
    }

    /// Set the value(s) of the [`Vary`][mdn] header.
    ///
    /// In contrast to the other headers, this one has a non-empty default of
    /// [`preflight_request_headers()`].
    ///
    /// You only need to set this is you want to remove some of these defaults,
    /// or if you use a closure for one of the other headers and want to add a
    /// vary header accordingly.
    ///
    /// [mdn]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Vary
    pub fn vary<T>(mut self, headers: impl Into<Vary>) -> Self {
        self.vary = headers.into();
        self
    }

    /// Returns a new `CorsHandler` using current cors settings.
    pub fn into_handler(self) -> CorsHandler {
        self.ensure_usable_cors_rules();
        CorsHandler(self)
    }

    fn ensure_usable_cors_rules(&self) {
        if self.allow_credentials.is_true() {
            assert!(
                !self.allow_headers.is_wildcard(),
                "Invalid CORS configuration: Cannot combine `Access-Control-Allow-Credentials: true` \
                 with `Access-Control-Allow-Headers: *`"
            );

            assert!(
                !self.allow_methods.is_wildcard(),
                "Invalid CORS configuration: Cannot combine `Access-Control-Allow-Credentials: true` \
                 with `Access-Control-Allow-Methods: *`"
            );

            assert!(
                !self.allow_origin.is_wildcard(),
                "Invalid CORS configuration: Cannot combine `Access-Control-Allow-Credentials: true` \
                 with `Access-Control-Allow-Origin: *`"
            );

            assert!(
                !self.expose_headers.is_wildcard(),
                "Invalid CORS configuration: Cannot combine `Access-Control-Allow-Credentials: true` \
                 with `Access-Control-Expose-Headers: *`"
            );
        }
    }
}

/// CorsHandler
#[derive(Clone, Debug)]
pub struct CorsHandler(Cors);

#[async_trait]
impl Handler for CorsHandler {
    async fn handle(&self, req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
        let origin = req.headers().get(&header::ORIGIN);

        let mut headers = HeaderMap::new();

        // These headers are applied to both preflight and subsequent regular CORS requests:
        // https://fetch.spec.whatwg.org/#http-responses

        headers.extend(self.0.allow_origin.to_header(origin, req, depot));
        headers.extend(self.0.allow_credentials.to_header(origin, req, depot));

        let mut vary_headers = self.0.vary.values();
        if let Some(first) = vary_headers.next() {
            let mut header = match headers.entry(header::VARY) {
                header::Entry::Occupied(_) => {
                    unreachable!("no vary header inserted up to this point")
                }
                header::Entry::Vacant(v) => v.insert_entry(first),
            };

            for val in vary_headers {
                header.append(val);
            }
        }

        // Return results immediately upon preflight request
        if req.method() == Method::OPTIONS {
            // These headers are applied only to preflight requests
            headers.extend(self.0.allow_methods.to_header(origin, req, depot));
            headers.extend(self.0.allow_headers.to_header(origin, req, depot));
            headers.extend(self.0.max_age.to_header(origin, req, depot));
            res.status_code = Some(StatusCode::NO_CONTENT);
        } else {
            // This header is applied only to non-preflight requests
            headers.extend(self.0.expose_headers.to_header(origin, req, depot));
        }
        res.headers_mut().extend(headers);
        ctrl.call_next(req, depot, res).await;
    }
}

/// Returns an iterator over the three request headers that may be involved in a CORS preflight request.
///
/// This is the default set of header names returned in the `vary` header
pub fn preflight_request_headers() -> impl Iterator<Item = HeaderName> {
    [
        header::ORIGIN,
        header::ACCESS_CONTROL_REQUEST_METHOD,
        header::ACCESS_CONTROL_REQUEST_HEADERS,
    ]
    .into_iter()
}

#[cfg(test)]
mod tests {
    use salvo_core::http::header::*;
    use salvo_core::prelude::*;
    use salvo_core::test::TestClient;

    use super::*;

    #[tokio::test]
    async fn test_cors() {
        let cors_handler = Cors::new()
            .allow_origin("https://salvo.rs")
            .allow_methods(vec![Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers(vec![
                "CONTENT-TYPE",
                "Access-Control-Request-Method",
                "Access-Control-Allow-Origin",
                "Access-Control-Allow-Headers",
                "Access-Control-Max-Age",
            ])
            .into_handler();

        #[handler]
        async fn hello() -> &'static str {
            "hello"
        }

        let router = Router::new()
            .hoop(cors_handler)
            .push(Router::with_path("hello").goal(hello));
        let service = Service::new(router);

        async fn options_access(service: &Service, origin: &str) -> Response {
            TestClient::options("http://127.0.0.1:5801/hello")
                .add_header("Origin", origin, true)
                .add_header("Access-Control-Request-Method", "POST", true)
                .add_header("Access-Control-Request-Headers", "Content-Type", true)
                .send(service)
                .await
        }

        let res = TestClient::options("https://salvo.rs").send(&service).await;
        assert!(res.headers().get(ACCESS_CONTROL_ALLOW_METHODS).is_none());

        let res = options_access(&service, "https://salvo.rs").await;
        let headers = res.headers();
        assert!(headers.get(ACCESS_CONTROL_ALLOW_METHODS).is_some());
        assert!(headers.get(ACCESS_CONTROL_ALLOW_HEADERS).is_some());

        let res = TestClient::options("https://google.com").send(&service).await;
        let headers = res.headers();
        assert!(
            headers.get(ACCESS_CONTROL_ALLOW_METHODS).is_none(),
            "POST, GET, DELETE, OPTIONS"
        );
        assert!(headers.get(ACCESS_CONTROL_ALLOW_HEADERS).is_none());
    }
}
