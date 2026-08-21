// Top-down aircraft silhouette icons, keyed by ICAO type designator.
//
// Artwork and the type→icon mapping are adapted from VATSIM Radar
// (https://github.com/VATSIM-Radar/vatsim-radar), used under CC BY-NC 4.0.
// See web/src/assets/aircraft/ATTRIBUTION.md. OIS is a non-commercial VATUSA
// project; the map surfaces a "planes by VATSIM Radar" credit for attribution.

// Vite bundles each vendored silhouette and hands us its hashed URL.
const modules = import.meta.glob("../assets/aircraft/*.svg", {
  eager: true,
  query: "?url",
  import: "default",
}) as Record<string, string>;

/** icon key (e.g. "a320") → served URL. */
const ICON_URL: Record<string, string> = {};
for (const [path, url] of Object.entries(modules)) {
  const key = path.split("/").pop()!.replace(/\.svg$/, "");
  ICON_URL[key] = url;
}

const FALLBACK = "a320";

// Type designators that don't have their own silhouette but share another's.
// Ported verbatim from VATSIM Radar's getAircraftIcon() switch so common
// variants (helicopters, sub-types, freighters) still get a sensible shape.
const ALIAS_GROUPS: [string, string[]][] = [
  ["an26", ["AN32"]],
  ["vl3", ["VL3T"]],
  ["pa39", ["PA30"]],
  ["il18", ["IL38"]],
  ["pts1", ["PTS2"]],
  ["fdct", ["FD2E", "FDF2", "FDMC"]],
  ["uh1", ["UH1Y"]],
  ["b703", ["E6", "R135"]],
  ["as32", ["AS32", "H225", "EC225"]],
  ["b06", ["B06T"]],
  ["b407", ["B427", "B429", "B430", "A109", "A119", "A129", "A2RT", "ALH", "ALO2", "ALO3", "AS3B", "AS55", "AS65", "B212", "B214", "B222", "B230", "B412", "B47G", "B47J", "BK17", "BRB2", "BSTP", "CH7", "CHIF", "DJIN", "DYH2", "ELTO", "EXPL", "FREL", "GAZL", "H12T", "H269", "H500", "H53", "H53S", "HUCO", "K126", "K226", "KA25", "KA26", "KA50", "KA52", "KA62", "KMAX", "LAMA", "LR2T", "LYNX", "M74", "MD52", "MD60", "MH20", "MI10", "MI2", "MI24", "MI26", "MI28", "MI34", "MI38", "MI8", "NA40", "NH90", "PHIL", "PUMA", "R4", "RMOU", "RP1", "RVAL", "S274", "S278", "S330", "S51", "S52", "S55P", "S58P", "S61", "S61R", "S62", "S64", "TIGR", "UH12", "ULTS", "V500", "W3", "WASP", "WG30", "X2", "X49", "YNHL", "ZA6", "HELO", "HELI"]],
  ["g2ca", ["EN28"]],
  ["r22", ["ES11", "EXEC", "EXEJ"]],
  ["r66", ["FH11"]],
  ["as50", ["PSW4"]],
  ["f18", ["F18H", "F18S"]],
  ["c130", ["C30J", "E390"]],
  ["a3st", ["A337"]],
  ["cl30", ["CL35"]],
  ["h160", ["EC75", "H175"]],
  ["f16", ["T38", "F5"]],
  ["f27", ["F50", "FH27"]],
  ["l159", ["M346", "A29", "AMX", "T27"]],
  ["c510", ["C550"]],
  ["h64", ["SUCO"]],
  ["dc6", ["P3", "L188"]],
  ["mir2", ["MRF1"]],
  ["da40", ["DA50"]],
  ["b722", ["R722"]],
  ["c700", ["C680"]],
  ["glf5", ["GLF4"]],
  ["ship", ["B314"]],
  ["be35", ["T34P", "T34T"]],
  ["e145", ["E45X"]],
  ["c208", ["U20T"]],
  ["c46", ["E110"]],
  ["c160", ["C295"]],
  ["g91", ["MG21"]],
  ["aa5", ["AC11"]],
  ["sr22", ["COL4", "S22T", "SR22T", "SR20"]],
  ["f35", ["VF35"]],
  ["c206", ["C207", "C210"]],
  ["m20p", ["M20T", "M20"]],
  ["pa18", ["CC19"]],
  ["su27", ["SU30", "SU32", "SU33", "SU34", "SU35", "SU37"]],
  ["twen", ["P208"]],
  ["glid", ["A33E", "DISC", "JS3J", "JS3E", "LS4", "LS8", "DG80", "DG1T"]],
  ["be36", ["BT36", "B36T"]],
  ["k35e", ["K35R"]],
  ["e3cf", ["E3TF"]],
  ["b39m", ["B3XM"]],
  ["b738", ["P8"]],
  ["be9l", ["BE9T"]],
  ["c402", ["C401"]],
  ["evot", ["EVOP"]],
  ["tris", ["TRI"]],
  ["e75s", ["E750", "E755", "E75L"]],
  ["pite", ["PITA"]],
  ["a300", ["A306", "A30B", "A30F"]],
  ["a321", ["A21N"]],
  ["md11", ["MD1F"]],
  ["md80", ["MD82", "MD83", "MD88"]],
  ["b77l", ["B77F"]],
  ["b74s", ["B74F"]],
  ["b748", ["B48F"]],
  ["b741", ["B742", "B743"]],
  ["tbm7", ["BRAV", "ECHO", "GLST"]],
  ["a139", ["AW139", "VH139", "MH139", "UH139", "HH139"]],
  ["a149", ["AW149"]],
  ["a169", ["H169", "AW169"]],
  ["a189", ["H189", "AW189"]],
  ["ec45", ["H135", "H145", "EC20", "EC25", "EC30", "EC35", "EC55"]],
  ["dv20", ["DA20"]],
  ["eh10", ["AW101"]],
  ["c750", ["C700"]],
  ["b461", ["B461", "RJ70", "RJ85", "RJ1H"]],
  ["c152", ["C150", "C162"]],
  ["c172", ["C175", "C170", "C182", "C185", "C180"]],
  ["dh8a", ["DH8A", "DH8B"]],
  ["be58", ["B58T"]],
  ["kodi", ["K100"]],
  ["be60", ["B60T"]],
  ["gl5t", ["GL6T"]],
  ["at4x", ["AT42", "AT43", "AT45", "AT46"]],
  ["at7x", ["AT72", "AT73", "AT75", "AT76"]],
];

