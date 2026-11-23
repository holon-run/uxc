use uxc::mcp::{ListEventsParams, McpClient, McpConnectOptions, McpEndpoint};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = std::env::args()
        .nth(1)
        .ok_or("pass the path to the MCP stdio fixture")?;
    let mut client = McpClient::connect(McpConnectOptions::new(McpEndpoint::Stdio {
        program,
        args: Vec::new(),
        env_overrides: Default::default(),
    }))
    .await?;
    let result = async {
        let discovery = client.discover().await?;
        println!("Protocol: {}", discovery.protocol_version);
        let server = discovery
            .server_info
            .ok_or("fixture did not report its name")?;
        assert_eq!(server.name, "uxc-test-mcp-stdio");
        let tools = client.list_tools().await?;
        assert!(tools.iter().any(|tool| tool.name == "echo"));
        if discovery.capabilities.events {
            let page = client.list_events_page(ListEventsParams::default()).await?;
            println!("Event descriptors: {}", page.events.len());
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    }
    .await;
    let closed = client.close().await;
    result?;
    closed?;
    Ok(())
}
