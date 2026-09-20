import {can} from "@/lib/platform";
import {HOTKEY_ACTIONS} from "@/lib/hotkeys";

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

/**
 * A keyboard shortcut, recorded by pressing it.
 *
 * Not a text field: expecting someone to know that `CommandOrControl+Shift+O` is the literal string
 * to type is a developer's mental model. Pressing the combination is what people actually do.
 */
export interface HotkeyControl {
  kind: "hotkey";
  default: string;
  placeholder?: string;
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
  control: ToggleControl | SelectControl | HotkeyControl;
}

/** The opaque per-user jsonb blob shape (key → value) stored under the "settings" namespace. */
export type SettingsBlob = Record<string, unknown>;

/** Settings namespace for the preferences API (`/api/v1/me/preferences/settings`). */
export const SETTINGS_NAMESPACE = "settings";

// A switch the platform can't honour is worse than a missing one, so each desktop-only group is
// gated on *its own* capability rather than on "is this the desktop build".
const whenNotifications = () => can("notifications");
const whenTray = () => can("tray");

const NOTIFICATION_SETTINGS: SettingDef[] = [
  {
    key: "notifications.restrictions",
    group: "Notifications",
    available: whenNotifications,
    label: "TMIs and ground stops",
    description:
      "Notify when a TMI, ground stop, GDP or TMU program is published — the same events the in-app restriction alerts show.",
    control: { kind: "toggle", default: false },
  },
  {
    key: "notifications.releases",
    group: "Notifications",
    available: whenNotifications,
    label: "EDCT releases",
    description: "Notify when a release time is issued for a flight crossing one of your FCAs.",
    control: { kind: "toggle", default: false },
  },
  {
    key: "notifications.metering",
    group: "Notifications",
    available: whenNotifications,
    label: "Heavy metering delay",
    description:
      "Notify when metering assigns a crossing more delay than the threshold below. Uses the same delay the ladder shows.",
    control: { kind: "toggle", default: false },
  },
  {
    key: "notifications.meteringDelayMin",
    group: "Notifications",
    available: whenNotifications,
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
    available: whenNotifications,
    label: "Access granted",
    description: "Notify when someone grants you a new permission or role.",
    control: { kind: "toggle", default: false },
  },
  {
    key: "notifications.eventReminders",
    group: "Notifications",
    available: whenNotifications,
    label: "Event reminders",
    description:
      "Notify 24 hours and 6 hours before an event you have claimed an ACE position for — the same reminders the Discord bot sends.",
    control: { kind: "toggle", default: false },
  },
];

const TRAY_SETTINGS: SettingDef[] = [
  {
    key: "tray.show",
    group: "Menu bar",
    available: whenTray,
    label: "Show OIS in the menu bar",
    description:
      "A menu-bar icon with pilots online, active TMIs and feed health, plus quick links into the app.",
    control: { kind: "toggle", default: false },
  },
  {
    key: "tray.closeToTray",
    group: "Menu bar",
    available: whenTray,
    label: "Closing the window keeps OIS running",
    description:
      "Closing hides the window to the menu bar instead of quitting, so notifications keep arriving. Quit from the menu-bar icon.",
    control: { kind: "toggle", default: false },
  },
  {
    key: "tray.launchAtLogin",
    group: "Menu bar",
    available: whenTray,
    label: "Launch at login",
    description: "Start OIS automatically when you sign in to this computer.",
    control: { kind: "toggle", default: false },
  },
];

const whenHotkeys = () => can("globalHotkeys");

/**
 * One typed accelerator per action.
 *
 * Every one defaults to **empty**: a global shortcut takes a key combination away from every other
 * application on the machine, and nobody should have that happen to them by installing an update.
 */
const HOTKEY_SETTINGS: SettingDef[] = HOTKEY_ACTIONS.map(({settingKey, label}) => ({
  key: settingKey,
  group: "Shortcuts",
  available: whenHotkeys,
  label,
  description: undefined,
  control: {
    kind: "hotkey",
    default: "",
    placeholder: "Click, then press a shortcut",
  },
}));

const whenSounds = () => can("audioAlerts");

/** The categories that can make a noise — the same ones that can notify (#348). */
const SOUND_CATEGORIES: {key: string; label: string}[] = [
  {key: "restrictions", label: "TMIs and ground stops"},
  {key: "releases", label: "EDCT releases"},
  {key: "metering", label: "Heavy metering delay"},
  {key: "access", label: "Access granted"},
  {key: "eventReminders", label: "Event reminders"},
];

/**
 * A toggle and a volume per category.
 *
 * Every one is off by default: installing an update should never start making noise at someone.
 */
const SOUND_SETTINGS: SettingDef[] = SOUND_CATEGORIES.flatMap(({key, label}) => [
  {
    key: `sounds.${key}`,
    group: "Sounds",
    available: whenSounds,
    label: `Play a sound for ${label.toLowerCase()}`,
    control: {kind: "toggle", default: false},
  },
  {
    key: `sounds.${key}.volume`,
    group: "Sounds",
    available: whenSounds,
    label: `${label} volume`,
    control: {
      kind: "select",
      default: "normal",
      options: [
        {value: "quiet", label: "Quiet"},
        {value: "normal", label: "Normal"},
        {value: "loud", label: "Loud"},
      ],
    },
  },
]);

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

  // --- Menu bar (#351) ---
  ...TRAY_SETTINGS,

  // --- Shortcuts (#352) ---
  ...HOTKEY_SETTINGS,

  // --- Sounds (#353) ---
  ...SOUND_SETTINGS,
];

/** The default value for a setting key (used before the server value loads, or when signed out). */
export function settingDefault(key: string): unknown {
  return SETTINGS.find((s) => s.key === key)?.control.default;
}
