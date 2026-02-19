use clap::{Parser as ClapParser, Subcommand};
use std::path::PathBuf;

#[derive(ClapParser, Debug)]
#[command(name = "govm", about = "Go UDF compiler targeting WebAssembly")]
struct Cli {
    #[clap(subcommand)]
    action: SubCommand,
}

#[derive(Subcommand, Debug)]
enum SubCommand {
    #[clap(name = "compile", about = "Compile Go UDF source to WebAssembly")]
    Compile {
        source: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.action {
        SubCommand::Compile { source, output } => {
            let out_path = output.unwrap_or_else(|| {
                let mut p = source.clone();
                p.set_extension("wasm");
                p
            });
            println!("compiling {:?} -> {:?}", source, out_path);
            println!("WASM compilation not yet implemented");
        }
    }
}
