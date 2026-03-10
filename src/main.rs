// flowhkp - a proxy server connecting Flowcrypt's API to HKP
// Copyright (C) 2026 Sam Wilson
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::{Request, Response, StatusCode, body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use reqwest::Client;
use sequoia_openpgp::{Cert, parse::Parse};
use snafu::{Report, ResultExt, Snafu, Whatever};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{borrow::Borrow, collections::HashMap, net::SocketAddr};
use tokio::net::TcpListener;
use tracing::instrument;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Debug, Snafu, Clone)]
#[snafu(transparent)]
struct HttpError {
    source: Arc<reqwest::Error>,
}

impl PartialEq for HttpError {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source)
    }
}

impl Eq for HttpError {}

type Cache = quick_cache::sync::Cache<String, (Instant, Result<Bytes, HttpError>)>;

static USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[snafu::report]
#[tokio::main]
async fn main() -> Result<(), Whatever> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .with(tracing_journald::layer().whatever_context("couldn't connect to journald")?)
        .init();

    let proxy = Proxy::new()?;

    let addr = SocketAddr::from(([127, 0, 0, 1], 11371));

    let listener = TcpListener::bind(addr)
        .await
        .whatever_context("tcp bind failed")?;

    // We start a loop to continuously accept incoming connections
    loop {
        let (stream, addr) = listener
            .accept()
            .await
            .whatever_context("tcp accept failed")?;

        // Use an adapter to access something implementing `tokio::io` traits as if they implement
        // `hyper::rt` IO traits.
        let io = TokioIo::new(stream);

        // Spawn a tokio task to serve multiple connections concurrently
        let proxy_clone = proxy.clone();
        tokio::task::spawn(async move {
            // Finally, we bind the incoming connection to our `hello` service
            if let Err(err) = http1::Builder::new()
                // `service_fn` converts our function in a `Service`
                .serve_connection(io, service_fn(|r| proxy_clone.hello(r)))
                .await
            {
                tracing::warn!(error = %Report::from_error(err), %addr, "failed to serve connection");
            }
        });
    }
}

#[derive(Debug, Clone)]
struct Proxy {
    cache: Arc<Cache>,
    client: Client,
}

impl Proxy {
    fn new() -> Result<Self, Whatever> {
        Ok(Self {
            cache: Arc::new(Cache::new(10)),
            client: Client::builder()
                .user_agent(USER_AGENT)
                .build()
                .whatever_context("failed to create reqwest client")?,
        })
    }

    async fn hello(&self, request: Request<Incoming>) -> Result<Response<String>, Whatever> {
        match request.uri().path() {
            "/pks/lookup" => self.lookup(request).await,
            _ => Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Default::default())
                .unwrap()),
        }
    }

    async fn lookup(&self, request: Request<Incoming>) -> Result<Response<String>, Whatever> {
        let query = request.uri().query().unwrap_or_default();

        let query: HashMap<_, _> = url::form_urlencoded::parse(query.as_bytes()).collect();

        match query.get("op").map(|q| &**q) {
            Some("get") => self.get(query).await,
            Some("index") => self.index(query).await,
            _ => Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Default::default())
                .unwrap()),
        }
    }

    #[instrument(skip(self))]
    async fn fetch_cert(&self, search: &str) -> Result<Bytes, Whatever> {
        loop {
            let guard = match self.cache.get_value_or_guard_async(search).await {
                Ok((fetched_at, bytes)) if Instant::now() - fetched_at > Duration::from_mins(5) => {
                    tracing::debug!(?fetched_at, "cache expired");
                    self.cache.remove_if(search, |v| v == &(fetched_at, bytes));
                    continue;
                }
                Ok((_, result)) => {
                    tracing::debug!("cache hit");
                    break result.whatever_context("cached error");
                }
                Err(guard) => guard,
            };

            tracing::debug!("cache miss");

            let result = self
                .client
                .get(format!("https://flowcrypt.com/attester/pub/{search}"))
                .send()
                .await
                .and_then(|x| x.error_for_status());

            let response = match result {
                Ok(r) => r,
                Err(e) => {
                    let error = Err(HttpError::from(Arc::new(e)));
                    guard.insert((Instant::now(), error.clone())).ok();
                    break error.whatever_context("http error");
                }
            };

            let bytes = match response.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    let error = Err(HttpError::from(Arc::new(e)));
                    guard.insert((Instant::now(), error.clone())).ok();
                    break error.whatever_context("http body error");
                }
            };

            let result = Ok(bytes);
            guard.insert((Instant::now(), result.clone())).ok();

            break result.whatever_context("http body error");
        }
    }

    async fn index<S>(&self, query: HashMap<S, S>) -> Result<Response<String>, Whatever>
    where
        S: Borrow<str> + Eq + std::hash::Hash,
    {
        let search = match query.get("search") {
            Some(e) => e.borrow(),
            None => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Default::default())
                    .unwrap());
            }
        };

        let packets = match self.fetch_cert(search).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %Report::from_error(&e), "failed to fetch certificate");
                // TODO: Correctly forward status codes (eg. 404 should stay 404).
                return Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(e.to_string())
                    .whatever_context("unable to format bad gateway response");
            }
        };

        let now = Instant::now();
        let cert = Cert::from_bytes(&packets).whatever_context("unable to parse certificate")?;

        let pk = cert.primary_key().key();

        let keyid = cert.keyid().to_hex();
        let algo: u8 = pk.pk_algo().into();
        let bits = pk.mpis().bits().unwrap_or_default();

        let output = format!("info:1:1\npub:{keyid}:{algo}:{bits}:::\n");

        self.cache.insert(keyid, (now, Ok(packets)));

        Response::builder()
            .status(StatusCode::OK)
            .body(output)
            .whatever_context("unable to build response")
    }

    async fn get<S>(&self, query: HashMap<S, S>) -> Result<Response<String>, Whatever>
    where
        S: Borrow<str> + Eq + std::hash::Hash,
    {
        let search = match query.get("search") {
            Some(e) => e.borrow(),
            None => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Default::default())
                    .unwrap());
            }
        };

        let response = self
            .fetch_cert(search.strip_prefix("0x").unwrap_or(search))
            .await;

        let response = match response {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %Report::from_error(&e), "failed to fetch certificate");
                // TODO: Correctly forward status codes (eg. 404 should stay 404).
                return Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(e.to_string())
                    .whatever_context("unable to format bad gateway response");
            }
        };

        let response = std::str::from_utf8(&response).whatever_context("invalid utf8")?;

        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/pgp-keys")
            .body(response.to_owned())
            .whatever_context("write body")
    }
}
