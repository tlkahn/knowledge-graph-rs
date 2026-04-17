mod cli;
mod envelope;

use std::io::Write;
use std::path::PathBuf;
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
        Ok(()) => ExitCode::from(0),
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
            let env: Envelope<()> =
                Envelope::err("unknown_subcommand", first_line(&err.to_string()));
            emit_stdout(&env);
            ExitCode::from(2)
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

fn dispatch(cli: Cli) -> Result<(), kg_core::Error> {
    match cli.command {
        Command::Parse { pretty } => cmd_parse(cli.vault, pretty),
    }
}

fn cmd_parse(vault: Option<PathBuf>, pretty: bool) -> Result<(), kg_core::Error> {
    let vault_path = vault.ok_or_else(|| kg_core::Error::VaultNotFound {
        path: "(provide --vault or set KG_VAULT_PATH)".into(),
    })?;

    let events = kg_core::parser::parse_vault(&vault_path)?;

    if pretty {
        let wrapper = serde_json::json!({
            "ok": true,
            "data": events,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&wrapper).expect("serialize")
        );
    } else {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        for event in &events {
            serde_json::to_writer(&mut out, event).expect("serialize");
            let _ = writeln!(out);
        }
    }

    Ok(())
}