const ALIAS: Record<string, string> = {};
for (const [target, sources] of ALIAS_GROUPS) {
  for (const s of sources) ALIAS[s] = target;
}

const keyCache = new Map<string, string>();

/**
 * Resolve a flight-plan aircraft type (e.g. "B738/L", "A320", "C172") to a silhouette icon key.
 * Mirrors VATSIM Radar's precedence: PA-28 prefix → explicit alias → exact type file → A320 fallback.
 */
export function resolveIconKey(actype: string | null | undefined): string {
  const key = (actype ?? "").toUpperCase().split("/")[0].trim();
  const cached = keyCache.get(key);
  if (cached) return cached;

  let iconKey: string;
  if (key.startsWith("P28")) iconKey = "p28x";
  else if (ALIAS[key]) iconKey = ALIAS[key];
  else if (ICON_URL[key.toLowerCase()]) iconKey = key.toLowerCase();
  else iconKey = FALLBACK;

  keyCache.set(key, iconKey);
  return iconKey;
}

/** Resolve a flight-plan aircraft type to its served silhouette icon URL. */
export function aircraftIconUrl(actype: string | null | undefined): string {
  const iconKey = resolveIconKey(actype);
  return ICON_URL[iconKey] ?? ICON_URL[FALLBACK];
}
