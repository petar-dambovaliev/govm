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
        
        type Ager interface {
           ageOneYear()
        }
        
        type Namer interface {
           getName()
        }
        
        type User struct {
            age int
            name string
        }
        
        func (u *User) ageOneYear() {
            u.age += 1
        }
        
        func (u *User) getName() {
            println(u.name)
        }
        
        func Age(a Ager) {
            a.ageOneYear()
        }
        
        func printName(n Namer) {
            n.getName()
        }
        
        func main() {
            peter := User{age: 36, name: "peter"}
            Age(&peter)
            printName(&peter)
            println(peter)
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
