mod cli;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let mut command = Box::pin(cli::run());
    tokio::select! {
        code = &mut command => code,
        signal = stop_signal() => {
            let code = match signal {
                Ok(code) => code,
                Err(error) => {
                    eprintln!("Court Jester could not install interruption handling: {error}");
                    return command.await;
                }
            };
            drop(command);
            eprintln!("Court Jester interrupted; waiting for bounded container cleanup");
            if !court_jester::tools::sandbox::wait_for_docker_cleanup(std::time::Duration::from_secs(20)).await {
                eprintln!("Container cleanup remains unconfirmed after the shutdown deadline");
            }
            std::process::ExitCode::from(code)
        }
    }
}

async fn stop_signal() -> std::io::Result<u8> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result.map(|_| 130),
            _ = terminate.recv() => Ok(143),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.map(|_| 130)
    }
}
