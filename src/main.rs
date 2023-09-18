pub mod parser;
pub mod vm;

use crate::parser::Parser;
use crate::vm::compiler::compiler::Compiler;
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

func main() {

    m := make(map[string]int)

    m["k1"] = 7
    m["k2"] = 13

    println("map:", m)
    
    v1 := m["k1"]
    println("v1:", v1)
    
    v3 := m["k3"]
    println("v3:", v3)
    
    println("len:", len(m))
    
    delete(m, "k2")
    println("map:", m)
    
    clear(m)
    println("map:", m)
    
    _, prs := m["k2"]
    println("prs:", prs)
    
    n := map[string]int{"foo": 1, "bar": 2}
    println("map:", n)
    
    n2 := map[string]int{"foo": 1, "bar": 2}
    
    
    
    // if maps.Equal(n, n2) {
    //     println("n == n2")
    // }
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
