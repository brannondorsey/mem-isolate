use libc::c_int;
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkReturn {
    Parent(i32),
    Child,
}
pub fn fork() -> Result<ForkReturn, io::Error> {
    const FORK_FAILED: i32 = -1;
    const FORK_CHILD: i32 = 0;

    let ret = unsafe { libc::fork() };
    match ret {
        FORK_FAILED => Err(io::Error::last_os_error()),
        FORK_CHILD => Ok(ForkReturn::Child),
        _ => {
            let child_pid = ret;
            Ok(ForkReturn::Parent(child_pid))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeFds {
    pub read_fd: c_int,
    pub write_fd: c_int,
}

pub fn pipe() -> Result<PipeFds, io::Error> {
    let mut pipe_fds: [c_int; 2] = [0; 2];
    let ret = unsafe { libc::pipe(pipe_fds.as_mut_ptr()) };
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(PipeFds {
            read_fd: pipe_fds[0],
            write_fd: pipe_fds[1],
        })
    }
}

pub fn close(fd: c_int) -> Result<(), io::Error> {
    let ret = unsafe { libc::close(fd) };
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn _exit(status: c_int) -> ! {
    unsafe { libc::_exit(status) };
}

type ReturnStatus = c_int;
pub fn waitpid(pid: c_int) -> Result<ReturnStatus, io::Error> {
    let mut status: ReturnStatus = 0;
    // TODO: Explore if any options are needed
    let ret = unsafe { libc::waitpid(pid, &mut status as *mut c_int, 0) };
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(status)
    }
}
