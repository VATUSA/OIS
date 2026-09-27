import {rotateOnLaunch} from "@/lib/desktop-auth";
import {forgetOnClose, restoreWindows} from "@/lib/popout";

/**
 * Launch-time desktop work, in the one order that is safe (#346, #350).
 *
 * The session rotation goes first and is awaited. It deletes the token it presents, and every
 * window keeps the token it first read for as long as it lives. Restoring route windows alongside
 * it let a restored window read the keychain mid-rotation and cache a token the rotation then
 * deleted — that window stayed signed out (VATUSA/OIS#350 review).
 *
 * Restoring is not awaited: a window failing to reopen must not hold up first paint. Both steps
 * gate themselves to the desktop app's main window; `forgetOnClose` runs in route windows only.
 */
export async function launchDesktop(): Promise<void> {
  void forgetOnClose();
  await rotateOnLaunch();
  void restoreWindows();
}
