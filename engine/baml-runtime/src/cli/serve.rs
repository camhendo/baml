use anyhow::{Context, Result};
use axum::{
    extract,
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use axum_streams::*;
use baml_types::BamlValue;
use core::pin::Pin;
use futures::Stream;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{convert::Infallible, time::Duration};
use std::{path::PathBuf, sync::Arc, task::Poll};

use crate::{BamlRuntime, FunctionResult, RuntimeContextManager};

#[derive(clap::Args, Clone, Debug)]
pub struct ServeArgs {
    #[arg(long, help = "path/to/baml_src", default_value = "./baml_src")]
    from: String,
    #[arg(long, help = "port to expose BAML on", default_value = "2024")]
    port: u16,
    #[arg(
        long,
        help = "Generate baml_client without checking for version mismatch",
        default_value_t = false
    )]
    no_version_check: bool,
}

impl ServeArgs {
    pub fn run(&self) -> Result<()> {
        let server = Arc::new(Server {
            args: self.clone(),
            t: tokio::runtime::Runtime::new()?,
            b: BamlRuntime::from_directory(&PathBuf::from(&self.from), std::env::vars().collect())?,
        });

        server.serve()
    }
}
struct Server {
    args: ServeArgs,
    t: tokio::runtime::Runtime,
    b: BamlRuntime,
}

impl Server {
    pub fn serve(self: Arc<Self>) -> Result<()> {
        // build our application with a route
        let app = axum::Router::new();

        let s = self.clone();
        let app = app.route("/echo/:msg", get(move |path| s.echo(path)));

        let s = self.clone();
        let app = app.route("/sse/:msg", get(move |path| sse_handler(path)));

        let s = self.clone();
        let app = app.route(
            "/call/:msg",
            post(move |b_fn, b_args| s.baml_call_axum(b_fn, b_args)),
        );

        let s = self.clone();
        let app = app.route(
            "/stream/:msg",
            post(move |b_fn, b_args| s.baml_stream_axum2(b_fn, b_args)),
        );

        let s = self.clone();
        let app = app.route(
            "/stream_nd/:msg",
            post(move |b_fn, b_args| s.baml_stream_axum(b_fn, b_args)),
        );

        let s = self.clone();

        self.t.block_on(async move {
            let listener =
                tokio::net::TcpListener::bind(format!("0.0.0.0:{}", s.args.port)).await?;
            axum::serve(listener, app).await?;

            Ok(())
        })
    }

    fn echo_stream(self: Arc<Self>, msg: String) -> Pin<Box<dyn Stream<Item = String> + Send>> {
        use tokio_stream::StreamExt;
        Box::pin(
            futures::stream::iter(std::iter::repeat(msg).take(3))
                .throttle(std::time::Duration::from_secs(1)),
        )
    }

    async fn echo(
        self: Arc<Self>,
        axum::extract::Path(msg): axum::extract::Path<String>,
    ) -> impl IntoResponse {
        log::info!("stream_jsonnl0 called {}", msg);
        StreamBodyAs::json_nl(self.echo_stream(msg))
    }

    async fn baml_call(
        self: Arc<Self>,
        b_fn: String,
        b_args: serde_json::Value,
    ) -> Result<BamlValue> {
        let args: BamlValue = serde_json::from_value(b_args)
            .context("Failed to convert arguments from JSON to BamlValue")?;

        let args = args
            .as_map()
            .context("Arguments provided must be a map, from arg name to value")?;

        let ctx_mgr = RuntimeContextManager::new_from_env_vars(std::env::vars().collect(), None);

        let (result, trace_id) = self
            .b
            .call_function(b_fn, &args, &ctx_mgr, None, None)
            .await;

        result?
            .parsed_content()
            .context("Failed to parse result")
            .map(|v| v.into())
    }

    async fn baml_call_axum(
        self: Arc<Self>,
        extract::Path(b_fn): extract::Path<String>,
        extract::Json(b_args): extract::Json<serde_json::Value>,
    ) -> impl IntoResponse {
        match self.baml_call(b_fn, b_args).await {
            Ok(result) => (StatusCode::OK, Json(result)),
            Err(err) => {
                log::error!("Error calling BAML function: {:?}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::from_value(json!({"error": err.to_string()})).unwrap()),
                )
            }
        }
    }

    fn baml_stream(
        self: Arc<Self>,
        b_fn: String,
        b_args: serde_json::Value,
    ) -> Pin<Box<dyn Stream<Item = BamlValue> + Send>> {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();

        tokio::spawn(async move {
            let args: BamlValue = serde_json::from_value(b_args)
                .context("Failed to convert arguments from JSON to BamlValue")?;

            let args = args
                .as_map()
                .context("Arguments provided must be a map, from arg name to value")?;

            let ctx_mgr =
                RuntimeContextManager::new_from_env_vars(std::env::vars().collect(), None);

            let mut result_stream = self.b.stream_function(b_fn, &args, &ctx_mgr, None, None)?;

            let (result, trace_id) = result_stream
                .run(
                    Some(move |result| {
                        log::info!("on_event sending result");
                        sender.send(result);
                    }),
                    &ctx_mgr,
                    None,
                    None,
                )
                .await;

            result
        });

        let event_stream = EventStream { receiver };

        Box::pin(event_stream)
    }

    async fn baml_stream_axum(
        self: Arc<Self>,
        extract::Path(path): extract::Path<String>,
        extract::Json(body): extract::Json<serde_json::Value>,
    ) -> impl IntoResponse {
        StreamBodyAs::json_nl(self.baml_stream(path, body))
    }

    async fn baml_stream_axum2(
        self: Arc<Self>,
        extract::Path(path): extract::Path<String>,
        extract::Json(body): extract::Json<serde_json::Value>,
    ) -> Sse<impl Stream<Item = Result<Event, axum::Error>>> {
        use tokio_stream::StreamExt;
        // A `Stream` that repeats an event every second
        let stream = self
            .baml_stream(path, body)
            .map(|bv| Event::default().json_data(bv));

        Sse::new(stream).keep_alive(KeepAlive::default())
    }
}

async fn sse_handler(
    extract::Path(msg): extract::Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    use tokio_stream::StreamExt;
    // A `Stream` that repeats an event every second
    let stream =
        futures::stream::repeat(Event::default().data(format!("<message>{msg}</message>")))
            .take(10)
            .map(Ok)
            .throttle(Duration::from_secs(1));

    Sse::new(stream).keep_alive(KeepAlive::default())
}

struct EventStream {
    receiver: tokio::sync::mpsc::UnboundedReceiver<FunctionResult>,
}

impl Stream for EventStream {
    type Item = BamlValue;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match self.receiver.poll_recv(cx) {
            Poll::Ready(Some(item)) => Poll::Ready(Some(
                item.parsed().as_ref().unwrap().as_ref().unwrap().into(),
            )),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
