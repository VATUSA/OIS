import {readFileSync} from "node:fs";

import {defineConfig} from "vitepress";

// Full version shown in the nav, e.g. "1.0.1-a1b2c3d". CI passes the finished string as OIS_VERSION;
// locally we read the repo-root VERSION file (the single programmer-controlled base).
function version(): string {
  if (process.env.OIS_VERSION) return process.env.OIS_VERSION;
  try {
    return readFileSync(new URL("../../VERSION", import.meta.url), "utf8").trim();
  } catch {
    return "dev";
  }
}

// https://vitepress.dev/reference/site-config
export default defineConfig({
  title: "OIS Docs",
  description: "Operational Information System — the VATUSA traffic-management platform.",
  lang: "en-US",
  cleanUrls: true,
  // Note: no `lastUpdated` — it shells out to `git`, which isn't present in the Docker build
  // (and `.git` is excluded from the build context), so it would fail CI.
  head: [["link", { rel: "icon", href: "/favicon.svg" }]],
  themeConfig: {
    logo: "/favicon.svg",
    outline: { level: [2, 3] },
    search: { provider: "local" },
    nav: [
      { text: "Guide", link: "/introduction/what-is-ois" },
      { text: "Traffic Management", link: "/tmu/fcas" },
      { text: "Pilots", link: "/advisories/board" },
      { text: `v${version()}`, link: "https://github.com/VATUSA/OIS" },
    ],
    sidebar: [
      {
        text: "Introduction",
        items: [
          { text: "What is OIS", link: "/introduction/what-is-ois" },
          { text: "Signing in", link: "/introduction/signing-in" },
          { text: "Your profile & VATUSA", link: "/introduction/profile" },
        ],
      },
      {
        text: "Using the map",
        items: [
          { text: "The flow map", link: "/map/overview" },
          { text: "Live traffic", link: "/map/traffic" },
          { text: "The facility map", link: "/map/facility-map" },
          { text: "The ATC layer", link: "/map/atc" },
          { text: "Finding a flight", link: "/map/flight-search" },
        ],
      },
      {
        text: "Traffic management",
        items: [
          { text: "Flow Constrained Areas", link: "/tmu/fcas" },
          { text: "Ground Delay Programs", link: "/tmu/gdp" },
          { text: "Runway Balancer", link: "/tmu/runway-balancer" },
          { text: "Release times (CFR)", link: "/tmu/releases" },
          { text: "Departure scheduling (IDST)", link: "/tmu/idst" },
          { text: "Restrictions & programs", link: "/tmu/restrictions" },
        ],
      },
      {
        text: "Advisories & pilots",
        items: [
          { text: "Advisories board", link: "/advisories/board" },
          { text: "Flight lookup", link: "/advisories/pilot" },
        ],
      },
      {
        text: "Reference",
        items: [
          { text: "Roles & permissions", link: "/reference/permissions" },
          { text: "Glossary", link: "/reference/glossary" },
          { text: "FAQ", link: "/reference/faq" },
        ],
      },
    ],
    docFooter: { prev: true, next: true },
  },
});
