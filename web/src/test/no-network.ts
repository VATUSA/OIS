// Vitest setup: no web test talks to the network (VATUSA/OIS#387).
//
// The generated API client (`@/lib/api` → openapi-fetch) captures `fetch` when its module loads, so
// a test that stubs `fetch` itself is too late — the DOM tests' queries and mutations used to go
// straight to whatever listened on API_BASE, including PUTs to a developer's running backend. A setup
// file runs before any test module is imported, so the `fetch` installed here is the one the client
// captures. Every request fails in-process and at once — which is what CI, with no backend, already
// gave these tests — so none of them depends on the machine it runs on.
globalThis.fetch = (input: RequestInfo | URL) => {
  const url = input instanceof Request ? input.url : String(input);
  return Promise.reject(new Error(`network disabled in tests: ${url}`));
};
