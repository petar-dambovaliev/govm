pub mod parser;
pub mod vm;

use crate::parser::Parser;
use crate::vm::compiler::compiler::Compiler;
use crate::vm::VM;
use bdwgc_alloc::Allocator;
use std::time::Instant;

fn main() {
    unsafe { Allocator::initialize() }
    //todo definition order matters and it shouldn't
    let i = Instant::now();
    //todo
    // this seems to make the vm crazy &[]int{1,2,3}
    let mut parser = Parser::from(
        r#"
package main

func main() {
    var printDog func(
            int,
            struct {
	            name   string
	            isGood bool
            })
    
    printDog = func(
        i int,
        dog struct {
            name   string
            isGood bool
    }) {
        if i == 0 {
            return
        }
    
        println(dog)
        printDog(i - 1, dog)
    }
    
    
	dog := struct {
		name   string
		isGood bool
	}{
		"Rex",
		true,
	}

	printDog(4, dog)
}
    "#,
    );

    let f = parser.parse_file().unwrap();
    //panic!("{:#?}", f);
    let mut goc = Compiler::new();
    let code = goc.compile_ast(&f).unwrap();

    //panic!("{}", bytecode_to_human(&code.instructions, true));
    let mut vm = VM::new();
    let _ = vm.run(code.clone()).unwrap();

    //println!("{:#?}", parser.parse_file().unwrap());
    // let mut vm = VM::new(opts, parser);
    //  let i = Instant::now();
    // vm.run();
    // println!("{:#?}", vm);
    println!("{:#?}", i.elapsed());
}
