import {can} from "@/lib/platform";

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
  /**
   * Whether this setting is offered on the current platform. Omitted means always.
   *
   * A setting the platform can't honour is worse than a missing one — it invites the user to turn
   * something on and then quietly does nothing. The notification settings use this so they never
   * appear on the web build (#348).
   */
  available?: () => boolean;
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

/** Notifications are offered only where they can actually fire. */
const onDesktop = () => can("notifications");

const NOTIFICATION_SETTINGS: SettingDef[] = [
  {
    key: "notifications.restrictions",
    group: "Notifications",
    available: onDesktop,
    label: "TMIs and ground stops",
    description:
      "Notify when a TMI, ground stop, GDP or TMU program is published — the same events the in-app restriction alerts show.",
    control: { kind: "toggle", default: false },
  },
  {
    key: "notifications.releases",
    group: "Notifications",
    available: onDesktop,
    label: "EDCT releases",
    description: "Notify when a release time is issued for a flight crossing one of your FCAs.",
    control: { kind: "toggle", default: false },
  },
  {
    key: "notifications.metering",
    group: "Notifications",
    available: onDesktop,
    label: "Heavy metering delay",
    description:
      "Notify when metering assigns a crossing more delay than the threshold below. Uses the same delay the ladder shows.",
    control: { kind: "toggle", default: false },
  },
  {
    key: "notifications.meteringDelayMin",
    group: "Notifications",
    available: onDesktop,
    label: "Delay threshold",
    description: "How much metering delay counts as worth interrupting you for.",
    control: {
      kind: "select",
      default: "15",
      options: [
        { value: "5", label: "5 minutes" },
        { value: "10", label: "10 minutes" },
        { value: "15", label: "15 minutes" },
        { value: "30", label: "30 minutes" },
      ],
    },
  },
  {
    key: "notifications.access",
    group: "Notifications",
    available: onDesktop,
    label: "Access granted",
    description: "Notify when someone grants you a new permission or role.",
    control: { kind: "toggle", default: false },
  },
  {
    key: "notifications.eventReminders",
    group: "Notifications",
    available: onDesktop,
    label: "Event reminders",
    description:
      "Notify 24 hours and 6 hours before an event you have claimed an ACE position for — the same reminders the Discord bot sends.",
    control: { kind: "toggle", default: false },
  },
];

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
    key: "map.tooltips",
    group: "Map",
    label: "Show map tooltips",
    description: "Show a details card when hovering aircraft and ATC labels on the map. Off hides every map hover card.",
    control: { kind: "toggle", default: true },
  },
  {
    key: "map.aircraftTooltips",
    group: "Map",
    label: "Aircraft hover tooltips",
    description:
      "Show a details card when hovering an aircraft, including metering times for traffic in an FCA. ATC hover cards follow “Show map tooltips” only.",
    control: { kind: "toggle", default: true },
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

  // --- Notifications (#348) ---
  // Desktop only: the web build has no OS notification centre, and `can("notifications")` is false
  // there, so these would be dead switches. Every one defaults OFF — an operator opts in to being
  // interrupted, never the other way round.
  ...NOTIFICATION_SETTINGS,
];

/** The default value for a setting key (used before the server value loads, or when signed out). */
export function settingDefault(key: string): unknown {
  return SETTINGS.find((s) => s.key === key)?.control.default;
}
