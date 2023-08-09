use criterion::{black_box, criterion_group, criterion_main, Criterion};
use gno_rs::parser::Parser;
use gno_rs::vm::compiler::{Compiler};
use gno_rs::vm::VM;

fn fibonacci() {
    let mut parser = Parser::from(
        r#"
        package main

       func FibonacciRecursion(n int) int {
            if n < 2 {
                return n
            }
            return FibonacciRecursion(n-1) + FibonacciRecursion(n-2)
        }

        func main() {
            var a = FibonacciRecursion(30);
            //print("{}", a);
       }
    "#,
    );
    let f = parser.parse_file().unwrap();
    let mut goc = Compiler::new();
    let code = goc.compile_ast(&f).unwrap();

    let mut vm = VM::new();
    let _ = vm.run(code).unwrap();
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("fib", |b| {
        b.iter(|| {
            fibonacci()
        })
    });

    c.bench_function("fib_rust", |b| {
        b.iter(|| {
            fibonacci_rust(black_box(30))
        })
    });
}

fn fibonacci_rust(n: u64) -> u64 {
    if n <= 1 {
        return n;
    }
    return fibonacci_rust(n - 1) + fibonacci_rust(n - 2);
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
