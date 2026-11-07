// Typed wrappers over the app's Tauri commands.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Kind = "command" | "automation";
export type Command = Record<string, unknown> & { name?: string; command?: string };

export interface AppInfo {
  version: string;
  platform: string;
  language: string;
  dataDir: string;
  computerName: string;
  hostname: string;
  backgroundSupported: boolean;
}

export interface InstalledApp {
  name: string;
  command: string;
}

export interface RunRecord {
  id: string;
  at: number;
  source: string;
  kind: Kind;
  target: string;
  name: string;
  params: string | null;
  status: "queued" | "running" | "succeeded" | "failed" | "offline" | "timed_out";
  exitCode: number | null;
  output: string;
  durationMs: number;
}

export type Weekday = "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun";
export type Trigger =
  | { type: "schedule"; at: string; days: Weekday[] }
  | { type: "interval"; minutes: number }
  | { type: "startup" }
  | { type: "wake" }
  | { type: "network"; network: string; event: "joined" | "left" }
  | { type: "file"; path: string; recursive: boolean }
  | { type: "usb"; event: "connected" | "disconnected"; name: string }
  | { type: "display"; event: "connected" | "disconnected" }
  | { type: "webhook"; hook: string };
export type Condition =
  | { type: "network"; network: string; not: boolean }
  | { type: "time"; after: string; before: string }
  | { type: "days"; days: Weekday[] };
export type Step =
  | { type: "command"; command: string; params: string }
  | { type: "shell"; line: string }
  | { type: "wait"; seconds: number }
  | { type: "wakeOnLan"; mac: string; broadcast: string }
  | { type: "notify"; title: string; body: string };

export interface Automation {
  id: string;
  name: string;
  enabled: boolean;
  icon: string;
  description: string;
  ground: "foreground" | "background";
  triggers: Trigger[];
  conditions: Condition[];
  steps: Step[];
  stopOnError: boolean;
  allowParams: boolean;
}

export interface HomeKitConfig {
  enabled: boolean;
  bridgeName: string;
  pin: string;
  username: string;
  debugLogging: boolean;
}
export interface BridgeStatus {
  running: boolean;
  paired: boolean;
  controllers: number;
  lights: number;
  port: number;
  error: string | null;
}
export interface MqttConfig {
  enabled: boolean;
  host: string;
  port: number;
  username: string;
  password: string;
  discoveryPrefix: string;
}
export interface MqttStatus {
  connected: boolean;
  error: string | null;
  entities: number;
}
export interface ApiConfig {
  enabled: boolean;
  port: number;
  allowLan: boolean;
  key: string;
}
export interface ApiStatus {
  running: boolean;
  address: string | null;
  error: string | null;
}
export type LinkStatus =
  | { state: "off" }
  | { state: "connecting" }
  | { state: "online" }
  | { state: "offline"; error: string }
  | { state: "rejected"; message: string };
export interface PairingInfo {
  code: string;
  url: string;
  qrSvg: string;
  expiresIn: number;
  hosts: string[];
  port: number;
  pairedWith: string | null;
}
export interface PhoneView {
  id: string;
  name: string;
  pairedAt: number;
  lastSeen: number | null;
}
export interface NetInterface {
  name: string;
  mac: string;
  ip: string;
  primary: boolean;
}
export interface PhoneInfo {
  enabled: boolean;
  port: number;
  phones: PhoneView[];
  status: { running: boolean; port: number | null; error: string | null; pairing: PairingInfo | null } | null;
  interfaces: NetInterface[];
  wakeForNetwork: boolean | null;
  platform: string;
}
export interface ServerInfo {
  url: string;
  email: string;
  computerId: string;
  status: LinkStatus | null;
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  setLanguage: (code: string) => invoke<void>("set_language", { code }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  generalGet: () => invoke<{ computerName: string; hostname: string; autostart: boolean; macAddresses: string[] }>("general_get"),
  generalSave: (computerName: string, autostart: boolean) => invoke<void>("general_save", { computerName, autostart }),
  loadCommands: () => invoke<Command[]>("load_commands"),
  saveCommands: (commands: Command[]) => invoke<void>("save_commands", { commands }),
  readCommandsText: () => invoke<string>("read_commands_text"),
  writeCommandsText: (text: string) => invoke<number>("write_commands_text", { text }),
  examples: () => invoke<Command[]>("examples"),
  run: (kind: Kind, target: string, params?: string | null) => invoke<RunRecord>("run_item", { kind, target, params: params ?? null }),
  history: (limit = 200) => invoke<RunRecord[]>("history", { limit }),
  clearHistory: () => invoke<void>("clear_history"),
  loadAutomations: () => invoke<Automation[]>("load_automations"),
  saveAutomations: (automations: Automation[]) => invoke<void>("save_automations", { automations }),
  newId: (length = 10) => invoke<string>("new_id", { length }),
  currentNetwork: () => invoke<{ id: string; label: string } | null>("current_network"),
  installedApps: () => invoke<InstalledApp[]>("installed_apps"),
  usbDevices: () => invoke<string[]>("usb_devices"),
  wake: (mac: string) => invoke<void>("wake", { mac }),
  homekitGet: () => invoke<{ config: HomeKitConfig; status: BridgeStatus | null }>("homekit_get"),
  homekitSave: (config: HomeKitConfig) => invoke<{ config: HomeKitConfig; status: BridgeStatus | null }>("homekit_save", { config }),
  mqttGet: () => invoke<{ config: MqttConfig; status: MqttStatus | null }>("mqtt_get"),
  mqttSave: (config: MqttConfig) => invoke<{ config: MqttConfig; status: MqttStatus | null }>("mqtt_save", { config }),
  apiGet: () => invoke<{ config: ApiConfig; status: ApiStatus | null; executable: string }>("api_get"),
  apiSave: (enabled: boolean, port: number, allowLan: boolean, newKey: boolean) =>
    invoke<{ config: ApiConfig; status: ApiStatus | null; executable: string }>("api_save", { enabled, port, allowLan, newKey }),
  phoneGet: () => invoke<PhoneInfo>("phone_get"),
  phoneSave: (enabled: boolean) => invoke<PhoneInfo>("phone_save", { enabled }),
  phonePairStart: () => invoke<PairingInfo>("phone_pair_start"),
  phonePairCancel: () => invoke<void>("phone_pair_cancel"),
  phoneRemove: (id: string) => invoke<PhoneInfo>("phone_remove", { id }),
  wakeAccessEnable: () => invoke<PhoneInfo>("wake_access_enable"),
  serverGet: () => invoke<ServerInfo>("server_get"),
  serverSignIn: (url: string, email: string, password: string, create: boolean) =>
    invoke<ServerInfo>("server_sign_in", { url, email, password, create }),
  serverSignOut: () => invoke<ServerInfo>("server_sign_out"),
};

export function on<T = unknown>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => handler(e.payload));
}

export function flag(value: unknown): boolean {
  return value === true || value === "true" || value === 1 || value === "1";
}
