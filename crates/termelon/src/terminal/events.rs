//! [`Events`], the program's end of the queue the reader thread fills.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use super::{Error, Result};
use crate::event::Event;
use crate::sys::{Next, Shared};

/// What the terminal reports, in order, from [`Terminal::events`](super::Terminal::events).
///
/// Events read together come out together, so after [`next`](Self::next) gives one,
/// [`batch`](Self::batch) gives those that came with it. The events end when the terminal closes,
/// the terminal stops taking output, or the [`Terminal`](super::Terminal) closes, and then
/// [`next_timeout`](Self::next_timeout) and [`next`](Self::next) give [`Error::Closed`] once none is
/// left. Clones share one queue, and each event goes to one of them.
#[derive(Debug, Clone)]
pub struct Events {
    shared: Arc<Shared>,
    /// A failure was given to the stream, which then ends.
    #[cfg(feature = "stream")]
    failure_given: bool,
}

impl Events {
    pub(crate) fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            #[cfg(feature = "stream")]
            failure_given: false,
        }
    }

    /// The next event, waiting at most `timeout` for one. `None` when none came in time.
    ///
    /// # Errors
    ///
    /// [`Error::Closed`] once the events ended and none is left, and [`Error::Io`] when reading
    /// the terminal failed.
    pub fn next_timeout(&mut self, timeout: Duration) -> Result<Option<Event>> {
        match self.shared.events.next_timeout(timeout) {
            Next::Event(event) => Ok(Some(event)),
            Next::Empty => Ok(None),
            Next::Closed => Err(Error::Closed),
            Next::Failed(kind) => Err(Error::Io(kind.into())),
        }
    }

    /// The next event already queued, without waiting.
    pub fn try_next(&mut self) -> Option<Event> {
        self.shared.events.try_next()
    }

    /// Every event already queued, without waiting.
    pub fn batch(&mut self) -> Vec<Event> {
        self.shared.events.batch()
    }

    /// The next event, as a future that works on any executor, `tokio::select!` included.
    ///
    /// Every task waiting on it is woken when an event comes, not only the first, so no event
    /// waits for something else to wake its task.
    #[expect(
        clippy::should_implement_trait,
        reason = "an async `next`, as streams have, which `Iterator::next` can't be"
    )]
    pub fn next(&mut self) -> NextEvent<'_> {
        NextEvent { events: self }
    }

    fn poll(&self, cx: &Context<'_>) -> Poll<Result<Event>> {
        self.shared.events.poll_next(cx).map(|next| match next {
            Ok(Some(event)) => Ok(event),
            Ok(None) => Err(Error::Closed),
            Err(kind) => Err(Error::Io(kind.into())),
        })
    }
}

/// The future of [`Events::next`].
///
/// # Errors
///
/// It completes with [`Error::Closed`] once the events ended and none is left, as
/// [`Events::next_timeout`] does, and with [`Error::Io`] when reading the terminal failed.
#[derive(Debug)]
#[must_use = "futures do nothing unless polled"]
pub struct NextEvent<'a> {
    events: &'a mut Events,
}

impl Future for NextEvent<'_> {
    type Output = Result<Event>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.events.poll(cx)
    }
}

