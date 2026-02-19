use clap::{Parser as ClapParser, Subcommand};
use gno_rs::wasm::compiler::WasmCompiler;
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
            let wasm_path = output.unwrap_or_else(|| {
                let mut p = source.clone();
                p.set_extension("wasm");
                p
            });
            let manifest_path = {
                let mut p = wasm_path.clone();
                p.set_extension("json");
                p
            };

            let src = match std::fs::read_to_string(&source) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: failed to read {}: {}", source.display(), e);
                    std::process::exit(1);
                }
            };

            let mut compiler = WasmCompiler::new();
            let result = match compiler.compile_source(&src) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: compilation failed: {}", e);
                    std::process::exit(1);
                }
            };

            if let Err(e) = std::fs::write(&wasm_path, &result.wasm_bytes) {
                eprintln!("error: failed to write {}: {}", wasm_path.display(), e);
                std::process::exit(1);
            }

            let manifest_json = serde_json::to_string_pretty(&result.manifest)
                .expect("manifest serialization failed");
            if let Err(e) = std::fs::write(&manifest_path, manifest_json) {
                eprintln!("error: failed to write {}: {}", manifest_path.display(), e);
                std::process::exit(1);
            }

            println!(
                "compiled {} -> {} + {}",
                source.display(),
                wasm_path.display(),
                manifest_path.display()
            );
        }
    }
}
