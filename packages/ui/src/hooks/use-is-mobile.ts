import * as React from "react";

const QUERY = "(max-width: 767px)";

/** True on phone-width viewports (below the Tailwind `md` breakpoint). */
export function useIsMobile(): boolean {
  const [mobile, setMobile] = React.useState(
    () => typeof window !== "undefined" && window.matchMedia(QUERY).matches,
  );
  React.useEffect(() => {
    const mq = window.matchMedia(QUERY);
    const on = () => setMobile(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return mobile;
}
