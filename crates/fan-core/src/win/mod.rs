//! Windows WASAPI engine: loopback capture fanned out to N render endpoints.

mod capture;
mod device;
mod drift;
mod notify;
mod render;
mod ring;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, IMMNotificationClient};

use crate::types::*;
use capture::{CaptureCmd, CaptureParams, Sink};
use device::{ComGuard, Format};
use render::RenderParams;
use ring::{db_to_linear, load_f32, store_f32, take_peak, OutputShared};

const TICK: Duration = Duration::from_millis(250);
const RETRY: Duration = Duration::from_secs(2);

pub fn list_outputs() -> Result<Vec<DeviceInfo>> {
    let _com = ComGuard::new()?;
    let enumerator = device::enumerator()?;
    unsafe { device::enumerate(&enumerator) }
}

pub(crate) enum Cmd {
    Apply(EngineConfig),
    DeviceAvailable(String),
    DeviceLost(String),
    DefaultRenderChanged(String),
    Stop,
}

pub(crate) struct EngineShared {
    pub(crate) stop: AtomicBool,
    pub(crate) running: AtomicBool,
    /// Linear master gain, f32 bits.
    pub(crate) master_gain: AtomicU32,
    /// Source peak since the last poll, f32 bits.
    pub(crate) source_peak: AtomicU32,
    pub(crate) source_name: RwLock<String>,
    pub(crate) outputs: RwLock<Vec<Arc<OutputShared>>>,
}

impl EngineShared {
    fn new(master_gain_db: f32) -> Self {
        Self {
            stop: AtomicBool::new(false),
            running: AtomicBool::new(false),
            master_gain: AtomicU32::new(db_to_linear(master_gain_db).to_bits()),
            source_peak: AtomicU32::new(0),
            source_name: RwLock::new(String::new()),
            outputs: RwLock::new(Vec::new()),
        }
    }
}

