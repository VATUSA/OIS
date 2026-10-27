/**
 * The release stage from the repo-root `STAGE` file (#841), read at build time by
 * `web/vite.config.ts` and `docs-site/.vitepress/config.mts`. DOM-free and dependency-free, because
 * both build configs import it.
 */

const STAGES = ["alpha", "beta", "rc", "stable"] as const;
export type Stage = (typeof STAGES)[number];

const LABEL: Record<Stage, string> = { alpha: "Alpha", beta: "Beta", rc: "RC", stable: "" };

const TITLE: Record<Stage, string> = {
  alpha: "Alpha: expect rough edges and frequent changes",
  beta: "Beta: early access, still stabilizing",
  rc: "Release candidate",
  stable: "",
};

const TONE = { alpha: "warn", beta: "brand", rc: "neutral", stable: "neutral" } as const;

/** The `STAGE` file's contents as a stage. Blank or unknown fails the build rather than guessing. */
export function parseStage(raw: string): Stage {
  const value = raw.trim();
  const stage = STAGES.find((s) => s === value);
  if (!stage) {
    throw new Error(`STAGE must be one of ${STAGES.join(", ")}; got ${JSON.stringify(value)}`);
  }
  return stage;
}

/** "Alpha", "Beta", "RC"; empty for stable, which shows no label anywhere. */
export function stageLabel(stage: Stage): string {
  return LABEL[stage];
}

/** The chip's hover text. */
export function stageTitle(stage: Stage): string {
  return TITLE[stage];
}

/** The `StatusPill` tone for the chip. */
export function stageTone(stage: Stage): (typeof TONE)[Stage] {
  return TONE[stage];
}

/**
 * A desktop window title with the stage in it: "OIS" → "OIS (Alpha)", "OIS · TMU board" →
 * "OIS (Alpha) · TMU board", and a panel title such as "ZDC · metering" → "ZDC · metering (Alpha)".
 * Stable returns the title unchanged.
 */
export function stagedTitle(title: string, stage: Stage): string {
  if (stage === "stable") return title;
  const tag = `(${LABEL[stage]})`;
  if (title.startsWith("OIS · ")) return `OIS ${tag}${title.slice(3)}`;
  return `${title} ${tag}`;
}

/** The docs-site nav entry: "v0.4.2 · Alpha", or "v0.4.2" for stable. */
export function navVersionText(version: string, stage: Stage): string {
  return stage === "stable" ? `v${version}` : `v${version} · ${LABEL[stage]}`;
}
