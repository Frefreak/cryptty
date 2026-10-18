use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use super::{Lookup, Snapshot, client::Server};
use crate::terminal::{Position, Terminal};

const REFRESH_INTERVAL: Duration = Duration::from_millis(100);
const IDLE_TIMEOUT: Duration = Duration::from_secs(5);

struct Request {
    generation: u64,
    size: (usize, usize),
}

struct Response {
    generation: u64,
    snapshot: Option<Snapshot>,
}

struct Worker {
    requests: Sender<Request>,
    responses: Receiver<Response>,
}

impl Worker {
    fn spawn(
        tty: PathBuf,
        wake: Arc<dyn Fn() + Send + Sync>,
        generation: Arc<AtomicU64>,
    ) -> Self {
        let (requests, receive) = mpsc::channel::<Request>();
        let (send, responses) = mpsc::channel();
        thread::spawn(move || {
            let mut connection = None;
            while let Ok(request) = receive.recv() {
                let current =
                    || generation.load(Ordering::Relaxed) == request.generation;
                let result = (|| {
                    if connection.is_none() {
                        connection =
                            Some(Server::default().connect(&tty, &current)?);
                    }
                    Snapshot::capture(
                        connection.as_mut().unwrap(),
                        &tty,
                        request.size,
                        &current,
                    )
                })();
                if result.is_err() {
                    connection = None;
                }
                if send
                    .send(Response {
                        generation: request.generation,
                        snapshot: result.ok(),
                    })
                    .is_err()
                {
                    break;
                }
                wake();
            }
        });
        Self {
            requests,
            responses,
        }
    }
}

/// No worker or tmux process exists until a text interaction is activated.
pub struct Integration {
    tty: Option<PathBuf>,
    wake: Arc<dyn Fn() + Send + Sync>,
    worker: Option<Worker>,
    active: bool,
    generation: u64,
    worker_generation: Arc<AtomicU64>,
    pending: bool,
    dirty: bool,
    snapshot: Option<Snapshot>,
    next_request: Instant,
    idle_until: Option<Instant>,
}

impl Integration {
    pub fn new(
        tty: Option<PathBuf>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            tty,
            wake: Arc::new(wake),
            worker: None,
            active: false,
            generation: 0,
            worker_generation: Arc::new(AtomicU64::new(0)),
            pending: false,
            dirty: false,
            snapshot: None,
            next_request: Instant::now(),
            idle_until: None,
        }
    }

    pub fn invalidate(&mut self) {
        if self.active {
            self.generation = self.generation.wrapping_add(1);
            self.worker_generation
                .store(self.generation, Ordering::Relaxed);
            self.snapshot = None;
            self.dirty = true;
        }
    }

    pub fn update(&mut self, active: bool, size: (usize, usize), now: Instant) {
        let active = active && self.tty.is_some();
        if self.active != active {
            self.active = active;
            self.generation = self.generation.wrapping_add(1);
            self.worker_generation
                .store(self.generation, Ordering::Relaxed);
            self.snapshot = None;
            self.dirty = active;
            self.next_request = now;
            self.idle_until = (!active).then_some(now + IDLE_TIMEOUT);
        }
        if self.idle_until.is_some_and(|deadline| deadline <= now) {
            self.worker = None;
            self.pending = false;
            self.idle_until = None;
        }
        if let Some(worker) = &self.worker {
            while let Ok(response) = worker.responses.try_recv() {
                self.pending = false;
                if self.active && response.generation == self.generation {
                    self.snapshot = response.snapshot;
                    if self.snapshot.is_none() {
                        self.next_request = now + Duration::from_secs(1);
                    }
                }
            }
        }
        if self.active
            && self.dirty
            && !self.pending
            && now >= self.next_request
        {
            let worker = self.worker.get_or_insert_with(|| {
                Worker::spawn(
                    self.tty.clone().unwrap(),
                    self.wake.clone(),
                    self.worker_generation.clone(),
                )
            });
            let request = Request {
                generation: self.generation,
                size,
            };
            self.pending = worker.requests.send(request).is_ok();
            self.dirty = false;
            self.next_request = now + REFRESH_INTERVAL;
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        if self.active && self.dirty && !self.pending {
            Some(self.next_request)
        } else {
            self.idle_until
        }
    }

    pub fn lookup(&self, terminal: &Terminal, position: Position) -> Lookup {
        if self.active && (self.pending || self.dirty) {
            Lookup::Pending
        } else if let Some(snapshot) = &self.snapshot {
            snapshot.lookup(terminal, position)
        } else {
            Lookup::Unavailable
        }
    }
}

impl Drop for Integration {
    fn drop(&mut self) {
        self.worker_generation
            .store(self.generation.wrapping_add(1), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_integration_has_no_worker_or_deadlines() {
        let mut integration =
            Integration::new(Some("/dev/pts/1".into()), || {});
        for _ in 0..10 {
            integration.invalidate();
            integration.update(false, (80, 24), Instant::now());
            assert!(integration.worker.is_none());
            assert!(integration.deadline().is_none());
            assert!(matches!(
                integration.lookup(
                    &Terminal::new(80, 24),
                    Position { row: 0, col: 0 }
                ),
                Lookup::Unavailable
            ));
        }
    }

    #[test]
    fn releasing_control_cancels_results_and_expires_the_worker() {
        let (requests, receive) = mpsc::channel();
        let (send, responses) = mpsc::channel();
        let mut integration =
            Integration::new(Some("/dev/pts/1".into()), || {});
        integration.worker = Some(Worker {
            requests,
            responses,
        });
        let now = Instant::now();
        integration.update(true, (80, 24), now);
        let request = receive.try_recv().unwrap();
        integration.update(false, (80, 24), now);
        send.send(Response {
            generation: request.generation,
            snapshot: Some(Snapshot {
                size: (80, 24),
                panes: Vec::new(),
            }),
        })
        .unwrap();
        integration.update(false, (80, 24), now);
        assert!(integration.snapshot.is_none());
        assert!(receive.try_recv().is_err());
        integration.update(false, (80, 24), now + IDLE_TIMEOUT);
        assert!(integration.worker.is_none());
        assert!(integration.deadline().is_none());
    }
}
