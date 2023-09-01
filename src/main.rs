pub mod parser;
pub mod vm;

use crate::parser::Parser;
use crate::vm::compiler::{bytecode_to_human, Compiler};
use crate::vm::VM;
use std::time::Instant;

fn main() {
    //todo definition order matters and it shouldn't
    let i = Instant::now();
    //todo
    // this seems to make the vm crazy &[]int{1,2,3}
    let mut parser = Parser::from(
        r#"
        package main

func fibonacciGo(n int) int {
	if n <= 1 {
		return n
	}

	a, b := 0, 1
	for i := 2; i <= n; i++ {
		c := a + b
		a, b = b, c
	}

	return b
}

func main() {
   for i:=0;i<2;i++ {
        print(fibonacciGo(70))
   }
}
    "#,
    );

    // func newClosure() func() {
    //     a := false
    //    return func() {
    //         switch a {
    //         case true:
    //             print(true)
    //         case false:
    //             print(false)
    //         }
    //    }
    // }

    // func foo(f func()) {
    //     f()
    // }

    // print("closure: switch")
    // switch b:=a; b {
    // case true:
    //     print(5)
    // case false:
    //     print(5)
    // }

    // func main() {
    //     a := 1
    //     switch b:=a; a {
    //     // case 4:
    //     //     print(4)
    //     case 1:
    //         print(1)
    //     case 1:
    //         print("default")
    //     }
    // }

    // let opts = Opts {
    //     gogc: 100.0,
    //     min_gc: 1024 * 1024,
    // };

    let f = parser.parse_file().unwrap();
    //panic!("{:#?}", f);
    let mut goc = Compiler::new();
    let code = goc.compile_ast(&f).unwrap();
    //println!("{:#?}", bytecode_to_human(&code.instructions, false));
    let mut vm = VM::new();
    let _ = vm.run(code.clone()).unwrap();

    //println!("{:#?}", parser.parse_file().unwrap());
    // let mut vm = VM::new(opts, parser);
    //  let i = Instant::now();
    // vm.run();
    // println!("{:#?}", vm);
    println!("{:#?}", i.elapsed());
}
