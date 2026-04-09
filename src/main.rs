use court_jester_mcp::CourtJester;
use rmcp::ServiceExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = CourtJester::new();
    let (stdin, stdout) = rmcp::transport::stdio();
    server.serve((stdin, stdout)).await?.waiting().await?;
    Ok(())
}
