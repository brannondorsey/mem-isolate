use mem_isolate::execute_in_isolated_process;

fn main() {
    let leaky_fn = || {
        // Leak 1KiB of memory
        let data: Vec<u8> = vec![42; 1024];
        let data = Box::new(data);
        let uh_oh = Box::leak(data);
        let leaked_ptr = format!("{:p}", uh_oh);
        assert!(
            check_memory_exists_and_holds_vec_data(&leaked_ptr),
            "The memory should exist in `leaky_fn()` where it was leaked"
        );
        leaked_ptr
    };

    let leaked_ptr: String = execute_in_isolated_process(leaky_fn).unwrap();
    assert!(
        !check_memory_exists_and_holds_vec_data(&leaked_ptr),
        "The leaked memory doesn't exist out here though"
    );
    println!(
        "Success, the memory leak in `leaky_fn()` was contained to the ephemeral child process it executed inside of"
    );
}

fn check_memory_exists_and_holds_vec_data(ptr_str: &str) -> bool {
    let addr = usize::from_str_radix(ptr_str.trim_start_matches("0x"), 16).unwrap();
    let vec_ptr = addr as *const Vec<u8>;
    !vec_ptr.is_null() && unsafe { std::ptr::read_volatile(vec_ptr) }.capacity() > 0
}

// Add a smoke test to make sure this example stays working
#[cfg(test)]
mod example_memory_leak {
    use super::*;

    #[test]
    fn execute_main() {
        main().unwrap();
    }
}
