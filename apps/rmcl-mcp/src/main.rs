use std::{error::Error, path::PathBuf, sync::Arc};

use clap::{Parser, Subcommand};
use rmcl_app::AppContext;
use tokio_util::sync::CancellationToken;

const DEFAULT_PORT: u16 = 39271;

#[derive(Debug, Parser)]
#[command(name = "rmcl-mcp", about = "Expose RMCL instances and tasks over MCP")]
struct Args {
    /// Launcher configuration file; defaults to the platform RMCL path.
    #[arg(long, value_name = "PATH", global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve MCP over standard input and output.
    Stdio,
    /// Serve MCP over Streamable HTTP on loopback.
    Serve {
        /// Loopback TCP port to listen on.
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    if let Some(Command::Serve { port: 0 }) = &args.command {
        return Err("MCP port must be between 1 and 65535".into());
    }
    let context = match args.config {
        Some(path) => AppContext::from_config_path(path)?,
        None => AppContext::from_default_config()?,
    };
    let runtime = context.runtime.handle().clone();
    let result = runtime.block_on(async {
        match args.command.unwrap_or(Command::Stdio) {
            Command::Stdio => rmcl_mcp::serve_stdio(context.clone()).await?,
            Command::Serve { port } => {
                eprintln!("RMCL MCP listening on http://127.0.0.1:{port}/mcp");
                rmcl_mcp::serve_http(context.clone(), port, CancellationToken::new()).await?;
            }
        }
        Ok::<(), Box<dyn Error>>(())
    });
    if let Ok(context) = Arc::try_unwrap(context) {
        context.runtime.shutdown_background();
    }
    result
}
