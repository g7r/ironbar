use crate::channels::SyncSenderExt;
use crate::register_client;
use gtk::glib;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// Tracks the single, global inhibit state: the authoritative remaining
/// duration, plus the live countdown. The actual OS-level idle inhibitor is
/// created per-bar by the inhibit module (which owns a `gtk::Window` and lets
/// GTK translate it into a Wayland `zwp_idle_inhibitor`); this client only holds
/// the shared state so every bar's widget shows the same thing.
struct Inhibitor {
    duration: Option<Duration>,
}

impl Inhibitor {
    fn new() -> Self {
        Self { duration: None }
    }

    fn remaining(&self) -> Option<Duration> {
        self.duration
    }

    fn is_counting_down(&self) -> bool {
        self.duration
            .is_some_and(|duration| duration != Duration::MAX)
    }

    /// `Some` starts the inhibit or updates its duration; `None` stops it.
    fn set_remaining(&mut self, target: Option<Duration>) {
        self.duration = target;
    }

    /// Decrements the countdown, clearing the state at zero.
    fn tick(&mut self) {
        if let Some(duration) = &mut self.duration {
            *duration = duration.saturating_sub(Duration::from_secs(1));
        }
        if self.duration == Some(Duration::ZERO) {
            self.duration = None;
        }
    }
}

/// Process-global inhibit state: owns the live countdown in a single `glib`
/// task and broadcasts the remaining duration. Every widget subscribes to the
/// same remaining duration (so they stay in sync); each bar creates its own
/// inhibitor from the shared active state. When the timer stops, each widget
/// reverts to its currently-selected preset.
#[derive(Debug)]
pub struct Client {
    req_tx: mpsc::UnboundedSender<Option<Duration>>,
    remaining_tx: watch::Sender<Option<Duration>>,
}

impl Client {
    /// Must be called on the GTK main thread - spawns the countdown task.
    pub(crate) fn new() -> Self {
        let (req_tx, mut rx) = mpsc::unbounded_channel::<Option<Duration>>();
        let (remaining_tx, _) = watch::channel(None::<Duration>);

        glib::spawn_future_local({
            let remaining_tx = remaining_tx.clone();
            async move {
                let mut inhibitor = Inhibitor::new();

                loop {
                    tokio::select! {
                        Some(duration) = rx.recv() => inhibitor.set_remaining(duration),
                        // Timer armed only while a finite countdown runs.
                        () = glib::timeout_future_seconds(1),
                            if inhibitor.is_counting_down() => inhibitor.tick(),
                    }

                    remaining_tx.send_replace(inhibitor.remaining());
                }
            }
        });

        Self {
            req_tx,
            remaining_tx,
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<Option<Duration>> {
        self.remaining_tx.subscribe()
    }

    /// Set the inhibit duration: `Some` starts it (or updates the duration if
    /// already inhibiting), `None` stops it. `Duration::MAX` = infinite.
    pub fn set_duration(&self, duration: Option<Duration>) {
        self.req_tx.send_expect(duration);
    }
}

register_client!(Client, inhibit);
