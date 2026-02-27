//! gRPC test server for E2E testing

use super::common::{write_addr_file, Scenario, ServerHandle};
use anyhow::Result;
use tokio::signal::ctrl_c;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, Response, Status};
use tracing::info;

pub mod addsvc {
    include!("proto/addsvc.rs");
    pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!("proto/addsvc_descriptor.bin");
}

#[derive(Clone, Copy)]
struct AddService {
    scenario: Scenario,
}

#[tonic::async_trait]
impl addsvc::add_server::Add for AddService {
    async fn sum(
        &self,
        request: Request<addsvc::SumRequest>,
    ) -> std::result::Result<Response<addsvc::SumReply>, Status> {
        match self.scenario {
            Scenario::Ok => {
                let req = request.into_inner();
                Ok(Response::new(addsvc::SumReply { v: req.a + req.b }))
            }
            Scenario::AuthRequired => Err(Status::unauthenticated("authentication required")),
            Scenario::Malformed => Err(Status::internal("malformed response")),
            Scenario::Timeout => {
                tokio::time::sleep(super::common::timeout_duration()).await;
                Err(Status::deadline_exceeded("request timed out"))
            }
        }
    }
}

pub async fn run(scenario: Scenario) -> Result<ServerHandle> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    info!("gRPC test server listening on {}", addr);
    write_addr_file(addr, "grpc")?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let add_service = addsvc::add_server::AddServer::new(AddService { scenario });

    let server = tonic::transport::Server::builder().add_service(add_service);

    let server = if matches!(scenario, Scenario::AuthRequired) {
        server
    } else {
        let reflection = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(addsvc::FILE_DESCRIPTOR_SET)
            .build()?;
        server.add_service(reflection)
    };

    tokio::spawn(async move {
        if let Err(err) = server
            .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async {
                let _ = shutdown_rx.await;
            })
            .await
        {
            tracing::error!("gRPC test server failed: {}", err);
        }
    });

    Ok(ServerHandle {
        addr,
        shutdown: shutdown_tx,
    })
}

pub async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let scenario = if args.len() > 1 {
        Scenario::from_str(&args[1])?
    } else {
        Scenario::Ok
    };

    tracing_subscriber::fmt()
        .with_env_filter("uxc_test_server=info,tonic=info")
        .init();

    let handle = run(scenario).await?;

    ctrl_c().await?;
    let _ = handle.shutdown.send(());

    Ok(())
}
