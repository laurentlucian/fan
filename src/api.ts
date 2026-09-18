import { invoke } from "@tauri-apps/api/core";
import * as mock from "./mock";
import type {
  AppSettings,
  AppState,
  DeviceInfo,
  EngineConfig,
  EngineStats,
} from "./types";

const native = () => "__TAURI_INTERNALS__" in window;

export const listDevices = () =>
  native() ? invoke<DeviceInfo[]>("list_devices") : mock.listDevices();

export const getState = () =>
  native() ? invoke<AppState>("get_state") : mock.getState();

export const saveConfig = (config: EngineConfig) =>
  native() ? invoke<void>("save_config", { config }) : mock.saveConfig(config);

export const start = (config: EngineConfig) =>
  native() ? invoke<void>("start", { config }) : mock.start(config);

export const stop = () => (native() ? invoke<void>("stop") : mock.stop());

export const getStats = () =>
  native() ? invoke<EngineStats>("get_stats") : mock.getStats();

export const getSettings = () =>
  native() ? invoke<AppSettings>("get_settings") : mock.getSettings();

export const setSettings = (settings: AppSettings) =>
  native()
    ? invoke<void>("set_settings", { settings })
    : mock.setSettings(settings);

export function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
