import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  AppState,
  DeviceInfo,
  EngineConfig,
  EngineStats,
} from "./types";

export const listDevices = () => invoke<DeviceInfo[]>("list_devices");

export const getState = () => invoke<AppState>("get_state");

export const saveConfig = (config: EngineConfig) =>
  invoke<void>("save_config", { config });

export const start = (config: EngineConfig) => invoke<void>("start", { config });

export const stop = () => invoke<void>("stop");

export const getStats = () => invoke<EngineStats>("get_stats");

export const getSettings = () => invoke<AppSettings>("get_settings");

export const setSettings = (settings: AppSettings) =>
  invoke<void>("set_settings", { settings });

export function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
