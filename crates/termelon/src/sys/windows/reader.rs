//! The reader thread on Windows. It waits on the console's input and a wake event, reads input
//! records, and decodes them, as records or, in [`InputMode::Vt`], as the bytes their characters
//! carry.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{Decode, Input, InputMode, Ready, Wake, Window};
use crate::event::Event;
use crate::event::decode::{Decoded, Query};
use crate::event::records::{InputRecord, KeyRecord};
use crate::output::writer::lock;
use crate::sys::Shared;

/// How long [`Reader::stop`] waits for the reader's thread.
const CONTROL_WAIT: Duration = Duration::from_secs(1);

/// How long the input may be silent while the decoder holds a sequence, a string or a paste, before
/// it's given up on.
pub(super) const HELD_WAIT: Duration = Duration::from_secs(1);

/// What the program changes while the reader runs, read at each turn.
#[derive(Debug, Default)]
pub(crate) struct Settings {
    /// Key records carry bytes, [`InputMode::Vt`].
    vt: AtomicBool,
    /// Focus records become events. The console sends them whether or not the program asked.
    focus: AtomicBool,
}

impl Settings {
    pub(crate) fn set_mode(&self, mode: InputMode) {
        self.vt.store(mode == InputMode::Vt, Ordering::Release);
    }

    pub(crate) fn set_focus(&self, focus: bool) {
        self.focus.store(focus, Ordering::Release);
    }

    fn mode(&self) -> InputMode {
        if self.vt.load(Ordering::Acquire) {
            InputMode::Vt
        } else {
            InputMode::Records
        }
    }
}

/// The reader thread's handle.
pub(crate) struct Reader {
    thread: Option<JoinHandle<()>>,
    control: Arc<Control>,
    wake: Arc<dyn Wake>,
}

impl std::fmt::Debug for Reader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reader")
            .field("control", &self.control)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct Control {
    state: Mutex<Requests>,
    changed: Condvar,
}

/// What the program asks of the decoder, done before it decodes what the reader reads next.
#[derive(Debug)]
enum Ask {
    Expect(Query),
    Cancel,
}

#[derive(Debug, Default)]
struct Requests {
    /// What the decoder is to do before it decodes what the reader reads next, in order.
    asks: Vec<Ask>,
    /// A new escape timeout, from what the reader reads next.
    escape_timeout: Option<Duration>,
    /// The program asks the reader to hold still.
    park: bool,
    /// The reader holds still.
    parked: bool,
    stop: bool,
    /// The reader's thread ended.
    ended: bool,
}

impl Reader {
    /// Starts the reader's thread. It reads from `input`, decodes with `decoder` as `settings`
    /// say, and queues the events in `shared`. A pending ESC among the bytes records carry waits
    /// `escape_timeout` for the rest, or decodes at once when it's zero and no more input is
    /// ready, and a sequence or a paste held while the input is silent for a second is given up
    /// on. A window size record reads the window's size and queues [`Event::Resize`]. When the
    /// input fails, or the thread ends, the queue closes.
    pub(crate) fn spawn(
        input: impl Input,
        wake: Arc<dyn Wake>,
        decoder: impl Decode,
        shared: Arc<Shared>,
        settings: Arc<Settings>,
        escape_timeout: Duration,
    ) -> io::Result<Self> {
        let control = Arc::new(Control::default());
        let window = input.window().unwrap_or_default();
        let run = Loop {
            input,
            decoder,
            settings,
            escape_timeout,
            window,
            surrogate: None,
            ended: Ended {
                shared,
                control: control.clone(),
            },
        };
        let thread = std::thread::Builder::new()
            .name("termelon-reader".into())
            .spawn(move || run.run())?;
        Ok(Self {
            thread: Some(thread),
            control,
            wake,
        })
    }

    /// Makes the decoder expect the answer to `query`, before it decodes what the reader reads
    /// next, so calling it before the query is written is enough.
    pub(crate) fn expect(&self, query: Query) {
        lock(&self.control.state).asks.push(Ask::Expect(query));
    }

    /// Makes a pending ESC wait `timeout` for what follows it, from what the reader reads next.
    pub(crate) fn set_escape_timeout(&self, timeout: Duration) {
        lock(&self.control.state).escape_timeout = Some(timeout);
    }

    /// Makes the decoder forget the queries whose answers are outstanding, before it decodes what
    /// the reader reads next.
    pub(crate) fn cancel_expected(&self) {
        lock(&self.control.state).asks.push(Ask::Cancel);
    }

    /// Holds the reader still, reading nothing until [`unpark`](Self::unpark), for while another
    /// program has the console. Returns whether it held still within a second.
    pub(crate) fn park(&self) -> bool {
        lock(&self.control.state).park = true;
        self.wake.wake();
        self.wait_until(|requests| requests.parked || requests.ended)
    }

    /// Lets the reader go on.
    pub(crate) fn unpark(&self) {
        lock(&self.control.state).park = false;
        self.control.changed.notify_all();
    }

