pub mod parser;
pub mod vm;

use crate::parser::Parser;
use crate::vm::compiler::Compiler;
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
        
        var b bool
        
        var s string
        
        var (
            i   int
            i8  int8
            i16 int16
            i32 int32
            i64 int64
        )
         
        var (
            ui   uint
            ui8  uint8
            ui16 uint16
            ui32 uint32
            ui64 uint64
        )

        var (
            by byte // alias for uint8
            r  rune // alias for int32. Represents a Unicode code point.
        )

        var (
            f32 float32
            f64 float64
        )

        func Print() {
            println(b)
            println(s)
            println(i, i8, i16, i32, i64)
            println(ui, ui8, ui16, ui32, ui64)
            println(by, r)
            println(f32, f64)
        }

        func SetValues() {
            b = true
        
            s = "a string"
        
            i = -42
            i8 = -8
            i16 = -4216
            i32 = -4232
            i64 = -4264
        
            ui = 42
            ui8 = 8
            ui16 = 4216
            ui32 = 4232
            ui64 = 4264
        
            by = byte('A')
            r = rune('A')
        
            f32 = 42.32
            f64 = 42.64
        }

        func main() {
            SetValues()
            Print()
        }

// package main
// 
// type Foo struct {}
// 
// func (f *Foo) blah() {
//     print(f)
// }
// 
// 
// func main() {
//     
// }
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
