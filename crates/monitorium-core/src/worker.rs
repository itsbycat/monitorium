use std::collections::BTreeMap;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded, unbounded};

use crate::backend::{Backend, Display, HardwareKind, level_to_slider, slider_to_level};
use crate::model::{Method, MonitorId, MonitorInfo, SoftwareDimMode};
use crate::protocol::{WorkerCmd, WorkerEvent};
use crate::settings::{Settings, State};

const RECENT_WRITE: Duration = Duration::from_secs(2);
const STATE_SAVE_DELAY: Duration = Duration::from_secs(2);

pub type Notify = Arc<dyn Fn() + Send + Sync>;

pub struct WorkerHandle {
    tx: Sender<WorkerCmd>,
    done: Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl WorkerHandle {
    pub fn sender(&self) -> Sender<WorkerCmd> {
        self.tx.clone()
    }

    pub fn send(&self, cmd: WorkerCmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn shutdown(mut self, timeout: Duration) {
        let _ = self.tx.send(WorkerCmd::Shutdown);
        if self.done.recv_timeout(timeout).is_ok() {
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        } else {
            log::warn!("worker did not stop within {timeout:?}");
        }
    }
}

pub fn spawn<F>(
    factory: F,
    settings: Settings,
    events: Sender<WorkerEvent>,
    notify: Notify,
) -> WorkerHandle
where
    F: FnOnce() -> Box<dyn Backend> + Send + 'static,
{
    let (tx, rx) = unbounded();
    let (done_tx, done) = bounded(1);
    let thread = thread::Builder::new()
        .name("monitorium-worker".into())
        .spawn(move || {
            let mut worker = Worker {
                backend: factory(),
                settings,
                state: State::load(),
                state_dirty_since: None,
                entries: Vec::new(),
                events,
                notify,
            };
            worker.run(&rx);
            let _ = done_tx.send(());
        })
        .expect("failed to spawn worker thread");

    WorkerHandle {
        tx,
        done,
        thread: Some(thread),
    }
}

struct Entry {
    display: Display,
    method: Method,
    value: u8,
    last_write: Option<Instant>,
}

struct Worker {
    backend: Box<dyn Backend>,
    settings: Settings,
    state: State,
    state_dirty_since: Option<Instant>,
    entries: Vec<Entry>,
    events: Sender<WorkerEvent>,
    notify: Notify,
}

impl Worker {
    fn run(&mut self, rx: &Receiver<WorkerCmd>) {
        self.refresh();
        self.emit();

        loop {
            let timeout = match self.state_dirty_since {
                Some(since) => STATE_SAVE_DELAY.saturating_sub(since.elapsed()),
                None => Duration::from_secs(3600),
            };
            let first = match rx.recv_timeout(timeout) {
                Ok(cmd) => cmd,
                Err(RecvTimeoutError::Timeout) => {
                    self.save_state();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let batch: Vec<WorkerCmd> = std::iter::once(first).chain(rx.try_iter()).collect();
            if !self.process(batch) {
                break;
            }
        }

        self.backend.clear_all_software();
        self.save_state();
    }

    fn process(&mut self, batch: Vec<WorkerCmd>) -> bool {
        let mut pending: BTreeMap<MonitorId, u8> = BTreeMap::new();
        let mut refresh = false;
        let mut reassign = false;
        let mut turn_off = false;
        let mut shutdown = false;

        for cmd in batch {
            match cmd {
                WorkerCmd::Refresh => refresh = true,
                WorkerCmd::SetBrightness { id, value } => {
                    if self.settings.link_levels {
                        self.set_all_pending(&mut pending, value.min(100));
                    } else {
                        pending.insert(id, value.min(100));
                    }
                }
                WorkerCmd::SetAll(value) => self.set_all_pending(&mut pending, value.min(100)),
                WorkerCmd::Step(delta) => self.step_pending(&mut pending, delta),
                WorkerCmd::TurnOffDisplays => turn_off = true,
                WorkerCmd::UpdateSettings(settings) => {
                    self.settings = *settings;
                    reassign = true;
                }
                // still apply what came before it: `--set` sends its value right before this
                WorkerCmd::Shutdown => {
                    shutdown = true;
                    break;
                }
            }
        }

        if refresh {
            self.refresh();
        } else if reassign {
            self.assign_methods();
        }
        let changed = !pending.is_empty();
        for (id, value) in pending {
            self.apply(&id, value);
        }
        if refresh || reassign || changed {
            self.emit();
        }
        if turn_off {
            let ddc: Vec<MonitorId> = self
                .entries
                .iter()
                .filter(|e| matches!(e.display.hardware, Some((HardwareKind::Ddc, _))))
                .map(|e| e.display.id.clone())
                .collect();
            self.backend.power_off(self.settings.power_off_method, &ddc);
        }
        !shutdown
    }

    fn visible_entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|e| e.method != Method::None && !self.settings.monitor(&e.display.id).hidden)
    }

    fn set_all_pending(&self, pending: &mut BTreeMap<MonitorId, u8>, value: u8) {
        for entry in self.visible_entries() {
            pending.insert(entry.display.id.clone(), value);
        }
    }

    fn step_pending(&self, pending: &mut BTreeMap<MonitorId, u8>, delta: i16) {
        let current = |e: &Entry| pending.get(&e.display.id).copied().unwrap_or(e.value);
        let updates: Vec<(MonitorId, u8)> = if self.settings.link_levels {
            let Some(base) = self.visible_entries().next().map(current) else {
                return;
            };
            let value = (i16::from(base) + delta).clamp(0, 100) as u8;
            self.visible_entries()
                .map(|e| (e.display.id.clone(), value))
                .collect()
        } else {
            self.visible_entries()
                .map(|e| {
                    let value = (i16::from(current(e)) + delta).clamp(0, 100) as u8;
                    (e.display.id.clone(), value)
                })
                .collect()
        };
        pending.extend(updates);
    }

    fn refresh(&mut self) {
        let displays = self.backend.enumerate();
        log::info!("found {} monitor(s)", displays.len());

        let mut old: BTreeMap<MonitorId, Entry> = self
            .entries
            .drain(..)
            .map(|e| (e.display.id.clone(), e))
            .collect();

        for display in displays {
            let previous = old.remove(&display.id);
            let recently_written = previous
                .as_ref()
                .and_then(|p| p.last_write)
                .is_some_and(|t| t.elapsed() < RECENT_WRITE);
            let value = previous.as_ref().map_or(100, |p| p.value);
            self.entries.push(Entry {
                method: Method::None,
                last_write: previous
                    .and_then(|p| p.last_write)
                    .filter(|_| recently_written),
                display,
                value,
            });
        }
        self.entries
            .sort_by_key(|e| (e.display.bounds.x, e.display.bounds.y));

        for id in old.keys() {
            self.backend.clear_software(id);
        }
        self.assign_methods();
    }

    fn method_for(&self, display: &Display) -> Method {
        let ms = self.settings.monitor(&display.id);
        if !ms.force_software {
            // monitors usually lock their backlight in HDR mode, so DDC/CI does nothing visible
            if display.hdr && display.sdr.is_some() && self.settings.hdr_sdr_brightness {
                return Method::Sdr;
            }
            match display.hardware {
                Some((HardwareKind::Ddc, _)) => return Method::Ddc,
                Some((HardwareKind::Native, _)) => return Method::Native,
                None => {}
            }
        }
        if !self.settings.software_fallback && !ms.force_software {
            return Method::None;
        }
        match self.settings.software_dim_mode {
            // gamma ramps don't apply to HDR output
            SoftwareDimMode::Gamma if !display.hdr => Method::Gamma,
            _ => Method::Overlay,
        }
    }

    fn assign_methods(&mut self) {
        for i in 0..self.entries.len() {
            let new = self.method_for(&self.entries[i].display);
            let entry = &mut self.entries[i];
            let old = std::mem::replace(&mut entry.method, new);
            let id = entry.display.id.clone();

            if old.is_software() && old != new {
                self.backend.clear_software(&id);
            }
            if new.is_software() {
                if !old.is_software() {
                    self.entries[i].value =
                        self.state.software_levels.get(&id).copied().unwrap_or(100);
                }
                let value = self.entries[i].value;
                self.write_software(&id, new, value);
            } else if old != new {
                // monitors can report the old level for a moment after a write
                let entry = &self.entries[i];
                let recently_written = entry.last_write.is_some_and(|t| t.elapsed() < RECENT_WRITE);
                if let Some(percent) = reading(&entry.display, new).filter(|_| !recently_written) {
                    let ms = self.settings.monitor(&id);
                    self.entries[i].value = level_to_slider(percent, ms.min, ms.max);
                }
            }
        }
    }

    fn apply(&mut self, id: &MonitorId, value: u8) {
        let Some(entry) = self.entries.iter_mut().find(|e| &e.display.id == id) else {
            return;
        };
        entry.value = value;
        entry.last_write = Some(Instant::now());
        let method = entry.method;
        let ms = self.settings.monitor(id);
        match method {
            Method::Ddc | Method::Native => {
                let level = slider_to_level(value, ms.min, ms.max);
                match self.backend.set_hardware(id, level) {
                    Ok(()) => {
                        if let Some((_, percent)) = &mut entry.display.hardware {
                            *percent = level;
                        }
                    }
                    Err(err) => log::warn!("setting brightness of {id} failed: {err}"),
                }
            }
            Method::Sdr => {
                let level = slider_to_level(value, ms.min, ms.max);
                match self.backend.set_sdr(id, level) {
                    Ok(()) => entry.display.sdr = Some(level),
                    Err(err) => log::warn!("setting SDR brightness of {id} failed: {err}"),
                }
            }
            Method::Overlay | Method::Gamma => self.write_software(id, method, value),
            Method::None => {}
        }
    }

    fn write_software(&mut self, id: &MonitorId, method: Method, value: u8) {
        let mode = match method {
            Method::Gamma => SoftwareDimMode::Gamma,
            _ => SoftwareDimMode::Overlay,
        };
        let ms = self.settings.monitor(id);
        let level = slider_to_level(value, ms.min, ms.max);
        if let Err(err) = self.backend.set_software(id, mode, level) {
            log::warn!("software dimming of {id} failed: {err}");
        }
        if self.state.software_levels.get(id) != Some(&value) {
            self.state.software_levels.insert(id.clone(), value);
            self.state_dirty_since.get_or_insert_with(Instant::now);
        }
    }

    fn save_state(&mut self) {
        if self.state_dirty_since.take().is_some()
            && let Err(err) = self.state.save()
        {
            log::warn!("saving state failed: {err}");
        }
    }

    fn emit(&self) {
        let monitors = self
            .entries
            .iter()
            .map(|e| {
                let ms = self.settings.monitor(&e.display.id);
                MonitorInfo {
                    id: e.display.id.clone(),
                    name: ms
                        .name
                        .clone()
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or_else(|| e.display.name.clone()),
                    hardware_name: e.display.name.clone(),
                    method: e.method,
                    brightness: e.value,
                    internal: e.display.internal,
                    hidden: ms.hidden,
                    has_hardware: e.display.hardware.is_some() || e.display.sdr.is_some(),
                    hdr: e.display.hdr,
                }
            })
            .collect();
        let _ = self.events.send(WorkerEvent::Monitors(monitors));
        (self.notify)();
    }
}

fn reading(display: &Display, method: Method) -> Option<u8> {
    match method {
        Method::Ddc | Method::Native => display.hardware.map(|(_, percent)| percent),
        Method::Sdr => display.sdr,
        Method::Overlay | Method::Gamma | Method::None => None,
    }
}
