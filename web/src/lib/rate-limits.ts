import type {components} from "@ois/api-client";

export type CredentialUsage = components["schemas"]["CredentialUsageBody"];

/**
 * What an admin's edit to a credential's rate limit should send (#611), or `undefined` to send
 * nothing: empty clears the override (back to the deployment default), a positive whole number sets
 * it, and anything else — zero, negative, fractional, junk — is not a request at all.
 */
export function parseLimit(input: string): number | null | undefined {
  const text = input.trim();
  if (text === "") return null;
  if (!/^\d+$/.test(text)) return undefined;
  const value = Number(text);
  return value > 0 ? value : undefined;
}

/** "120 this hour · 2,400 / 24 h · 3 refused" — a credential's recent volume, summed across replicas. */
export function usageLabel(usage: CredentialUsage): string {
  const n = (v: number) => v.toLocaleString("en-US");
  const refused = usage.refused_last_day ? ` · ${n(usage.refused_last_day)} refused` : "";
  return `${n(usage.requests_this_hour)} this hour · ${n(usage.requests_last_day)} / 24 h${refused}`;
}
