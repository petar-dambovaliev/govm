use std::time::Instant;
use parser::Parser;
use vm::{Opts, VM};

fn main() {
    let mut parser = Parser::from(
        r#"
        package main

       var b = true;
    "#,
    );

    /*
    package main

        func main() {
            a := 0
            for i:=0;i<1000;i++ {
                a += i
            }
        }
     */

    let opts = Opts {
        gogc: 100.0,
        min_gc: 1024 * 1024,
    };

    println!("{:#?}", parser.parse_file().unwrap());
    // let mut vm = VM::new(opts, parser);
    //  let i = Instant::now();
    // vm.run();
    // println!("{:#?}", vm);
    // println!("{:#?}", i.elapsed());
}
