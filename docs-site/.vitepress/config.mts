import {defineConfig} from "vitepress";

// https://vitepress.dev/reference/site-config
export default defineConfig({
  title: "OIS Docs",
  description: "Operational Information System — the VATUSA traffic-management platform.",
  lang: "en-US",
  cleanUrls: true,
  lastUpdated: true,
  head: [["link", { rel: "icon", href: "/favicon.svg" }]],
  themeConfig: {
    logo: "/favicon.svg",
    outline: { level: [2, 3] },
    search: { provider: "local" },
    nav: [
      { text: "Guide", link: "/introduction/what-is-ois" },
      { text: "Traffic Management", link: "/tmu/fcas" },
      { text: "Pilots", link: "/advisories/board" },
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
