
mod cli;
mod client;
mod protocol;
mod report;
mod server;
mod util;

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn main() {
    let args = match cli::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error: {}", e);
            cli::print_help();
            std::process::exit(1);
        }
    };

    if args.help {
        cli::print_help();
        return;
    }
    if args.version {
        println!("{}", protocol::VERSION);
        return;
    }

    // Валидация внутри parse() уже гарантирует, что server XOR client установлен,
    // но оставляем else-ветку как защиту от будущих изменений.
    let result: Res<()> = if args.server {
        server::run(args)
    } else if let Some(host) = args.client.clone() {
        client::run(args, host)
    } else {
        cli::print_help();
        std::process::exit(1);
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
