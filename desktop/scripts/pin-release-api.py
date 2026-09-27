#!/usr/bin/env python3
"""Point a release build of the desktop app at the production API (VATUSA/OIS#347).

A bundle has no container start to inject `window.__OIS_API_URL__` (the web image's `config.js`), so
the API base is baked in at build time through `VITE_OIS_API_URL`. Unset, it falls back to
`http://127.0.0.1:3000`: an installer that only ever talks to the user's own machine. The desktop
CSP (`app.security.csp.connect-src`, #346) must also allow that origin, REST and realtime alike, or
every call from the shipped app is blocked and looks exactly like a down server.

Usage: pin-release-api.py <api-origin> <tauri.conf.json> <github-env-file>

Fails closed. The origin must be a bare `https://host[:port]`: no path, no trailing slash, not
loopback, because the app appends `/api/v1/...` to it and a release must not ship pointed at a
developer's machine.
"""

import collections
import json
import sys
from urllib.parse import urlsplit


def fail(message: str) -> None:
    print(f"::error::{message}")
    sys.exit(1)


def main() -> None:
    if len(sys.argv) != 4:
        fail("usage: pin-release-api.py <api-origin> <tauri.conf.json> <github-env-file>")
    origin, conf_path, env_path = sys.argv[1].strip(), sys.argv[2], sys.argv[3]

    if not origin:
        fail("OIS_DESKTOP_API_URL is not set. Set the repo variable to the production API origin, "
             "e.g. https://api.example.org — see desktop/README.md.")
    parts = urlsplit(origin)
    if parts.scheme != "https" or not parts.hostname:
        fail(f"OIS_DESKTOP_API_URL must be an https:// origin, got {origin!r}.")
    if parts.path or parts.query or parts.fragment:
        fail(f"OIS_DESKTOP_API_URL must be a bare origin with no path or trailing slash, got {origin!r}.")
    if parts.hostname in ("localhost", "127.0.0.1", "::1"):
        fail(f"OIS_DESKTOP_API_URL points at loopback ({origin!r}); a release would only reach the user's own machine.")

    websocket = "wss://" + parts.netloc
    with open(conf_path) as f:
        conf = json.load(f, object_pairs_hook=collections.OrderedDict)
    csp = conf["app"]["security"]["csp"]
    sources = csp["connect-src"].split()
    for source in (origin, websocket):
        if source not in sources:
            sources.append(source)
    csp["connect-src"] = " ".join(sources)
    with open(conf_path, "w") as f:
        json.dump(conf, f, indent=2, ensure_ascii=False)
        f.write("\n")

    with open(env_path, "a") as f:
        f.write(f"VITE_OIS_API_URL={origin}\n")
    print(f"desktop API base set to {origin}; connect-src now allows {origin} and {websocket}")


if __name__ == "__main__":
    main()
