// The download page for the OIS desktop app (#347), reachable signed out — you download the app
// before you have any reason to be signed in — and from the sidebar's User group once signed in
// (#534).
//
// Each row links to `/api/v1/public/desktop/download/{platform}` on the API origin, which resolves
// the current release's installer server-side and redirects to it. That replaced a browser-side call to
// `api.github.com` (#534), which failed in two ordinary situations and silently degraded every row
// to the generic releases page:
//
//   * the host is absent from the desktop app's CSP `connect-src`, so inside the app it was blocked;
//   * GitHub's unauthenticated limit is 60 requests/hour per IP.
//
// There is no fetch here any more. A row's destination no longer depends on a call succeeding, so
// "it looks like a direct download but isn't" is no longer a state this page can be in.

import * as React from "react";
import {Apple, Download, Monitor, Terminal} from "lucide-react";

import {isTauri} from "@/lib/platform";
import {usePageHeader} from "@/components/shell/page-meta";
import {API_BASE} from "@/lib/api";

/** Kept as an explicit secondary link, never as a silent substitute for a platform row. */
const RELEASES_URL = "https://github.com/VATUSA/OIS/releases";

type Platform = {
  id: "macos" | "windows" | "linux";
  label: string;
  icon: typeof Apple;
};

const PLATFORMS: Platform[] = [
  { id: "macos", label: "macOS", icon: Apple },
  { id: "windows", label: "Windows", icon: Monitor },
  { id: "linux", label: "Linux", icon: Terminal },
];

/**
 * The server-side resolver. Which asset a platform maps to is decided in `handlers::desktop`.
 *
 * Built from `API_BASE` exactly as the typed client builds its URLs (trailing slash stripped, then
 * the path appended), because in production the API is a different host from this page (#738). A
 * root-relative path resolved against the web origin, where nginx answers with the SPA shell. An
 * empty `API_BASE` (same-origin deployment) leaves the relative path, which is then correct.
 */
const downloadHref = (platform: Platform["id"]) =>
  `${API_BASE.replace(/\/$/, "")}/api/v1/public/desktop/download/${platform}`;

/** Best guess at the visitor's OS, only ever used to decide what to put first. */
function detectPlatform(): Platform["id"] | undefined {
  if (typeof navigator === "undefined") return undefined;
  const ua = navigator.userAgent;
  if (/Mac/i.test(ua)) return "macos";
  if (/Win/i.test(ua)) return "windows";
  if (/Linux|X11/i.test(ua)) return "linux";
  return undefined;
}

function PlatformRow({ platform, primary }: { platform: Platform; primary: boolean }) {
  const Icon = platform.icon;
  return (
    <a
      href={downloadHref(platform.id)}
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

  const detected = React.useMemo(detectPlatform, []);

  // Detected platform first; the rest keep their declared order.
  const ordered = React.useMemo(
    () => [...PLATFORMS].sort((a, b) => Number(b.id === detected) - Number(a.id === detected)),
    [detected],
  );

  return (
    <div className="flex w-full max-w-2xl flex-col gap-4">
      <section className="flex flex-col gap-3">
        {/*
          Inside the desktop app (#751) the platform rows have no job: the user already has the app,
          and it updates itself. A bare link there would also navigate the main window off-app, to
          the installer redirect, with no way back.
        */}
        {isTauri() ? (
          <p className="text-sm text-ink-3">
            You&apos;re already running the desktop app, and it keeps itself up to date. To install it
            on another computer, open OIS in a browser there and come back to this page.
          </p>
        ) : (
          <>
            <div className="flex flex-col gap-2">
              {ordered.map((platform) => (
                <PlatformRow
                  key={platform.id}
                  platform={platform}
                  primary={platform.id === detected}
                />
              ))}
            </div>

            {/*
              The installers are deliberately not OS-code-signed yet — `.github/workflows/release.yml`
              documents the decision — so the first launch shows a publisher warning. Saying so here is
              the difference between a user thinking the download is broken and knowing what to click.
              This note goes away with the signing work (#535).
            */}
            <p className="text-sm text-ink-3">
              The installers aren&apos;t signed yet, so the first launch shows a warning about an
              unidentified developer. On macOS, open it from Finder with <strong>right-click → Open</strong>;
              on Windows, choose <strong>More info → Run anyway</strong>. Updates after that are
              automatic and verified.
            </p>
          </>
        )}

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