    /// Waits a second at most until `done` holds of the requests, and says whether it did.
    fn wait_until(&self, done: impl Fn(&Requests) -> bool) -> bool {
        let until = Instant::now() + CONTROL_WAIT;
        let mut requests = lock(&self.control.state);
        while !done(&requests) {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            requests = self
                .control
                .changed
                .wait_timeout(requests, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    /// Ends the reader's thread, waiting a second at most.
    pub(crate) fn stop(mut self) {
        self.end();
    }

    fn end(&mut self) {
        lock(&self.control.state).stop = true;
        self.control.changed.notify_all();
        self.wake.wake();
        let ended = self.wait_until(|requests| requests.ended);
        if let Some(thread) = self.thread.take()
            && ended
        {
            let _ = thread.join();
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.end();
        }
    }
}

/// Closes the queue and says the thread ended, also when it ends by a panic.
struct Ended {
    shared: Arc<Shared>,
    control: Arc<Control>,
}

impl Drop for Ended {
    fn drop(&mut self) {
        self.shared.events.close();
        lock(&self.control.state).ended = true;
        self.control.changed.notify_all();
    }
}

/// The reader's thread.
struct Loop<I, D> {
    input: I,
    decoder: D,
    settings: Arc<Settings>,
    escape_timeout: Duration,
    /// The window's rectangle, read at the start and at each window size record.
    window: Window,
    /// The first half of a surrogate pair, among the records whose characters are bytes.
    surrogate: Option<u16>,
    ended: Ended,
}

impl<I: Input, D: Decode> Loop<I, D> {
    fn run(mut self) {
        let mut records = Vec::new();
        let mut decoded = Vec::new();
        let mut bytes = Vec::new();
        // (Since when, and for how long, the decoder waits for the rest of what it holds.)
        let mut waiting: Option<(Instant, Duration)> = None;
        loop {
            {
                let control = self.ended.control.clone();
                let mut requests = lock(&control.state);
                if requests.park && !requests.stop {
                    requests.parked = true;
                    control.changed.notify_all();
                    while requests.park && !requests.stop {
                        requests = control
                            .changed
                            .wait(requests)
                            .unwrap_or_else(PoisonError::into_inner);
                    }
                    requests.parked = false;
                }
                if requests.stop {
                    break;
                }
            }
            let timeout = waiting.map(|(since, wait)| wait.saturating_sub(since.elapsed()));
            let ready = match self.input.wait(timeout) {
                Ok(ready) => ready,
                Err(error) => {
                    self.ended.shared.events.fail(error.kind());
                    break;
                }
            };
            match ready {
                Ready::Woken => continue,
                Ready::TimedOut => {
                    // (Nothing more came in time.)
                    if waiting.take().is_some() {
                        if self.decoder.has_pending() {
                            self.decoder.flush_pending(&mut decoded);
                        } else {
                            self.decoder.flush_held(&mut decoded);
                        }
                        waiting = self.wait_for_rest(&mut decoded);
                    }
                }
                Ready::Input => {
                    records.clear();
                    if let Err(error) = self.input.read(&mut records) {
                        self.ended.shared.events.fail(error.kind());
                        break;
                    }
                    self.take_requests();
                    self.decode(&records, &mut bytes, &mut decoded);
                    waiting = self.wait_for_rest(&mut decoded);
                }
            }
            self.deliver(&mut decoded);
        }
        self.decoder.flush_pending(&mut decoded);
        self.deliver(&mut decoded);
    }

    /// Takes the queries the program asked to expect or forget and a new escape timeout, just
    /// before decoding a read, so they apply to whatever came after they were asked for, also when
    /// the reader runs late.
    fn take_requests(&mut self) {
        let (asks, timeout) = {
            let mut requests = lock(&self.ended.control.state);
            (
                std::mem::take(&mut requests.asks),
                requests.escape_timeout.take(),
            )
        };
        for ask in asks {
            match ask {
                Ask::Expect(query) => self.decoder.expect(query),
                Ask::Cancel => self.decoder.cancel_expected(),
            }
        }
        if let Some(timeout) = timeout {
            self.escape_timeout = timeout;
        }
    }

    /// How long to wait for the rest of what the decoder holds, once the input is read. A pending
    /// ESC or introducer waits the escape timeout, or none when that's zero and no more input is
    /// ready, and anything else held waits [`HELD_WAIT`] before it's given up on.
    fn wait_for_rest(&mut self, decoded: &mut Vec<Decoded>) -> Option<(Instant, Duration)> {
        if self.decoder.has_pending() {
            let more = self.input.more().unwrap_or(false);
            if !self.escape_timeout.is_zero() || more {
                return Some((Instant::now(), self.escape_timeout));
            }
            self.decoder.flush_pending(decoded);
        }
        self.decoder.has_held().then(|| (Instant::now(), HELD_WAIT))
    }

    /// Decodes a batch of records in the mode the program set. In [`InputMode::Records`], the
    /// records that carry what a terminal sent rather than a key typed go to the bytes' decoder:
    /// those with no virtual key, an ESC that more characters follow in the batch and what follows
    /// it, and what comes while the bytes' decoder holds a sequence. A reply that comes late then
    /// decodes as a reply, never as keys, and keys typed while the probe ran in
    /// [`InputMode::Vt`] decode as the keys they were.
    fn decode(&mut self, records: &[InputRecord], bytes: &mut Vec<u8>, out: &mut Vec<Decoded>) {
        let mode = self.settings.mode();
        // (Within a burst of characters an ESC led.)
        let mut burst = false;
        for (at, record) in records.iter().enumerate() {
            match *record {
                InputRecord::Key(key) if mode == InputMode::Vt => self.feed_key(key, bytes, out),
                InputRecord::Key(key) if burst && !key.key_down => {}
                InputRecord::Key(key)
                    if key.unicode != 0
                        && (burst || key.virtual_key == 0 || self.decoder.has_held()) =>
                {
                    // (What follows in the batch is the rest of it, key-ups and all.)
                    burst |= key.key_down;
                    self.feed_key(key, bytes, out);
                }
                InputRecord::Key(key)
                    if key.key_down && key.unicode == 0x1b && starts_burst(records, at) =>
                {
                    burst = true;
                    self.feed_key(key, bytes, out);
                }
                InputRecord::WindowSize(_) => {
                    burst = false;
                    if let Ok(window) = self.input.window() {
                        self.window = window;
                    }
                    // (A console sends one at the start, and for buffer changes that leave the
                    // window as it was, so only a new size is a resize.)
                    let size = self.window.size();
                    if size != self.ended.shared.size() {
                        self.ended.shared.set_size(size);
                        self.ended.shared.request_redraw();
                        out.push(Decoded::Event(Event::Resize(size)));
                    }
                }
                InputRecord::Focus(_) if !self.settings.focus.load(Ordering::Acquire) => {
                    burst = false;
                }
                InputRecord::Mouse(mut mouse) => {
                    burst = false;
                    (mouse.column, mouse.row) = self.window.relative(mouse.column, mouse.row);
                    self.decoder
                        .decode_records(&[InputRecord::Mouse(mouse)], out);
                }
                record => {
                    burst = false;
                    self.decoder.decode_records(&[record], out);
                }
            }
        }
    }

    /// Decodes the bytes a key record carries.
    fn feed_key(&mut self, key: KeyRecord, bytes: &mut Vec<u8>, out: &mut Vec<Decoded>) {
        bytes.clear();
        vt_bytes(key, &mut self.surrogate, bytes);
        if !bytes.is_empty() {
            self.decoder.feed(bytes, out);
        }
    }

    /// Queues the events, and offers the replies to the reply slot, which keeps them while the
    /// probe waits and drops them otherwise, so none reaches the program as keys.
    fn deliver(&self, decoded: &mut Vec<Decoded>) {
        let shared = &self.ended.shared;
        let mut events = Vec::new();
        for decoded in decoded.drain(..) {
            match decoded {
                Decoded::Event(event) => events.push(event),
                Decoded::Reply(reply) => shared.replies.offer(reply),
            }
        }
        shared.events.push(&mut events);
    }
}

/// Appends the bytes a key record carries in [`InputMode::Vt`], in its character, as UTF-8. A
/// surrogate pair comes across two records, so the first half waits in `surrogate`. Key-up
/// records carry none.
pub(crate) fn vt_bytes(key: KeyRecord, surrogate: &mut Option<u16>, bytes: &mut Vec<u8>) {
    if !key.key_down || key.unicode == 0 {
        return;
    }
    let unit = key.unicode;
    let ch = match unit {
        0xd800..=0xdbff => {
            *surrogate = Some(unit);
            return;
        }
        0xdc00..=0xdfff => surrogate
            .take()
            .and_then(|high| char::decode_utf16([high, unit]).next())
            .and_then(Result::ok),
        _ => char::from_u32(u32::from(unit)),
    };
    if let Some(ch) = ch {
        let mut utf8 = [0; 4];
        for _ in 0..key.repeat.max(1) {
            bytes.extend_from_slice(ch.encode_utf8(&mut utf8).as_bytes());
        }
    }
}

/// Whether the ESC key-down record at `at` leads a burst of characters a terminal sent, which is
/// when the next key-down record of the batch carries a character too. A real Esc press comes in a
/// batch of its own, unless keys typed after it came while the reader was busy.
fn starts_burst(records: &[InputRecord], at: usize) -> bool {
    records
        .iter()
        .skip(at + 1)
        .find_map(|record| match record {
            InputRecord::Key(key) if !key.key_down => None,
            InputRecord::Key(key) => Some(key.unicode != 0),
            _ => Some(false),
        })
        .unwrap_or(false)
}
