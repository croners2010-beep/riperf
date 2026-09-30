
mod cli;
mod client;
mod protocol;
mod report;
mod server;
mod util;

use std::process::ExitCode;

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn main() -> ExitCode {
    let args = match cli::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error: {}", e);
            return ExitCode::FAILURE;
        }
    };

    if args.help {
        cli::print_help();
        return ExitCode::SUCCESS;
    }

    if args.version {
        println!("{}", protocol::VERSION);
        return ExitCode::SUCCESS;
    }

    let result: Res<()> = if args.server {
        server::run(args)
    } else if let Some(host) = args.client.clone() {
        client::run(args, host)
    } else {
        cli::print_help();
        return ExitCode::FAILURE;
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}
