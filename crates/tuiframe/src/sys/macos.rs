//! Waiting on macOS, whose `poll` doesn't work on `/dev/tty`, with `select`. Linux builds it for
//! tests too, where `select` works the same.

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::time::Duration;

use rustix::event::{FdSetElement, FdSetIter, fd_set_insert, fd_set_num_elements};

/// Which of `fds` have input or hung up, waiting at most `timeout`, or with no limit for `None`.
#[expect(
    unsafe_code,
    reason = "rustix's `select` is unsafe, as it can't check that the descriptors stay open"
)]
pub(crate) fn wait<const N: usize>(
    fds: [BorrowedFd<'_>; N],
    timeout: Option<Duration>,
) -> io::Result<[bool; N]> {
    let nfds = fds
        .iter()
        .map(AsRawFd::as_raw_fd)
        .max()
        .map_or(0, |fd| fd + 1);
    let mut readable = vec![FdSetElement::default(); fd_set_num_elements(N, nfds)];
    for fd in &fds {
        fd_set_insert(&mut readable, fd.as_raw_fd());
    }
    let timeout = timeout.map(super::unix::timespec);
    // SAFETY: [macos-select] every descriptor in the set is one of `fds`, which are borrowed for
    // the whole call, so each stays open until select returns.
    unsafe { rustix::event::select(nfds, Some(&mut readable), None, None, timeout.as_ref()) }?;
    let ready: Vec<RawFd> = FdSetIter::new(&readable).collect();
    Ok(fds.map(|fd| ready.contains(&fd.as_raw_fd())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Size;
    use crate::sys::unix::tests::Pty;
    use std::io::Write;
    use std::os::fd::AsFd;
    use std::time::Instant;

    /// `select` says which descriptors are ready, a terminal's included, gives up at its timeout,
    /// and takes descriptors above the classic limit of 1024.
    #[test]
    fn select_says_which_are_ready() {
        let (first, mut first_end) = io::pipe().unwrap();
        let (second, second_end) = io::pipe().unwrap();
        let started = Instant::now();
        let ready = wait(
            [first.as_fd(), second.as_fd()],
            Some(Duration::from_millis(50)),
        );
        assert_eq!(ready.unwrap(), [false, false]);
        assert!(started.elapsed() >= Duration::from_millis(50));
        first_end.write_all(b"x").unwrap();
        assert_eq!(
            wait([first.as_fd(), second.as_fd()], None).unwrap(),
            [true, false]
        );
        drop(second_end);
        assert_eq!(
            wait([second.as_fd()], Some(Duration::ZERO)).unwrap(),
            [true]
        );

        let mut pty = Pty::open(Size::new(80, 24));
        assert_eq!(
            wait([pty.slave.as_fd()], Some(Duration::ZERO)).unwrap(),
            [false]
        );
        // (In the terminal's usual modes, input is ready a line at a time.)
        pty.type_in(b"k\n");
        assert_eq!(wait([pty.slave.as_fd()], None).unwrap(), [true]);

        let high = rustix::io::fcntl_dupfd_cloexec(&first, 1500);
        if let Ok(high) = high {
            assert_eq!(wait([high.as_fd()], Some(Duration::ZERO)).unwrap(), [true]);
        }
    }
}
