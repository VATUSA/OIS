// The public download page for the OIS desktop app (#347). Linked from the footer and reachable
// signed out — you download the app before you have any reason to be signed in.
//
// Installers live on GitHub Releases (the repo is public, so that is a free CDN and needs no auth).
// Asset filenames carry the version, so the exact URLs can't be hardcoded; this reads the latest
// release from the public API instead and falls back to the releases page if that call fails —
// offline, or GitHub's 60/hr unauthenticated rate limit — so the page is never a dead end.

import * as React from "react";
import {Apple, Download, Monitor, Terminal} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";

const RELEASES_URL = "https://github.com/VATUSA/OIS/releases";
const LATEST_API = "https://api.github.com/repos/VATUSA/OIS/releases/latest";

type Platform = {
  id: "macos" | "windows" | "linux";
  label: string;
  icon: typeof Apple;
  /** Extensions Tauri's bundler produces for this platform, best first. */
  extensions: string[];
};

const PLATFORMS: Platform[] = [
  { id: "macos", label: "macOS", icon: Apple, extensions: [".dmg"] },
  { id: "windows", label: "Windows", icon: Monitor, extensions: ["-setup.exe", ".msi"] },
  { id: "linux", label: "Linux", icon: Terminal, extensions: [".AppImage", ".deb"] },
];

/** Best guess at the visitor's OS, only ever used to decide what to put first. */
function detectPlatform(): Platform["id"] | undefined {
  if (typeof navigator === "undefined") return undefined;
  const ua = navigator.userAgent;
  if (/Mac/i.test(ua)) return "macos";
  if (/Win/i.test(ua)) return "windows";
  if (/Linux|X11/i.test(ua)) return "linux";
  return undefined;
}

type Release = { version: string; assets: Partial<Record<Platform["id"], string>> };

async function fetchLatestRelease(signal: AbortSignal): Promise<Release> {
  const response = await fetch(LATEST_API, { signal, headers: { Accept: "application/vnd.github+json" } });
  if (!response.ok) throw new Error(`GitHub returned ${response.status}`);

  const body = (await response.json()) as {
    tag_name?: string;
    assets?: { name: string; browser_download_url: string }[];
  };

  const assets: Release["assets"] = {};
  for (const platform of PLATFORMS) {
    for (const extension of platform.extensions) {
      const match = body.assets?.find((a) => a.name.endsWith(extension));
      if (match) {
        assets[platform.id] = match.browser_download_url;
        break;
      }
    }
  }

  return { version: body.tag_name ?? "", assets };
}

function PlatformRow({
  platform,
  href,
  primary,
}: {
  platform: Platform;
  href: string | undefined;
  primary: boolean;
}) {
  const Icon = platform.icon;
  return (
    <a
      href={href ?? RELEASES_URL}
      target="_blank"
      rel="noreferrer"
      className={[
        "flex items-center gap-3 rounded-md border border-line px-4 py-3 transition-colors",
        primary ? "bg-card hover:bg-chip" : "bg-panel-2 hover:bg-card",
      ].join(" ")}
    >
      <Icon className="size-5 shrink-0 text-ink-2" aria-hidden />
      <span className={primary ? "font-semibold text-ink" : "text-ink"}>{platform.label}</span>
      {primary && <span className="text-sm text-ink-3">Detected</span>}
      <Download className="ml-auto size-4 shrink-0 text-ink-3" aria-hidden />
    </a>
  );
}

export function DownloadPage() {
  usePageHeader({
    subtitle: "Install the OIS desktop app. It keeps itself up to date once installed.",
  });

  const [release, setRelease] = React.useState<Release | undefined>();
  const [failed, setFailed] = React.useState(false);
  const detected = React.useMemo(detectPlatform, []);

  React.useEffect(() => {
    const controller = new AbortController();
    fetchLatestRelease(controller.signal)
      .then(setRelease)
      .catch((error: unknown) => {
        if (!controller.signal.aborted) setFailed(error instanceof Error);
      });
    return () => controller.abort();
  }, []);

  // Detected platform first; the rest keep their declared order.
  const ordered = React.useMemo(
    () => [...PLATFORMS].sort((a, b) => Number(b.id === detected) - Number(a.id === detected)),
    [detected],
  );

  return (
    <div className="flex w-full max-w-2xl flex-col gap-4">
      <section className="flex flex-col gap-3">
        {release?.version && (
          <span className="font-mono text-sm text-ink-3">{release.version}</span>
        )}

        {failed && (
          <p className="text-sm text-ink-2">
            Couldn&apos;t reach GitHub to look up the latest build. Every installer is on the{" "}
            <a
              href={RELEASES_URL}
              target="_blank"
              rel="noreferrer"
              className="text-brand-ink underline underline-offset-2 hover:text-ink"
            >
              releases page
            </a>
            .
          </p>
        )}

        <div className="flex flex-col gap-2">
          {ordered.map((platform) => (
            <PlatformRow
              key={platform.id}
              platform={platform}
              href={release?.assets[platform.id]}
              primary={platform.id === detected}
            />
          ))}
        </div>

        <p className="text-sm text-ink-3">
          Looking for an older build, or a format not listed here? See{" "}
          <a
            href={RELEASES_URL}
            target="_blank"
            rel="noreferrer"
            className="text-brand-ink underline underline-offset-2 hover:text-ink"
          >
            all releases
          </a>
          .
        </p>
      </section>
    </div>
  );
}