/// The events as a stream, which ends once the events end, after giving a failure to read the
/// terminal once.
#[cfg(feature = "stream")]
impl futures_core::Stream for Events {
    type Item = Result<Event>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.failure_given {
            return Poll::Ready(None);
        }
        self.as_ref().get_ref().poll(cx).map(|next| match next {
            Ok(event) => Some(Ok(event)),
            Err(Error::Closed) => None,
            Err(error) => {
                self.failure_given = true;
                Some(Err(error))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{KeyCode, KeyEvent, KeyModifiers};
    use crate::layout::Size;
    use crate::output::writer::tests::block_on;

    fn key(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn events_on_a_queue() -> (Arc<Shared>, Events) {
        let shared = Arc::new(Shared::new(Size::new(80, 24)));
        (shared.clone(), Events::new(shared))
    }

    /// Pushes `events` from another thread after a moment, as the reader does.
    fn push_later(shared: &Arc<Shared>, events: Vec<Event>) -> std::thread::JoinHandle<()> {
        let shared = shared.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            shared.events.push(&mut events.clone());
        })
    }

    #[test]
    fn blocking_reads_time_out_and_then_see_the_end() {
        let (shared, mut events) = events_on_a_queue();
        assert!(events.next_timeout(Duration::ZERO).unwrap().is_none());
        assert_eq!(events.try_next(), None);
        shared.events.push(&mut vec![key('a'), key('b'), key('c')]);
        assert_eq!(events.next_timeout(Duration::ZERO).unwrap(), Some(key('a')));
        assert_eq!(events.batch(), [key('b'), key('c')]);
        shared.events.push(&mut vec![key('d')]);
        shared.events.close();
        assert_eq!(events.try_next(), Some(key('d')));
        assert!(matches!(
            events.next_timeout(Duration::from_secs(5)),
            Err(Error::Closed)
        ));
        let (shared, mut failed) = events_on_a_queue();
        shared.events.fail(std::io::ErrorKind::BrokenPipe);
        assert!(matches!(
            failed.next_timeout(Duration::ZERO),
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::BrokenPipe
        ));
    }

    /// On a plain executor, a task waiting for an event wakes when one comes, and draining the
    /// batch that came with it loses no wakeup for the next.
    #[test]
    fn next_wakes_its_task_after_a_batch_is_drained() {
        let (shared, mut events) = events_on_a_queue();
        let pushing = push_later(&shared, vec![key('a'), key('b')]);
        let mut first = None;
        assert!(block_on(
            async { first = Some(events.next().await.unwrap()) },
            Duration::from_secs(5)
        ));
        pushing.join().unwrap();
        assert_eq!(first, Some(key('a')));
        assert_eq!(events.batch(), [key('b')]);
        let pushing = push_later(&shared, vec![key('c')]);
        let mut second = None;
        assert!(block_on(
            async { second = Some(events.next().await.unwrap()) },
            Duration::from_secs(5)
        ));
        pushing.join().unwrap();
        assert_eq!(second, Some(key('c')));
        shared.events.close();
        let mut last = None;
        assert!(block_on(
            async { last = Some(events.next().await) },
            Duration::from_secs(5)
        ));
        assert!(matches!(last, Some(Err(Error::Closed))));
    }

    /// `next` works as a branch of `tokio::select!`, beside a timer.
    #[test]
    fn next_works_in_tokio_select() {
        let (shared, mut events) = events_on_a_queue();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let pushing = push_later(&shared, vec![key('a'), key('b')]);
        let mut ticks = 0;
        let got = runtime.block_on(async {
            loop {
                tokio::select! {
                    event = events.next() => break event.unwrap(),
                    () = tokio::time::sleep(Duration::from_millis(5)) => ticks += 1,
                }
            }
        });
        pushing.join().unwrap();
        assert_eq!(got, key('a'));
        assert!(ticks > 0);
        assert_eq!(events.batch(), [key('b')]);
        let timed_out = runtime.block_on(async {
            tokio::select! {
                _ = events.next() => false,
                () = tokio::time::sleep(Duration::from_millis(20)) => true,
            }
        });
        assert!(timed_out);
    }

    #[cfg(feature = "stream")]
    #[test]
    fn the_stream_gives_a_failure_once_and_ends() {
        use futures_core::Stream;
        let (shared, mut events) = events_on_a_queue();
        shared.events.push(&mut vec![key('a')]);
        shared.events.fail(std::io::ErrorKind::BrokenPipe);
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut pinned = Pin::new(&mut events);
        assert!(matches!(
            pinned.as_mut().poll_next(&mut cx),
            Poll::Ready(Some(Ok(event))) if event == key('a')
        ));
        assert!(matches!(
            pinned.as_mut().poll_next(&mut cx),
            Poll::Ready(Some(Err(Error::Io(_))))
        ));
        assert!(matches!(
            pinned.as_mut().poll_next(&mut cx),
            Poll::Ready(None)
        ));
    }
}
