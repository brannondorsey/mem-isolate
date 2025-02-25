use criterion::{Criterion, black_box, criterion_group, criterion_main};

use mem_isolate::execute_in_isolated_process;
use std::time::Duration;

pub fn criterion_benchmark(c: &mut Criterion) {
    pub fn times_ten(x: u32) -> u32 {
        x * 10
    }
    let mut group = c.benchmark_group("Overhead");

    group
        .measurement_time(Duration::from_secs(1))
        .warm_up_time(Duration::from_secs(1));

    group.bench_function("direct_function_call", |b| {
        b.iter(|| times_ten(black_box(1)))
    });

    group.bench_function("fork_alone", |b| {
        b.iter(|| {
            match unsafe { libc::fork() } {
                -1 => panic!("Fork failed"),
                // Child process
                0 => {
                    times_ten(black_box(1));
                    unsafe { libc::_exit(0) };
                }
                // Parent process
                _ => {
                    let mut status = 0;
                    unsafe {
                        libc::wait(&mut status);
                    }
                }
            }
        })
    });

    group.bench_function("execute_in_isolated_process", |b| {
        b.iter(|| execute_in_isolated_process(|| times_ten(black_box(1))))
    });
    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
