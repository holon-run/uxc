//! OpenAPI test server binary for E2E testing

fn main() -> anyhow::Result<()> {
    // Use tokio runtime
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(uxc::test_server::openapi::main())
}
