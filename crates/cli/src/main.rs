mod cli;
mod envelope;

use std::process::ExitCode;

use clap::{Parser, error::ErrorKind};
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};
use crate::envelope::{Envelope, emit_stdout};

fn main() -> ExitCode {
    init_tracing();

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => return handle_clap_error(err),
    };

    match dispatch(cli) {
        Ok(value) => {
            emit_stdout(&Envelope::ok(value));
            ExitCode::from(0)
        }
        Err(err) => {
            let env: Envelope<()> = Envelope::err_from(&err);
            emit_stdout(&env);
            ExitCode::from(1)
        }
    }
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
}

fn handle_clap_error(err: clap::Error) -> ExitCode {
    match err.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            print!("{err}");
            ExitCode::from(0)
        }
        _ => {
            // Suppress clap's native stderr render; emit envelope on stdout.
            let env: Envelope<()> = Envelope::err("unknown_subcommand", first_line(&err.to_string()));
            emit_stdout(&env);
            ExitCode::from(2)
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

fn dispatch(cli: Cli) -> Result<serde_json::Value, kg_core::Error> {
    match cli.command {
        Command::Parse {} => Err(kg_core::Error::NotImplemented {
            feature: "parse".into(),
        }),
    }
}
