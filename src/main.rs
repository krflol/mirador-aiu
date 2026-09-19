use clap::Parser;

#[derive(Parser)]
#[command(version, about = "AIU account usage as a Mirador protocol-v1 panel")]
struct Args {
    /// Show synthetic accounts without running AIU or reading credentials.
    #[arg(long)]
    demo: bool,
    /// Private subprocess mode used to let credential operations finish safely.
    #[arg(long, hide = true, conflicts_with = "demo")]
    worker: bool,
}

fn main() {
    let args = Args::parse();
    let result = if args.worker {
        mirador_aiu::backend::run_worker()
    } else {
        mirador_aiu::runtime::run(args.demo)
    };
    if let Err(error) = result
        && error.kind() != std::io::ErrorKind::BrokenPipe
    {
        // Do not echo input, AIU diagnostics, or command arguments.
        eprintln!("mirador-aiu: the protocol session ended with an error");
        std::process::exit(1);
    }
}