pub struct Engine {
    shared: Arc<EngineShared>,
    tx: Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn start(config: EngineConfig) -> Result<Self> {
        let shared = Arc::new(EngineShared::new(config.master_gain_db));
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();

        let thread_shared = shared.clone();
        let thread_tx = tx.clone();
        let thread = std::thread::Builder::new()
            .name("fan-supervisor".into())
            .spawn(move || supervise(thread_shared, thread_tx, rx, config, ready_tx))
            .map_err(|e| Error::Windows(e.to_string()))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                shared,
                tx,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(Error::Windows("engine thread exited".into()))
            }
        }
    }

    pub fn apply(&self, config: EngineConfig) -> Result<()> {
        self.tx
            .send(Cmd::Apply(config))
            .map_err(|_| Error::Windows("engine thread exited".into()))
    }

    pub fn stats(&self) -> EngineStats {
        let outputs = self.shared.outputs.read();
        EngineStats {
            running: self.shared.running.load(Ordering::Relaxed),
            source_name: self.shared.source_name.read().clone(),
            source_peak: take_peak(&self.shared.source_peak),
            outputs: outputs
                .iter()
                .map(|o| OutputStats {
                    device_id: o.device_id.clone(),
                    connected: o.connected.load(Ordering::Relaxed),
                    latency_ms: load_f32(&o.latency_ms),
                    underruns: o.underruns.load(Ordering::Relaxed),
                    overruns: o.overruns.load(Ordering::Relaxed),
                    drift_ppm: o.drift_ppm.load(Ordering::Relaxed) as f32,
                    peak: take_peak(&o.peak),
                })
                .collect(),
        }
    }

    /// Stops the engine and joins its threads. Dropping the handle does the same.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn supervise(
    shared: Arc<EngineShared>,
    tx: Sender<Cmd>,
    rx: Receiver<Cmd>,
    config: EngineConfig,
    ready: Sender<Result<()>>,
) {
    let com = match ComGuard::new() {
        Ok(com) => com,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let enumerator = match device::enumerator() {
        Ok(e) => e,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };

    let notifier: IMMNotificationClient = notify::Notifier::new(tx).into();
    let registered = unsafe { enumerator.RegisterEndpointNotificationCallback(&notifier) }.is_ok();

    let mut config = config;
    let mut graph = match Graph::build(&enumerator, &shared, &config) {
        Ok(graph) => Some(graph),
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    shared.running.store(true, Ordering::Relaxed);
    let _ = ready.send(Ok(()));

    let mut rebuild_at = Instant::now();
    loop {
        let mut rebuild = false;
        match rx.recv_timeout(TICK) {
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Cmd::Apply(next)) => {
                rebuild = next.source_device_id != config.source_device_id;
                config = next;
                store_f32(&shared.master_gain, db_to_linear(config.master_gain_db));
                if !rebuild {
                    if let Some(graph) = graph.as_mut() {
                        graph.reconcile(&shared, &config);
                    }
                }
            }
            Ok(Cmd::DefaultRenderChanged(id)) => {
                rebuild = config.source_device_id.is_none()
                    && !id.is_empty()
                    && graph.as_ref().is_some_and(|g| g.source_id != id);
            }
            Ok(Cmd::DeviceAvailable(id)) => {
                if let Some(graph) = graph.as_mut() {
                    graph.retry_now(&id);
                }
            }
            Ok(Cmd::DeviceLost(id)) => {
                if let Some(graph) = graph.as_mut() {
                    graph.drop_output(&id);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }

        // The capture thread also dies when the source endpoint goes away.
        rebuild |= graph.as_ref().is_some_and(Graph::capture_dead);
        if rebuild {
            if let Some(mut graph) = graph.take() {
                graph.shutdown();
            }
            rebuild_at = Instant::now();
        }

        if graph.is_none() && Instant::now() >= rebuild_at {
            match Graph::build(&enumerator, &shared, &config) {
                Ok(built) => graph = Some(built),
                Err(e) => {
                    log::warn!("engine rebuild failed: {e}");
                    rebuild_at = Instant::now() + RETRY;
                }
            }
        }
        if let Some(graph) = graph.as_mut() {
            graph.tick(&enumerator, &shared);
        }
        shared.running.store(graph.is_some(), Ordering::Relaxed);
    }

    if let Some(mut graph) = graph.take() {
        graph.shutdown();
    }
    shared.running.store(false, Ordering::Relaxed);
    if registered {
        unsafe {
            let _ = enumerator.UnregisterEndpointNotificationCallback(&notifier);
        }
    }
    drop(notifier);
    drop(com);
}

struct Slot {
    shared: Arc<OutputShared>,
    thread: Option<JoinHandle<()>>,
    next_retry: Instant,
}

struct Graph {
    engine: Arc<EngineShared>,
    source_id: String,
    src: Format,
    capture: Option<JoinHandle<()>>,
    capture_tx: Sender<CaptureCmd>,
    slots: Vec<Slot>,
}

impl Graph {
    fn build(
        enumerator: &IMMDeviceEnumerator,
        engine: &Arc<EngineShared>,
        config: &EngineConfig,
    ) -> Result<Self> {
        engine.stop.store(false, Ordering::Relaxed);

        let source_id = match &config.source_device_id {
            Some(id) => id.clone(),
            None => unsafe { device::default_render_id(enumerator)? },
        };
        let src = unsafe {
            let dev = device::device_by_id(enumerator, &source_id)?;
            let client = device::activate(&dev)?;
            device::mix_format(&client)?.parse()?
        };
        *engine.source_name.write() = unsafe { device::device_name(enumerator, &source_id) };

        let (capture_tx, capture_rx) = mpsc::channel();
        let mut slots = Vec::with_capacity(config.outputs.len());
        let mut sinks = Vec::with_capacity(config.outputs.len());

        for output in &config.outputs {
            let shared = Arc::new(OutputShared::new(output.device_id.clone()));
            let (producer, consumer) = ring::new_ring(&src);
            sinks.push(Sink {
                shared: shared.clone(),
                producer,
            });
            let thread = spawn_render(&output.device_id, src, consumer, &shared, engine);
            slots.push(Slot {
                shared,
                thread,
                next_retry: Instant::now() + RETRY,
            });
        }
        apply_output_settings(&slots, config);
        *engine.outputs.write() = slots.iter().map(|s| s.shared.clone()).collect();

        let engine_for_capture = engine.clone();
        let capture_source = source_id.clone();
        let capture = std::thread::Builder::new()
            .name("fan-capture".into())
            .spawn(move || {
                capture::run(CaptureParams {
                    device_id: capture_source,
                    format: src,
                    engine: engine_for_capture,
                    cmds: capture_rx,
                    sinks,
                })
            });
        let capture = match capture {
            Ok(capture) => capture,
            Err(e) => {
                engine.stop.store(true, Ordering::Relaxed);
                for slot in slots.iter_mut() {
                    if let Some(thread) = slot.thread.take() {
                        let _ = thread.join();
                    }
                }
                return Err(Error::Windows(e.to_string()));
            }
        };

        Ok(Self {
            engine: engine.clone(),
            source_id,
            src,
            capture: Some(capture),
            capture_tx,
            slots,
        })
    }

    /// Add/remove outputs and push gain, mute and offset changes without touching
    /// the threads that stay.
    fn reconcile(&mut self, engine: &Arc<EngineShared>, config: &EngineConfig) {
        let wanted: Vec<&str> = config
            .outputs
            .iter()
            .map(|o| o.device_id.as_str())
            .collect();
        for slot in self.slots.iter_mut() {
            if !wanted.contains(&slot.shared.device_id.as_str()) {
                slot.shared.stop.store(true, Ordering::Relaxed);
                if let Some(thread) = slot.thread.take() {
                    let _ = thread.join();
                }
                let _ = self
                    .capture_tx
                    .send(CaptureCmd::Remove(slot.shared.device_id.clone()));
            }
        }
        self.slots
            .retain(|s| wanted.contains(&s.shared.device_id.as_str()));

        for output in &config.outputs {
            if self
                .slots
                .iter()
                .any(|s| s.shared.device_id == output.device_id)
            {
                continue;
            }
            let shared = Arc::new(OutputShared::new(output.device_id.clone()));
            self.slots.push(Slot {
                shared,
                thread: None,
                next_retry: Instant::now(),
            });
        }

        apply_output_settings(&self.slots, config);
        *engine.outputs.write() = self.slots.iter().map(|s| s.shared.clone()).collect();
    }

    fn capture_dead(&self) -> bool {
        self.capture.as_ref().is_some_and(|t| t.is_finished())
    }

    fn retry_now(&mut self, device_id: &str) {
        for slot in self.slots.iter_mut() {
            if slot.shared.device_id == device_id {
                slot.next_retry = Instant::now();
            }
        }
    }

    fn drop_output(&mut self, device_id: &str) {
        for slot in self.slots.iter_mut() {
            if slot.shared.device_id != device_id {
                continue;
            }
            slot.shared.connected.store(false, Ordering::Relaxed);
            slot.shared.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = slot.thread.take() {
                let _ = thread.join();
            }
            slot.shared.stop.store(false, Ordering::Relaxed);
            slot.next_retry = Instant::now() + RETRY;
            let _ = self
                .capture_tx
                .send(CaptureCmd::Remove(device_id.to_string()));
        }
    }

    /// Restart render threads whose device died and has come back.
    fn tick(&mut self, enumerator: &IMMDeviceEnumerator, engine: &Arc<EngineShared>) {
        let now = Instant::now();
        for slot in self.slots.iter_mut() {
            if slot.thread.as_ref().is_some_and(|t| t.is_finished()) {
                if let Some(thread) = slot.thread.take() {
                    let _ = thread.join();
                }
                slot.next_retry = now + RETRY;
            }
            if slot.thread.is_some() || now < slot.next_retry {
                continue;
            }
            if !unsafe { device::is_active(enumerator, &slot.shared.device_id) } {
                slot.next_retry = now + RETRY;
                continue;
            }
            let (producer, consumer) = ring::new_ring(&self.src);
            let _ = self.capture_tx.send(CaptureCmd::Add(Sink {
                shared: slot.shared.clone(),
                producer,
            }));
            slot.shared.stop.store(false, Ordering::Relaxed);
            slot.thread = spawn_render(
                &slot.shared.device_id,
                self.src,
                consumer,
                &slot.shared,
                engine,
            );
            slot.next_retry = now + RETRY;
        }
    }

    fn shutdown(&mut self) {
        self.engine.stop.store(true, Ordering::Relaxed);
        for slot in self.slots.iter_mut() {
            slot.shared.stop.store(true, Ordering::Relaxed);
        }
        if let Some(capture) = self.capture.take() {
            let _ = capture.join();
        }
        for slot in self.slots.iter_mut() {
            if let Some(thread) = slot.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

fn spawn_render(
    device_id: &str,
    src: Format,
    consumer: rtrb::Consumer<f32>,
    shared: &Arc<OutputShared>,
    engine: &Arc<EngineShared>,
) -> Option<JoinHandle<()>> {
    let params = RenderParams {
        device_id: device_id.to_string(),
        src,
        consumer,
        shared: shared.clone(),
        engine: engine.clone(),
    };
    std::thread::Builder::new()
        .name(format!("fan-render-{device_id}"))
        .spawn(move || render::run(params))
        .map_err(|e| log::warn!("cannot spawn render thread: {e}"))
        .ok()
}

/// Offsets are relative: the most negative one becomes zero extra delay.
fn apply_output_settings(slots: &[Slot], config: &EngineConfig) {
    let floor = config
        .outputs
        .iter()
        .map(|o| o.offset_ms)
        .min()
        .unwrap_or(0)
        .min(0);
    for output in &config.outputs {
        let Some(slot) = slots
            .iter()
            .find(|s| s.shared.device_id == output.device_id)
        else {
            continue;
        };
        store_f32(&slot.shared.gain, db_to_linear(output.gain_db));
        slot.shared.muted.store(output.muted, Ordering::Relaxed);
        slot.shared
            .delay_ms
            .store((output.offset_ms - floor).max(0) as u32, Ordering::Relaxed);
    }
}
