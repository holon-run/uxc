//! MCP stdio test server binary for E2E testing

fn main() -> anyhow::Result<()> {
    uxc::test_server::mcp_stdio::main()
}
