/**
 * A plain upward-triangle glyph (data URI, white so deck's IconLayer mask tints it via getColor) for
 * "triangle" traffic style. The SVG needs explicit width/height — without them it has no intrinsic
 * size, so the browser rasterizes it to a 0×0 image and deck draws nothing.
 */
export const TRIANGLE_ICON =
  "data:image/svg+xml;base64," +
  btoa(
    '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 12 12"><path d="M6 0 L10.5 11 L6 8.5 L1.5 11 Z" fill="#fff"/></svg>',
  );
