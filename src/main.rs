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

import (
    "fmt"
    "slices"
)

func main() {

    var s []string
    
    println("uninit:", s, s == nil, len(s) == 0)
    
    s = make([]string, 3)
    println("emp:", s, "len:", len(s), "cap:", cap(s))
    
    s[0] = "a"
    s[1] = "b"
    s[2] = "c"
    println("set:", s)
    println("get:", s[2])
    
    println("len:", len(s))
    
    s = append(s, "d")
    s = append(s, "e", "f")
    println("apd:", s)
    
    c := make([]string, len(s))
    copy(c, s)
    println("cpy:", c)
    
    l := s[2:5]
    println("sl1:", l)
    
    l = s[:5]
    println("sl2:", l)
    
    l = s[2:]
    println("sl3:", l)
    
    twoD := make([][]int, 3)
    for i := 0; i < 3; i++ {
        innerLen := i + 1
        twoD[i] = make([]int, innerLen)
        for j := 0; j < innerLen; j++ {
            twoD[i][j] = i + j
        }
    }
    println("2d: ", twoD)
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
