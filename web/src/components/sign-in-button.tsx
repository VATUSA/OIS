import {Button, useToast} from "@ois/ui";
import {LogIn} from "lucide-react";

import {useLogin, useSignInPending} from "@/lib/auth";

/**
 * The one way to start sign-in (#428).
 *
 * It exists as a component rather than a call to `login()` because the bare function is not enough on
 * desktop: `login()` there resolves *in place* instead of navigating away, and `fetchMe` has already
 * cached the 401 as `null` for `staleTime`, so nothing re-renders and the app sits on the sign-in
 * button after a successful sign-in. Only `useLogin()`'s `["me"]` invalidation moves it, and that is
 * the frontend's only one.
 *
 * Two call sites had drifted onto the bare `login()` and each showed that bug — which is why this is
 * shared rather than a pattern to copy a fourth time. It also carries the two things the bare call
 * discards: desktop sign-in can fail in ways the web flow never could (the loopback listener failing
 * to bind, timing out, or having its code refused), and a second click while one is in flight used to
 * hit `EADDRINUSE` and wedge the user.
 */
export function SignInButton({
  size = "sm",
  className,
  iconOnly = false,
}: {
  size?: React.ComponentProps<typeof Button>["size"];
  className?: string;
  /** For the collapsed sidebar rail, where there is no room for the label. */
  iconOnly?: boolean;
}) {
  const signIn = useLogin();
  // App-wide, not this button's own: two of these can be on screen at once and a second `begin_login`
  // takes the loopback port from the first (#428 review).
  const pending = useSignInPending();
  const toast = useToast();

  const startSignIn = () => {
    signIn.mutate(undefined, {
      onError: (error) =>
        toast.error("Sign-in failed", {
          description: error instanceof Error ? error.message : "Please try again.",
        }),
    });
  };

  if (iconOnly) {
    return (
      <Button
        size="icon"
        aria-label="Sign in with VATSIM"
        onClick={startSignIn}
        disabled={pending}
        className={className}
      >
        <LogIn />
      </Button>
    );
  }

  return (
    <Button size={size} onClick={startSignIn} disabled={pending} className={className}>
      {pending ? "Signing in..." : "Sign in with VATSIM"}
    </Button>
  );
}
