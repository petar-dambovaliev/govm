use parser::Parser;
use vm::{Opts, VM};

fn main() {
    let parser = Parser::from(
        r#"
        package main
        
        func main() {
            println("hello world")
        }
    "#,
    );

    let opts = Opts {
        gogc: 100.0,
        min_gc: 1024 * 1024,
    };

    let mut vm = VM::new(opts, parser);
    vm.run();
}
