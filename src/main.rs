pub mod parser;
pub mod vm;

use crate::parser::{parse_dir_recursive, Parser};
use crate::vm::compiler::compiler::Compiler;
use crate::vm::module::parse_local_dependencies;
use crate::vm::VM;
use bdwgc_alloc::Allocator;
use clap::Args;
use clap::{Parser as ClapParser, Subcommand};
use gno_rs::gomod::{add_dependency, list_dependencies, remove_dependency};
use std::env;
use std::fs::File;
use std::io::Write;
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
    Build {
        #[arg(short, long)]
        output_assert: bool,
        source: PathBuf,
    },
    #[clap(name = "run", about = "Builds and runs a Go binary")]
    Run {
        #[arg(short, long)]
        output_assert: bool,
        binary: PathBuf,
    },
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
        SubCommand::Build {
            output_assert,
            source,
        } => {
            unimplemented!("Building project from source: {:?}", source);
        }
        SubCommand::Run {
            output_assert,
            mut binary,
        } => {
            //println!("Running binary: {:?}", binary);
            let pkgs = parse_local_dependencies(&binary).unwrap();

            let mut goc = Compiler::new();
            let main = binary.clone();
            binary.pop();

            let code = goc.compile(main, binary, pkgs, output_assert).unwrap();
            let mut vm = VM::new();

            vm.run(code).unwrap();
        }
        SubCommand::Mod(mod_command) => match mod_command.command {
            ModSubCommand::Init { module_name } => {
                let name = module_name.unwrap_or_else(|| {
                    env::current_dir()
                        .ok()
                        .as_ref()
                        .and_then(|cd| cd.file_name())
                        .and_then(|name| name.to_str())
                        .and_then(|a| Some(a.to_string()))
                        .expect("no module name")
                });

                let go_mod_content = format!("module {}\n", name);

                if let Ok(cd) = env::current_dir() {
                    let go_mod_path = cd.join("go.mod");
                    let mut file = File::create(go_mod_path).expect("Failed to create go.mod");
                    file.write_all(go_mod_content.as_bytes())
                        .expect("Failed to write to go.mod");
                } else {
                    panic!("Failed to get the current directory.");
                }
            }
            ModSubCommand::Add { dependency } => {
                add_dependency(&dependency).unwrap();
            }
            ModSubCommand::List => {
                list_dependencies().unwrap();
            }
            ModSubCommand::Remove { dependency } => {
                remove_dependency(&dependency).unwrap();
            }
        },
    }
}
