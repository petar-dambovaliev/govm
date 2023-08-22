use criterion::{black_box, criterion_group, criterion_main, Criterion};
use gno_rs::parser::Parser;
use gno_rs::vm::compiler::Compiler;
use gno_rs::vm::VM;

fn fibonacci() {
    let mut parser = Parser::from(
        r#"
       package main

        func fibonacci(n int) int {
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
            fibonacci(30)
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
    c.bench_function("fib", |b| b.iter(|| fibonacci()));

    c.bench_function("fib_rust", |b| b.iter(|| fibonacci_rust(black_box(30))));
}

fn fibonacci_rust(n: u32) -> u32 {
    if n <= 1 {
        return n;
    }

    let (mut a, mut b) = (0, 1);
    for _ in 2..=n {
        let c = a + b;
        a = b;
        b = c;
    }

    b
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
