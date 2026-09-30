import * as React from "react";

import {ois} from "@/lib/api";

/**
 * An event's banner, fetched through the API rather than pointed at directly (#429).
 *
 * Banners are third-party URLs mirrored from VATUSA, and organisers use whatever host they like —
 * five unrelated ones are already in the data. The bundled desktop app runs under a CSP whose
 * `img-src` cannot name them all without becoming `https:`, so a raw `<img src={banner_image_url}>`
 * is simply blocked there. Dev never shows it: `devCsp` is `null`, so the policy only exists in a
 * build nobody looks at until it ships.
 *
 * So the bytes come from `/api/v1/events/{id}/banner` and are handed to the page as a `blob:` URL,
 * which `img-src` already allows. That also means the request carries the bearer token the endpoint
 * requires — an `<img>` would have sent no credentials at all.
 *
 * Renders nothing until the image is there, and nothing at all if it never arrives: a missing banner
 * is decoration, and every one of these pages is readable without it.
 */
export function EventBanner({
  eventId,
  className,
  fallback = null,
}: {
  eventId: number;
  className?: string;
  /** Shown while the image is in flight and if it never arrives — keeps a card's shape stable. */
  fallback?: React.ReactNode;
}) {
  const [src, setSrc] = React.useState<string>();

  React.useEffect(() => {
    let alive = true;
    let objectUrl: string | undefined;

    void (async () => {
      try {
        const {data} = await ois.GET("/api/v1/events/{id}/banner", {
          params: {path: {id: eventId}},
          parseAs: "blob",
        });
        if (!data) return;
        // Revoking is not optional: an object URL pins its blob in memory until it is released, and
        // these pages remount as the user moves between events.
        objectUrl = URL.createObjectURL(data as Blob);
        if (alive) setSrc(objectUrl);
        else URL.revokeObjectURL(objectUrl);
      } catch {
        // No banner is a normal outcome — the event may have none, or its host may be down.
      }
    })();

    return () => {
      alive = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [eventId]);

  if (!src) return <>{fallback}</>;
  return <img src={src} alt="" className={className} />;
}
