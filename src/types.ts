// Mirrors crates/fan-core/src/types.rs (serde default naming).

export type DeviceKind =
  | "speakers"
  | "headphones"
  | "bluetooth"
  | "hdmi"
  | "other";

export interface DeviceInfo {
  id: string;
  name: string;
  is_default: boolean;
  kind: DeviceKind;
}

export interface OutputConfig {
  device_id: string;
  offset_ms: number;
  gain_db: number;
  muted: boolean;
}

export interface EngineConfig {
  /** null = follow the Windows default render endpoint. */
  source_device_id: string | null;
  outputs: OutputConfig[];
  master_gain_db: number;
}

export interface OutputStats {
  device_id: string;
  connected: boolean;
  latency_ms: number;
  underruns: number;
  overruns: number;
  drift_ppm: number;
  peak: number;
}

export interface EngineStats {
  running: boolean;
  source_name: string;
  source_peak: number;
  outputs: OutputStats[];
}

export interface AppSettings {
  autostart: boolean;
  start_on_launch: boolean;
  close_to_tray: boolean;
}

export interface AppState {
  config: EngineConfig;
  settings: AppSettings;
  running: boolean;
}

export const EMPTY_CONFIG: EngineConfig = {
  source_device_id: null,
  outputs: [],
  master_gain_db: 0,
};

export const DEFAULT_SETTINGS: AppSettings = {
  autostart: false,
  start_on_launch: false,
  close_to_tray: true,
};

export const EMPTY_STATS: EngineStats = {
  running: false,
  source_name: "",
  source_peak: 0,
  outputs: [],
};

export function newOutput(device_id: string): OutputConfig {
  return { device_id, offset_ms: 0, gain_db: 0, muted: false };
}
