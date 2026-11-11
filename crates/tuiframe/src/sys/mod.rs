//! The platform layer. Each system's reader shares what it reads with the program's thread through
//! [`Shared`].

#[cfg(any(target_os = "macos", all(test, target_os = "linux")))]
mod macos;
#[cfg(unix)]
pub(crate) mod unix;
#[cfg(any(windows, test))]
pub(crate) mod windows;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use crate::event::Event;
use crate::layout::Size;
use crate::output::writer::lock;

/// What the reader thread shares with the program's thread.
#[derive(Debug)]
pub(crate) struct Shared {
    pub(crate) events: EventQueue,
    /// The terminal's size, `width << 16 | height`, so a frame reads it without a system call.
    size: AtomicU32,
    /// The next frame redraws everything, after a resize or a resume.
    redraw: AtomicBool,
}

impl Shared {
    pub(crate) fn new(size: Size) -> Self {
        Self {
            events: EventQueue::default(),
            size: AtomicU32::new(pack(size)),
            redraw: AtomicBool::new(true),
        }
    }

    /// The terminal's size, as the reader last read it.
    pub(crate) fn size(&self) -> Size {
        let [w1, w0, h1, h0] = self.size.load(Ordering::Acquire).to_be_bytes();
        Size::new(u16::from_be_bytes([w1, w0]), u16::from_be_bytes([h1, h0]))
    }

    pub(crate) fn set_size(&self, size: Size) {
        self.size.store(pack(size), Ordering::Release);
    }

    /// Makes the next frame a full redraw.
    pub(crate) fn request_redraw(&self) {
        self.redraw.store(true, Ordering::Release);
    }

    /// Whether the next frame is a full redraw, and clears that. A frame that takes it and doesn't
    /// reach the writer requests it again.
    pub(crate) fn take_redraw(&self) -> bool {
        self.redraw.swap(false, Ordering::AcqRel)
    }
}

fn pack(size: Size) -> u32 {
    u32::from(size.width) << 16 | u32::from(size.height)
}

/// The events the reader decoded, waiting for the program, with a condvar for those who block and
/// the wakers of those who poll.
#[derive(Debug, Default)]
pub(crate) struct EventQueue {
    state: Mutex<Queue>,
    ready: Condvar,
}

#[derive(Debug, Default)]
struct Queue {
    events: VecDeque<Event>,
    /// The waker of every pending poll, so no task waits for another to be woken.
    wakers: Vec<Waker>,
    /// The terminal closed, or the reader ended. What's queued still comes out.
    closed: bool,
}

/// What [`EventQueue::next_timeout`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Next {
    Event(Event),
    /// None came in time.
    Empty,
    /// None will, as the queue closed and is empty.
    Closed,
}

impl EventQueue {
    /// Queues `events`, leaving it empty, and wakes those waiting.
    pub(crate) fn push(&self, events: &mut Vec<Event>) {
        if events.is_empty() {
            return;
        }
        let wakers = {
            let mut queue = lock(&self.state);
            queue.events.extend(events.drain(..));
            std::mem::take(&mut queue.wakers)
        };
        self.ready.notify_all();
        wakers.into_iter().for_each(Waker::wake);
    }

    /// Ends the queue once what's in it comes out, and wakes those waiting.
    pub(crate) fn close(&self) {
        let wakers = {
            let mut queue = lock(&self.state);
            queue.closed = true;
            std::mem::take(&mut queue.wakers)
        };
        self.ready.notify_all();
        wakers.into_iter().for_each(Waker::wake);
    }

    /// The next event, waiting at most `timeout` for one.
    pub(crate) fn next_timeout(&self, timeout: Duration) -> Next {
        let until = Instant::now() + timeout;
        let mut queue = lock(&self.state);
        loop {
            if let Some(event) = queue.events.pop_front() {
                return Next::Event(event);
            }
            if queue.closed {
                return Next::Closed;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Next::Empty;
            }
            queue = self
                .ready
                .wait_timeout(queue, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// The next event, or `None` once the queue closed and is empty, keeping the task's waker
    /// while there is none.
    pub(crate) fn poll_next(&self, cx: &Context<'_>) -> Poll<Option<Event>> {
        let mut queue = lock(&self.state);
        if let Some(event) = queue.events.pop_front() {
            return Poll::Ready(Some(event));
        }
        if queue.closed {
            return Poll::Ready(None);
        }
        if !queue.wakers.iter().any(|kept| kept.will_wake(cx.waker())) {
            queue.wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }

    /// Everything queued, without waiting.
    pub(crate) fn batch(&self) -> Vec<Event> {
        lock(&self.state).events.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::writer::tests::block_on;
    use std::future::poll_fn;
    use std::sync::Arc;

    fn key(c: char) -> Event {
        Event::Key(crate::event::KeyEvent::new(
            crate::event::KeyCode::Char(c),
            crate::event::KeyModifiers::NONE,
        ))
    }

    #[test]
    fn the_size_survives_the_round_trip_through_its_atomic() {
        let shared = Shared::new(Size::new(80, 24));
        assert_eq!(shared.size(), Size::new(80, 24));
        shared.set_size(Size::new(u16::MAX, 1));
        assert_eq!(shared.size(), Size::new(u16::MAX, 1));
        // The first frame redraws everything, and a request lasts until a frame takes it.
        assert!(shared.take_redraw());
        assert!(!shared.take_redraw());
        shared.request_redraw();
        assert!(shared.take_redraw());
    }

    /// Events come out in order, those queued before the end still come out, and then the queue
    /// says it closed.
    #[test]
    fn events_come_out_in_order_until_the_queue_closes() {
        let shared = Shared::new(Size::ZERO);
        let queue = &shared.events;
        assert_eq!(queue.next_timeout(Duration::ZERO), Next::Empty);
        queue.push(&mut vec![key('a'), key('b'), key('c')]);
        assert_eq!(queue.next_timeout(Duration::ZERO), Next::Event(key('a')));
        queue.close();
        assert_eq!(queue.batch(), [key('b'), key('c')]);
        assert_eq!(queue.next_timeout(Duration::from_secs(5)), Next::Closed);
    }

    /// Every task waiting for an event is woken when one comes, not only the first.
    #[test]
    fn every_waiting_task_is_woken() {
        let queue = Arc::new(EventQueue::default());
        let tasks: Vec<_> = (0..3)
            .map(|_| {
                let queue = queue.clone();
                std::thread::spawn(move || {
                    block_on(
                        async {
                            poll_fn(|cx| queue.poll_next(cx)).await;
                        },
                        Duration::from_secs(5),
                    )
                })
            })
            .collect();
        std::thread::sleep(Duration::from_millis(50));
        queue.push(&mut vec![key('a'), key('b'), key('c')]);
        for task in tasks {
            assert!(task.join().unwrap());
        }
    }
}
