pub mod parser;
pub mod vm;

use crate::parser::Parser;
use crate::vm::compiler::compiler::Compiler;
use crate::vm::VM;
use bdwgc_alloc::Allocator;
use std::time::Instant;

#[global_allocator]
static GLOBAL_ALLOCATOR: Allocator = Allocator;

fn main() {
    unsafe { Allocator::initialize() }
    //todo definition order matters and it shouldn't
    let i = Instant::now();
    //todo
    // this seems to make the vm crazy &[]int{1,2,3}
    let mut parser = Parser::from(
        r#"
package main

type User struct {
	name string
}

func NewUser() *User {
    return &User {
        name: "john",
	}
}

func Foo() {
    a := []*User{}
    
	for i := 0; i < 4; i++ {
	    for j := 0; j < 999999; j++ { 
		    a = append(a, NewUser())
	    }
	    println(len(a))
	    a = []*User{}
	}
}

func l() {
    println("loop")
    for i := 0; i < 10000; i++ {
	    for j := 0; j < 999990; j++ { 
		    
	    }
	}
}

func main() {
    Foo()
	l()
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
