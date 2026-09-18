import {
  DEFAULT_SETTINGS,
  EMPTY_CONFIG,
  type AppSettings,
  type AppState,
  type DeviceInfo,
  type EngineConfig,
  type EngineStats,
} from "./types";

const devices: DeviceInfo[] = [
  { id: "src", name: "Speakers (Realtek)", is_default: true, kind: "speakers" },
  { id: "hp", name: "WH-1000XM5", is_default: false, kind: "headphones" },
  { id: "tv", name: "LG CX", is_default: false, kind: "hdmi" },
  { id: "bt", name: "JBL Charge", is_default: false, kind: "bluetooth" },
];

let settings: AppSettings = { ...DEFAULT_SETTINGS };
let config: EngineConfig = {
  ...EMPTY_CONFIG,
  outputs: [
    { device_id: "hp", offset_ms: 0, gain_db: 0, muted: false },
    { device_id: "tv", offset_ms: 20, gain_db: -3, muted: false },
  ],
};
let running = false;

const peak = (id: string) => {
  if (!running) return 0;
  const t = Date.now() / 1000;
  const seed = id.charCodeAt(0) * 0.37;
  return Math.min(1, 0.15 + 0.7 * Math.abs(Math.sin(t * 2.2 + seed)));
};

export const listDevices = async () => devices;

export const getState = async (): Promise<AppState> => ({
  config,
  settings,
  running,
});

export const saveConfig = async (next: EngineConfig) => {
  config = next;
};

export const start = async (next: EngineConfig) => {
  config = next;
  running = true;
};

export const stop = async () => {
  running = false;
};

export const getStats = async (): Promise<EngineStats> => ({
  running,
  source_name: devices[0].name,
  source_peak: peak("src"),
  outputs: config.outputs.map((o) => ({
    device_id: o.device_id,
    connected: true,
    latency_ms: o.device_id === "tv" ? 28 : 8,
    underruns: 0,
    overruns: 0,
    drift_ppm: 0,
    peak: o.muted ? 0 : peak(o.device_id),
  })),
});

export const getSettings = async () => settings;

export const setSettings = async (next: AppSettings) => {
  settings = next;
};
