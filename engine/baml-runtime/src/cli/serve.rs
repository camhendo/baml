use anyhow::Result;
use aws_sdk_bedrockruntime::primitives::event_stream;
use axum::{
    http::StatusCode,
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
use std::{
    path::PathBuf,
    sync::Arc,
    task::{Context, Poll},
};

use crate::{BamlRuntime, FunctionResult, RuntimeContextManager};

#[derive(clap::Args, Clone, Debug)]
pub struct ServeArgs {
    #[arg(long, help = "path/to/baml_src", default_value = "./baml_src")]
    from: String,
    #[arg(
        long,
        help = "Generate baml_client without checking for version mismatch",
        default_value_t = false
    )]
    no_version_check: bool,
}

// TODO: wasm32 bail-out

impl ServeArgs {
    pub fn run(&self) -> Result<()> {
        let baml_runtime = Arc::new(
            BamlRuntime::from_directory(
                &PathBuf::from("/Users/sam/baml/integ-tests/baml_src"),
                std::env::vars().collect(),
            )
            .unwrap(),
        );

        let clone = self.clone();
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async move {
            clone.run_async(baml_runtime.clone()).await;
        });
        Ok(())
    }

    pub async fn run_async(&self, obaml_runtime: Arc<BamlRuntime>) -> Result<()> {
        // build our application with a route
        let app = Router::new();
        let baml_runtime = obaml_runtime.clone();
        let app = app.route(
            "/message/:msg",
            get(|path| stream_jsonnl0(baml_runtime, path)),
        );
        let baml_runtime = obaml_runtime.clone();
        let app = app.route(
            "/stream_jsonnl/:msg",
            get(|path| stream_jsonnl(baml_runtime, path)),
        );

        // run our app with hyper, listening globally on port 3000
        let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
        axum::serve(listener, app).await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
struct Message {
    msg: String,
}

fn create_message_stream0(
    baml_runtime: Arc<BamlRuntime>,
    msg: String,
) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
    use tokio_stream::StreamExt;
    Box::pin(
        futures::stream::iter(std::iter::repeat(Message { msg }).take(10))
            .throttle(std::time::Duration::from_secs(1)),
    )
}
async fn stream_jsonnl0(
    baml_runtime: Arc<BamlRuntime>,
    axum::extract::Path(msg): axum::extract::Path<String>,
) -> impl IntoResponse {
    log::info!("stream_jsonnl0 called {}", msg);
    StreamBodyAs::json_nl(create_message_stream0(baml_runtime, msg))
}

fn create_message_stream(
    baml_runtime: Arc<BamlRuntime>,
    msg: String,
) -> Pin<Box<dyn Stream<Item = BamlValue> + Send>> {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();

    log::info!("create_message_stream created {}", msg);
    tokio::spawn(async move {
        let mut args = IndexMap::new();
        args.insert("input".to_string(), BamlValue::String(msg));
        let ctx_mgr = RuntimeContextManager::new_from_env_vars(std::env::vars().collect(), None);

        let mut result_stream = baml_runtime
            .stream_function(
                "PromptTestStreaming".to_string(),
                &args,
                &ctx_mgr,
                None,
                None,
            )
            .unwrap();
        log::info!("result_stream created");

        result_stream
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
    });

    let event_stream = EventStream { receiver };

    Box::pin(event_stream)

    // use tokio_stream::StreamExt;
    // Box::pin(
    //     futures::stream::iter(std::iter::repeat(Message { msg }).take(10))
    //         .throttle(std::time::Duration::from_secs(1)),
    // )
}

async fn stream_jsonnl(
    baml_runtime: Arc<BamlRuntime>,
    axum::extract::Path(msg): axum::extract::Path<String>,
) -> impl IntoResponse {
    log::info!("stream_jsonnl called {}", msg);
    StreamBodyAs::json_nl(create_message_stream(baml_runtime, msg))
}

struct EventStream {
    receiver: tokio::sync::mpsc::UnboundedReceiver<FunctionResult>,
}

impl Stream for EventStream {
    type Item = BamlValue;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.receiver.poll_recv(cx) {
            Poll::Ready(Some(item)) => Poll::Ready(Some(
                item.parsed().as_ref().unwrap().as_ref().unwrap().into(),
            )),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
