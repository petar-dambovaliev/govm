mod gc;

use broom::prelude::*;
use parser::ast::Declaration;
use parser::Parser;
use std::time::{Duration, Instant};

fn main() {
    let start = Instant::now();
    let mut parser = Parser::from(
        r#"
        package main
        
        func main() {
            println("hello world")
        }
    "#,
    );

    let ast = parser.parse_file().unwrap();

    let duration = start.elapsed();

    println!("Time elapsed in expensive_function() is: {:?}", duration);
}
