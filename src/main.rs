pub mod parser;
pub mod vm;

use crate::parser::{parse_dir_recursive, Parser};
use crate::vm::compiler::compiler::Compiler;
use crate::vm::module::parse_dependencies;
use crate::vm::VM;
use bdwgc_alloc::Allocator;
use clap::Args;
use clap::{Parser as ClapParser, Subcommand};
use std::env;
use std::path::PathBuf;

#[global_allocator]
static GLOBAL_ALLOCATOR: Allocator = Allocator;

#[derive(ClapParser, Debug)]
struct Cli {
    #[clap(short, long)]
    debug: Option<u8>,
    #[clap(subcommand)]
    action: SubCommand,
}

#[derive(Subcommand, Debug)]
enum SubCommand {
    #[clap(name = "build", about = "A Go virtual machine")]
    Build { source: PathBuf },
    #[clap(name = "run", about = "Builds and runs a Go binary")]
    Run { binary: PathBuf },
    #[clap(name = "mod", about = "Manage Go modules")]
    Mod(ModArg),
}

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
struct ModArg {
    #[command(subcommand)]
    command: ModSubCommand,
}

#[derive(Debug, Subcommand)]
enum ModSubCommand {
    #[clap(name = "init", about = "Initialize a new module")]
    Init { module_name: Option<String> },
    #[clap(name = "add", about = "Add a new dependency")]
    Add { dependency: String },
    #[clap(name = "list", about = "List dependencies")]
    List,
    #[clap(name = "remove", about = "Remove a dependency")]
    Remove { dependency: String },
}

fn main() {
    unsafe { Allocator::initialize() }

    let cli = Cli::parse();
    //println!("{:?}", cli);

    match cli.action {
        SubCommand::Build { source } => {
            unimplemented!("Building project from source: {:?}", source);
        }
        SubCommand::Run { binary } => {
            println!("Running binary: {:?}", binary);
            let pkgs = parse_dependencies(&binary).unwrap();

            let mut goc = Compiler::new();
            let code = goc.compile(pkgs).unwrap();
            let mut vm = VM::new();

            vm.run(code).unwrap();
        }
        SubCommand::Mod(mod_command) => match mod_command.command {
            ModSubCommand::Init { module_name } => {
                let name = if let Some(mn) = module_name {
                    mn
                } else if let Ok(cd) = env::current_dir().map(|a| format!("{}", a.display())) {
                    cd
                } else {
                    panic!("Failed to get the current directory.");
                };
                // init name
            }
            ModSubCommand::Add { dependency } => {
                println!("Adding a new dependency: {}", dependency);
            }
            ModSubCommand::List => {
                println!("Listing dependencies:");
                // Implement listing dependencies logic
            }
            ModSubCommand::Remove { dependency } => {
                println!("Removing dependency: {}", dependency);
                // Implement removing dependency logic
            }
        },
    }
}
