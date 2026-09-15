/**
 * The user-settings registry — the single place to define a setting. Add an entry here and it shows up
 * on the /settings page automatically; read it anywhere with `useSetting(key, default)`.
 *
 * Values are stored server-side (per user) as one opaque jsonb blob under the "settings" preferences
 * namespace, so adding a setting needs no migration.
 */

/** A boolean on/off setting rendered as a toggle switch. */
export interface ToggleControl {
  kind: "toggle";
  default: boolean;
}
/** A one-of setting rendered as a dropdown. */
export interface SelectControl {
  kind: "select";
  default: string;
  options: { value: string; label: string }[];
}

export interface SettingDef {
  /** Stable storage key, dotted by area, e.g. "map.persistView". */
  key: string;
  /** Section heading it appears under on the settings page. */
  group: string;
  label: string;
  description?: string;
  control: ToggleControl | SelectControl;
}

/** The opaque per-user jsonb blob shape (key → value) stored under the "settings" namespace. */
export type SettingsBlob = Record<string, unknown>;

/** Settings namespace for the preferences API (`/api/v1/me/preferences/settings`). */
export const SETTINGS_NAMESPACE = "settings";

/** Every setting, in display order. Groups render in first-seen order. */
export const SETTINGS: SettingDef[] = [
  {
    key: "map.persistView",
    group: "Map",
    label: "Remember map position",
    description:
      "Keep the last pan and zoom on the FCA map and dashboard maps across page refreshes and new tabs.",
    control: { kind: "toggle", default: true },
  },
  {
    key: "map.dynamicAircraftScale",
    group: "Map",
    label: "Scale aircraft with zoom",
    description:
      "Grow aircraft icons as you zoom in and shrink them when zoomed out, instead of a fixed size. Off keeps every icon a constant size.",
    control: { kind: "toggle", default: true },
  },
  {
    key: "map.aircraftIconSize",
    group: "Map",
    label: "Aircraft icon size",
    description: "Scale aircraft symbols on the map up or down.",
    control: {
      kind: "select",
      default: "100",
      options: [
        { value: "60", label: "Small (60%)" },
        { value: "80", label: "Smaller (80%)" },
        { value: "100", label: "Default (100%)" },
        { value: "125", label: "Larger (125%)" },
      ],
    },
  },
  {
    key: "debug.enabled",
    group: "Advanced",
    label: "Debug mode",
    description:
      "Show extra developer detail across OIS — for FCA traffic, the aircraft performance profile used for each ETA, the speeds and wind applied, and any filed-route tokens that failed to resolve.",
    control: { kind: "toggle", default: false },
  },
  {
    key: "debug.predictionScrubber",
    group: "Advanced",
    label: "Prediction scrubber (map)",
    description:
      "Requires Debug mode. Adds a forward-in-time scrubber to the flow map — scrubbing ahead projects every aircraft along its resolved route using the same ETA model as metering, instead of showing live positions.",
    control: { kind: "toggle", default: false },
  },
];

/** The default value for a setting key (used before the server value loads, or when signed out). */
export function settingDefault(key: string): unknown {
  return SETTINGS.find((s) => s.key === key)?.control.default;
}
