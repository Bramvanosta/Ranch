/**
 * A window opened by `open_window` boots at index.html with the
 * route it was asked to show in `window.__RANCH_OPEN_ROUTE__` (an
 * initialization script). Put it in the address before the router reads it.
 * Imported first by `main.tsx`, so it runs before the router is created.
 */
const opened = (window as { __RANCH_OPEN_ROUTE__?: unknown }).__RANCH_OPEN_ROUTE__
if (typeof opened === "string" && opened.startsWith("/") && !opened.startsWith("//")) {
  window.history.replaceState(null, "", opened)
}
