use criterion::{black_box, criterion_group, criterion_main, Criterion};
use gno_rs::parser::Parser;
use gno_rs::vm::compiler::Compiler;
use gno_rs::vm::VM;

fn fibonacci() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .on_thread_start(|| {
            unsafe {
                bdwgc_alloc::Allocator::register_current_thread()
                    .expect("failed to register GC thread");
            }
        })
        .on_thread_stop(|| {
            unsafe { bdwgc_alloc::Allocator::unregister_current_thread() }
        })
        .build()
        .unwrap();
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
    let _ = rt.block_on(vm.run(code)).unwrap();
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("fib", |b| b.iter(|| fibonacci()));

    //c.bench_function("fib_rust", |b| b.iter(|| fibonacci_rust(black_box(30))));
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
