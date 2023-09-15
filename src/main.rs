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
    var s []string
    //println(s)
    //println("uninit:", s, s == nil, len(s) == 0)
     
    s = make([]string, 3)
    
    println("emp:", s, "len:", len(s), "cap:", cap(s))
     
    // s[0] = "a"
    // s[1] = "b"
    // s[2] = "c"
    // println("set:", s)
    // println("get:", s[2])
    //  
    // println("len:", len(s))
    // 
    // s = append(s, "d")
    // s = append(s, "e", "f")
    // println("apd:", s)
    // 
    // c := make([]string, len(s))
    // copy(c, s)
    // println("cpy:", c)
    // 
    // l := s[2:5]
    // println("sl1:", l)
    // 
    // l = s[:5]
    // println("sl2:", l)
    // 
    // l = s[2:]
    // println("sl3:", l)
    // 
    // t := []string{"g", "h", "i"}
    // println("dcl:", t)
    // 
    // t2 := []string{"g", "h", "i"}
    // if slices.Equal(t, t2) {
    //     println("t == t2")
    // }
    
    // twoD := make([][]int, 3)
    // for i := 0; i < 3; i++ {
    //     innerLen := i + 1
    //     twoD[i] = make([]int, innerLen)
    //     for j := 0; j < innerLen; j++ {
    //         twoD[i][j] = i + j
    //     }
    // }
    // println("2d: ", twoD)
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
