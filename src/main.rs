pub mod parser;
pub mod vm;

use crate::vm::compiler::bytecode::{deserialize_bytecode, serialize_bytecode};
use crate::vm::compiler::compiler::Compiler;
use crate::vm::module::parse_local_dependencies;
use crate::vm::VM;
use bdwgc_alloc::Allocator;
use clap::Args;
use clap::{Parser as ClapParser, Subcommand};
use gno_rs::gomod::{add_dependency, list_dependencies, remove_dependency};
use std::env;
use std::fs::File;
use std::io::{Read, Write};
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
    #[clap(name = "build", about = "Compile Go source to bytecode")]
    Build {
        source: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
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
            source,
            output,
        } => {
            let mut src = source.clone();
            let pkgs = parse_local_dependencies(&src).unwrap();

            let mut goc = Compiler::new();
            let main = src.clone();
            src.pop();

            let code = goc.compile(main, src, pkgs, false).unwrap();
            let bytes = serialize_bytecode(&code);

            let out_path = output.unwrap_or_else(|| {
                let mut p = source.clone();
                p.set_extension("govm");
                p
            });

            let mut file = File::create(&out_path)
                .unwrap_or_else(|e| panic!("failed to create output file {:?}: {}", out_path, e));
            file.write_all(&bytes)
                .unwrap_or_else(|e| panic!("failed to write bytecode: {}", e));

            println!("compiled to {:?}", out_path);
        }
        SubCommand::Run {
            output_assert,
            mut binary,
        } => {
            let is_bytecode = binary.extension().map_or(false, |ext| ext == "govm");

            let code = if is_bytecode {
                let mut file = File::open(&binary)
                    .unwrap_or_else(|e| panic!("failed to open {:?}: {}", binary, e));
                let mut data = Vec::new();
                file.read_to_end(&mut data)
                    .unwrap_or_else(|e| panic!("failed to read {:?}: {}", binary, e));
                deserialize_bytecode(&data)
                    .unwrap_or_else(|e| panic!("failed to load bytecode: {}", e))
            } else {
                let pkgs = parse_local_dependencies(&binary).unwrap();

                let mut goc = Compiler::new();
                let main = binary.clone();
                binary.pop();

                goc.compile(main, binary, pkgs, output_assert).unwrap()
            };

            let mut vm = VM::new();
            if let Err(e) = vm.run(code) {
                eprintln!("{}", vm.error_with_location(&e));
                std::process::exit(1);
            }
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
