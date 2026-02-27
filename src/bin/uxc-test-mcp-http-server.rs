//! MCP HTTP test server binary for E2E testing

fn main() -> anyhow::Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(uxc::test_server::mcp_http::main())
}
