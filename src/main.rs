pub mod parser;
pub mod vm;

use crate::parser::{parse_dir_recursive, Parser};
use crate::vm::compiler::compiler::Compiler;
use crate::vm::VM;
use bdwgc_alloc::Allocator;
use std::time::Instant;

#[global_allocator]
static GLOBAL_ALLOCATOR: Allocator = Allocator;

use crate::vm::module::parse_dependencies;
use clap::Parser as ClapParser;
use clap::Subcommand;
use std::path::PathBuf;

#[derive(ClapParser, Debug)]
struct Cli {
    #[clap(short, long)]
    debug: Option<u8>,
    #[clap(subcommand)]
    action: SubCommand,
}

#[derive(Subcommand, Debug)]
enum SubCommand {
    #[clap(name = "gors", about = "A Go virtual machine")]
    Build { source: PathBuf },
    #[clap(name = "run", about = "Builds and runs a Go binary")]
    Run { binary: PathBuf },
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
    }
}
