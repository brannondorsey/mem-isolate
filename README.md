# mem-isolate: Run unsafe code safely

`mem-isolate` runs your function via a `fork()`, waits for the result, and returns it.

This grants your code access to an exact copy of memory and state at the time just before the call, but gaurantees that the function will not affect the parent process in any way. It forces functions to be pure, even if they aren't.

```rust
use mem_isolate::execute_in_isolated_process;

// No heap, stack, or program memory out here...
let result = mem_isolate::execute_in_isolated_process(|| {
    // ...Can be affected by anything in here
    unsafe {
        gnarly_cpp_bindings::potentially_leaking_function();
        unstable_ffi::segfault_prone_function();
        heap_fragmenting_operation();
        something_that_panics_in_a_way_you_could_recover_from();
    }
});
```

Example use cases:

* Running code with a known memory leak
* Running code that fragments the heap
* Running `unsafe` code

## How it works

TODO
blah blah blah

We call this the "fork and free" pattern.

## Benchmarks

Raw function calls are ~1.5ns, forks() + wait is ~1.7ms, and my unoptimized `execute_in_isolated_process()` is about 1.9ms. Slow, but tolerable for many use cases.

```txt
cargo bench
    Finished `bench` profile [optimized] target(s) in 0.07s
     Running unittests src/lib.rs (target/release/deps/mem_isolate-d96fcfa5f2fd31c0)

running 3 tests
test tests::simple_example ... ignored
test tests::test_static_memory_mutation_with_isolation ... ignored
test tests::test_static_memory_mutation_without_isolation ... ignored

test result: ok. 0 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running benches/benchmarks.rs (target/release/deps/benchmarks-25c74db99f107a73)
Overhead/direct_function_call
                        time:   [1.4347 ns 1.4357 ns 1.4370 ns]
                        change: [-1.4983% +0.6412% +3.4486%] (p = 0.55 > 0.05)
                        No change in performance detected.
Found 11 outliers among 100 measurements (11.00%)
  4 (4.00%) high mild
  7 (7.00%) high severe
Overhead/fork_alone     time:   [1.6893 ms 1.6975 ms 1.7062 ms]
                        change: [+1.3025% +3.8968% +5.7914%] (p = 0.01 < 0.05)
                        Performance has regressed.
Found 2 outliers among 100 measurements (2.00%)
  1 (1.00%) high mild
  1 (1.00%) high severe
Overhead/execute_in_isolated_process
                        time:   [1.8769 ms 1.9007 ms 1.9226 ms]
                        change: [-7.6229% -5.7657% -3.7073%] (p = 0.00 < 0.05)
                        Performance has improved.
Found 1 outliers among 100 measurements (1.00%)
  1 (1.00%) low severe
```
