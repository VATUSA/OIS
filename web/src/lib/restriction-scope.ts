import type {Me} from "./auth";

/**
 * The ARTCCs whose restrictions this user hears about: their home facility and the ones they visit.
 * `null` means every ARTCC — a national (DCC) TMU reader.
 *
 * Without this, a controller who switched restriction alerts on was interrupted by every ground
 * stop, GDP, TMI and metering program published anywhere in the country (VATUSA/OIS#405). Deliberately
 * the same shape as `notifyFacilities` for FCA releases, so the two audiences can't drift apart.
 *
 * The national test is `tmu_national` rather than `server_admin`: a DCC controller holds
 * `tmu.program.read` nationally without being a server admin, and should still see the whole country.
 * Until the VATUSA profile has synced there is no facility to go on, so that is nothing rather than
 * everything — a silent alert is recoverable, a nationwide one trains people to ignore it.
 */
export function restrictionFacilities(me: Me | null | undefined): Set<string> | null {
  if (me?.tmu_national) return null;
  const v = me?.vatusa;
  return new Set([...(v?.home_facility ? [v.home_facility] : []), ...(v?.visits ?? [])]);
}

/**
 * Whether a restriction owned by `artccs` is in scope for `facilities`.
 *
 * A TMI carries two ARTCCs (requesting and providing) and matching either is enough — a centre cares
 * about a restriction whether it asked for it or is providing it. An ARTCC the facility map couldn't
 * resolve arrives as null and never matches, so an unresolvable restriction stays quiet.
 */
export function inRestrictionScope(
  facilities: Set<string> | null,
  artccs: (string | null | undefined)[],
): boolean {
  if (facilities == null) return true;
  return artccs.some((a) => a != null && facilities.has(a));
}
