# `mem-isolate`: *Run unsafe code safely*

[![Build Status](https://github.com/brannondorsey/mem-isolate/actions/workflows/build.yml/badge.svg)](https://github.com/brannondorsey/mem-isolate/actions/workflows/build.yml)

`mem-isolate` runs your function via a `fork()`, waits for the result, and returns it.

This grants your code access to an exact copy of memory and state at the time just before the call, but guarantees that the function will not affect the parent process's memory footprint in any way.

It forces functions to be *memory pure* (pure with respect to memory), even if they aren't.

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

* Run code with a known memory leak
* Run code that fragments the heap
* Run `unsafe` code
* Run your code 1ms slower (*har har* 😉, see [limitations](#limitations))

> NOTE: Because of its heavy use of POSIX system calls, this crate only supports Unix-like operating systems (e.g., Linux, macOS, BSD). Windows and wasm support are not planned at this time.

See the [examples/](examples/) for more uses, especially [the basic error handling example](examples/error-handling-basic.rs).

## How it works

POSIX systems use the `fork()` system call to create a new child process that is a copy of the parent. On modern systems, this is relatively cheap (~1ms) even if the parent process is using a lot of memory at the time of the call. This is because the OS uses copy-on-write memory techniques to avoid duplicating the entire memory of the parent process. At the time `fork()` is called, the parent and child all share the same physical pages in memory. Only when one of them modifies a page is it copied to a new location.

`mem-isolate` uses this implementation detail as a nifty hack to provide a `callable` function with a temporary and isolated memory space. You can think of this isolation almost like a snapshot is taken of your program's memory at the time `execute_in_isolated_process()` is called, which will be restored once the user-supplied `callable` function has finished executing.

When `execute_in_isolated_process()` is called, the process will:

1. Create a `pipe()` for inter-process communication between the process *it* has been invoked in (the "parent") and the new child process that will be created to isolate and run your `callable`
1. `fork()` a new child process
1. Execute the user-supplied `callable` in the child process and deliver its result back to the parent process through the pipe
1. Wait for the child process to finish with `waitpid()`
1. Return the result to the parent process

We call this trick the "fork and free" pattern. It's pretty nifty. 🫰

## Limitations

* Works only on POSIX systems (Linux, macOS, BSD)
* Data returned from the `callable` function must be serialized to and from the child process (using `serde`), which can be expensive for large data.
* Excluding serialization/deserialization cost, `execute_in_isolated_process()` introduces runtime overhead on the order of ~1ms compared to a direct invocation of the `callable`.

In performance critical systems, these overheads can be no joke. However, for many use cases, this is an affordable trade-off for the memory safety and snapshotting behavior that `mem-isolate` provides.

## Benchmarks

In a [simple benchmark](benches/benchmarks.rs), raw function calls are ~1.5ns, `fork()` + wait is ~1.7ms, and `execute_in_isolated_process()` is about 1.9ms. That's very slow by comparison, but tolerable for many use cases where memory safety is paramount.

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

All benchmarks were run on a ThinkPad T14 Gen 4 AMD (14″) laptop with a AMD Ryzen 5 PRO 7540U CPU @ 3.2Ghz max clock speed and 32 GB of RAM. The OS used was Debian 13 with Linux kernel 6.12.

## License

`mem-isolate` is dual-licensed under either of:

* [MIT license](https://opensource.org/license/mit)
* [Apache License, Version 2.0](https://opensource.org/license/apache-2-0)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you shall be dual licensed as above, without any
additional terms or conditions.
