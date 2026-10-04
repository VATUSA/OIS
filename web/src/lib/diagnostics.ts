/**
 * "Send diagnostics" (#629): what the webview contributes to a report, and the call that sends it.
 *
 * The desktop's Rust does the rest — platform facts, the log files, redaction, compression and the
 * upload with the token from its own store (`desktop/src-tauri/src/diagnostics.rs`) — so neither the
 * log files nor the credential pass through here. Who sent it is the server's to know from the
 * session; nothing below claims an identity.
 */

import {lastUpdateCheck} from "./desktop-update";
import {logTail} from "./logger";
import {API_BASE} from "./api";
import {capabilities, invokeDesktop, windowLabel} from "./platform";
import {realtimeHistory} from "./realtime";
import {webgl2Available} from "./webgl";

/** Everything the webview knows that the report should carry. */
export async function diagnosticsContext(route: string) {
  return {
    window_label: (await windowLabel()) ?? "",
    route,
    web_version: __APP_VERSION__,
    capabilities: capabilities(),
    webgl2: webgl2Available(),
    update: lastUpdateCheck(),
    realtime: realtimeHistory(),
    user_agent: navigator.userAgent,
    log_tail: logTail(),
  };
}

/**
 * Sends a report and resolves to its id. Rejects with a message fit to show the user — the Rust
 * command words its own failures (a 413, a 429, a lapsed session, no network).
 */
export async function sendDiagnostics(note: string): Promise<string> {
  const context = await diagnosticsContext(window.location.pathname);
  return invokeDesktop<string>("send_diagnostics", {
    apiBase: API_BASE || window.location.origin,
    note,
    context,
  });
}
