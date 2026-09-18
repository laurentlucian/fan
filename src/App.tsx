import { useEffect, useRef, useState } from "react";
import DeviceRow from "./components/DeviceRow";
import ErrorBanner from "./components/ErrorBanner";
import Meter from "./components/Meter";
import SettingsPopover from "./components/SettingsPopover";
import * as api from "./api";
import {
  DEFAULT_SETTINGS,
  EMPTY_CONFIG,
  EMPTY_STATS,
  newOutput,
  type AppSettings,
  type DeviceInfo,
  type EngineConfig,
  type EngineStats,
  type OutputConfig,
} from "./types";
import "./App.css";

const POLL_MS = 25;

type Levels = { source: number; by: Record<string, number> };
const NO_LEVELS: Levels = { source: 0, by: {} };

// Peak 0..1 to bar fill, -40 dBFS floor.
const norm = (peak: number) =>
  peak <= 0 ? 0 : Math.min(1, Math.max(0, 1 + Math.log10(peak) / 2));
const decay = (prev: number, next: number) => Math.max(next, prev * 0.85);

export default function App() {
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [config, setConfig] = useState<EngineConfig>(EMPTY_CONFIG);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [running, setRunning] = useState(false);
  const [stats, setStats] = useState<EngineStats>(EMPTY_STATS);
  const [levels, setLevels] = useState<Levels>(NO_LEVELS);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const pending = useRef<EngineConfig | null>(null);
  const saveTimer = useRef<number | undefined>(undefined);
  // Bumped on every start/stop so a stats reply in flight can't revive old state.
  const epoch = useRef(0);

  useEffect(() => {
    (async () => {
      try {
        setDevices(await api.listDevices());
      } catch (e) {
        setError(api.errText(e));
      }
      try {
        const s = await api.getState();
        setConfig(s.config);
        setSettings(s.settings);
        setRunning(s.running);
      } catch (e) {
        setError((prev) => prev ?? api.errText(e));
      }
    })();
  }, []);

  useEffect(() => {
    if (!running) {
      setLevels(NO_LEVELS);
      return;
    }
    let alive = true;
    let inFlight = false;
    let timer: number | undefined;

    const tick = async () => {
      if (inFlight) return;
      inFlight = true;
      const at = epoch.current;
      try {
        const s = await api.getStats();
        if (!alive || at !== epoch.current) return;
        setStats(s);
        setRunning(s.running);
        setLevels((prev) => {
          const by: Record<string, number> = {};
          for (const o of s.outputs) {
            by[o.device_id] = decay(prev.by[o.device_id] ?? 0, norm(o.peak));
          }
          return { source: decay(prev.source, norm(s.source_peak)), by };
        });
      } catch (e) {
        if (alive) setError(api.errText(e));
      } finally {
        inFlight = false;
      }
    };

    const play = () => {
      if (timer === undefined) timer = window.setInterval(tick, POLL_MS);
    };
    const pause = () => {
      window.clearInterval(timer);
      timer = undefined;
    };
    const visibility = () => (document.hidden ? pause() : play());

    visibility();
    document.addEventListener("visibilitychange", visibility);
    return () => {
      alive = false;
      pause();
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [running]);

  const push = (next: EngineConfig) => {
    setConfig(next);
    pending.current = next;
    if (saveTimer.current !== undefined) return;
    saveTimer.current = window.setTimeout(async () => {
      saveTimer.current = undefined;
      const c = pending.current;
      pending.current = null;
      if (!c) return;
      try {
        await api.saveConfig(c);
      } catch (e) {
        setError(api.errText(e));
      }
    }, 120);
  };

  const source = devices.find((d) => d.is_default);
  const outputs = devices.filter((d) => !d.is_default);
  const byId = new Map(config.outputs.map((o) => [o.device_id, o]));
  const latency = stats.outputs.length
    ? Math.round(Math.max(...stats.outputs.map((o) => o.latency_ms)))
    : 0;
  const drops = stats.outputs.reduce(
    (n, o) => n + o.underruns + o.overruns,
    0,
  );

  return (
    <main className="app">
      <header className="head">
        <span className="brand">FAN</span>
        <SettingsPopover
          settings={settings}
          onChange={async (patch) => {
            const next = { ...settings, ...patch };
            setSettings(next);
            try {
              await api.setSettings(next);
            } catch (e) {
              setError(api.errText(e));
            }
          }}
        />
      </header>

      {error && (
        <ErrorBanner text={error} onDismiss={() => setError(null)} />
      )}

      <div className="source">
        <span className="label">Source</span>
        <span className="name">{source?.name ?? stats.source_name ?? "—"}</span>
        <Meter level={levels.source} wide />
      </div>

      <div className="list">
        {outputs.length === 0 ? (
          <p className="empty">No outputs</p>
        ) : (
          outputs.map((d) => (
            <DeviceRow
              key={d.id}
              device={d}
              output={byId.get(d.id)}
              level={levels.by[d.id] ?? 0}
              stats={stats.outputs.find((o) => o.device_id === d.id)}
              running={running}
              onToggle={(on) =>
                push({
                  ...config,
                  outputs: on
                    ? [...config.outputs, newOutput(d.id)]
                    : config.outputs.filter((o) => o.device_id !== d.id),
                })
              }
              onChange={(patch: Partial<OutputConfig>) =>
                push({
                  ...config,
                  outputs: config.outputs.map((o) =>
                    o.device_id === d.id ? { ...o, ...patch } : o,
                  ),
                })
              }
            />
          ))
        )}
      </div>

      <footer className="foot">
        <button
          className="primary"
          data-running={running || undefined}
          disabled={busy || (!running && config.outputs.length === 0)}
          onClick={async () => {
            setBusy(true);
            epoch.current++;
            try {
              if (running) {
                await api.stop();
                setRunning(false);
                setStats(EMPTY_STATS);
              } else {
                await api.start(config);
                setRunning(true);
              }
              setError(null);
            } catch (e) {
              setError(api.errText(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          {running ? "Stop" : "Start"}
        </button>
        <span className="status">
          {running
            ? `${latency} ms · ${drops} drops`
            : `${config.outputs.length} selected`}
        </span>
      </footer>
    </main>
  );
}
