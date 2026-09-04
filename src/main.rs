mod cli;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    cli::run().await
}
