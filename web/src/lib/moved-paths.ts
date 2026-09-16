/**
 * Planning and Historical moved under Admin (#292). Old bookmarks keep working: the whole subtree,
 * params and query string included, maps onto the new prefix. Returns null for any other path.
 */
export function movedPath(pathname: string, searchStr: string): string | null {
  const m = /^\/(planning|historical)(\/.*)?$/.exec(pathname);
  return m ? `/admin/${m[1]}${m[2] ?? ""}${searchStr}` : null;
}
