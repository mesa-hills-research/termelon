//! The platform layer. Each system's reader shares what it reads with the program's thread through
//! [`Shared`].

#[cfg(any(target_os = "macos", all(test, target_os = "linux")))]
mod macos;
#[cfg(unix)]
pub(crate) mod unix;

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use crate::event::Event;
use crate::event::decode::Reply;
use crate::layout::Size;
use crate::output::writer::lock;

/// What the reader thread shares with the program's thread.
#[derive(Debug)]
pub(crate) struct Shared {
    pub(crate) events: EventQueue,
    pub(crate) replies: ReplySlot,
    /// The terminal's size, `width << 16 | height`, so a frame reads it without a system call.
    size: AtomicU32,
    /// The next frame redraws everything, after a resize or a resume.
    redraw: AtomicBool,
}

impl Shared {
    pub(crate) fn new(size: Size) -> Self {
        Self {
            events: EventQueue::default(),
            replies: ReplySlot::default(),
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

    /// Whether the next frame is a full redraw, leaving that as it is.
    pub(crate) fn redraw_pending(&self) -> bool {
        self.redraw.load(Ordering::Acquire)
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
    /// Why reading the terminal failed, when it did.
    failed: Option<io::ErrorKind>,
}

/// What [`EventQueue::next_timeout`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Next {
    Event(Event),
    /// None came in time.
    Empty,
    /// None will, as the queue closed and is empty.
    Closed,
    /// None will, as reading the terminal failed.
    Failed(io::ErrorKind),
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
        self.end(None);
    }

    /// Ends the queue as [`close`](Self::close) does, because reading the terminal failed with
    /// `kind`. A failure is kept over a close that came first.
    pub(crate) fn fail(&self, kind: io::ErrorKind) {
        self.end(Some(kind));
    }

    fn end(&self, failed: Option<io::ErrorKind>) {
        let wakers = {
            let mut queue = lock(&self.state);
            queue.closed = true;
            if failed.is_some() {
                queue.failed = failed;
            }
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
                return queue.failed.map_or(Next::Closed, Next::Failed);
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
    /// while there is none. Fails once the queue is empty, when reading the terminal failed.
    pub(crate) fn poll_next(&self, cx: &Context<'_>) -> Poll<Result<Option<Event>, io::ErrorKind>> {
        let mut queue = lock(&self.state);
        if let Some(event) = queue.events.pop_front() {
            return Poll::Ready(Ok(Some(event)));
        }
        if queue.closed {
            return Poll::Ready(queue.failed.map_or(Ok(None), Err));
        }
        if !queue.wakers.iter().any(|kept| kept.will_wake(cx.waker())) {
            queue.wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }

    /// The next event queued, without waiting.
    pub(crate) fn try_next(&self) -> Option<Event> {
        lock(&self.state).events.pop_front()
    }

    /// Everything queued, without waiting.
    pub(crate) fn batch(&self) -> Vec<Event> {
        lock(&self.state).events.drain(..).collect()
    }
}

/// Where the replies to the probe's queries go while it waits for them. Replies that come while it
/// isn't waiting are dropped, so a late one never reaches the program.
#[derive(Debug, Default)]
pub(crate) struct ReplySlot {
    /// The replies so far, while the probe waits.
    state: Mutex<Option<Vec<Reply>>>,
    arrived: Condvar,
}

impl ReplySlot {
    /// Starts keeping replies.
    pub(crate) fn open(&self) {
        *lock(&self.state) = Some(Vec::new());
    }

    /// Keeps `reply` while the slot is open, and drops it otherwise.
    pub(crate) fn offer(&self, reply: Reply) {
        if let Some(replies) = lock(&self.state).as_mut() {
            replies.push(reply);
            self.arrived.notify_all();
        }
    }

    /// Waits at most `timeout` until the replies kept so far satisfy `done`. Whether they did.
    pub(crate) fn wait(&self, timeout: Duration, done: impl Fn(&[Reply]) -> bool) -> bool {
        let until = Instant::now() + timeout;
        let mut state = lock(&self.state);
        loop {
            if state.as_deref().is_some_and(&done) {
                return true;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self
                .arrived
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Stops keeping replies, and returns those kept.
    pub(crate) fn close(&self) -> Vec<Reply> {
        lock(&self.state).take().unwrap_or_default()
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

    /// The reply slot keeps replies only while the probe waits, so a late one is dropped.
    #[test]
    fn replies_are_kept_only_while_the_probe_waits() {
        let slot = Arc::new(ReplySlot::default());
        slot.offer(Reply::KittyKeyboard(0));
        slot.open();
        let offering = {
            let slot = slot.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                slot.offer(Reply::KittyKeyboard(1));
                slot.offer(Reply::Da1 {
                    attributes: vec![62],
                });
            })
        };
        let is_da1 = |replies: &[Reply]| replies.iter().any(|r| matches!(r, Reply::Da1 { .. }));
        assert!(slot.wait(Duration::from_secs(5), is_da1));
        offering.join().unwrap();
        assert_eq!(
            slot.close(),
            [
                Reply::KittyKeyboard(1),
                Reply::Da1 {
                    attributes: vec![62]
                }
            ]
        );
        slot.offer(Reply::KittyKeyboard(2));
        assert!(!slot.wait(Duration::from_millis(10), is_da1));
        assert!(slot.close().is_empty());
        // (The redraw a resize asks for stays until a frame takes it.)
        let shared = Shared::new(Size::ZERO);
        assert!(shared.redraw_pending());
        assert!(shared.take_redraw());
        assert!(!shared.redraw_pending());
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
                            poll_fn(|cx| queue.poll_next(cx)).await.unwrap();
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
