export default defineEventHandler((event) => {
  setResponseHeader(event, "content-type", "text/html; charset=utf-8")
  setResponseHeader(event, "cache-control", "no-cache")
  setResponseHeader(
    event,
    "content-security-policy",
    "default-src 'self'; script-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; style-src 'self' 'unsafe-inline'; img-src 'self' data: https:; font-src 'self' data: https:; connect-src 'self' https:; base-uri 'self'; form-action 'none'"
  )

  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Hyper CDC API Reference</title>
  <style>html, body { margin: 0; min-height: 100%; }</style>
</head>
<body>
  <script id="api-reference" data-url="/openapi.json"></script>
  <script>
    const dark = new URLSearchParams(window.location.search).get("theme") === "dark"
    document.getElementById("api-reference").dataset.configuration = JSON.stringify({
      spec: { url: "/openapi.json" },
      theme: "default",
      layout: "modern",
      darkMode: dark,
      forceDarkModeState: dark ? "dark" : "light",
      hideDarkModeToggle: true,
      showSidebar: true,
      hideModels: false,
      hideClientButton: false,
      hideTestRequestButton: false,
      showOperationId: true,
      documentDownloadType: "both",
      defaultHttpClient: { targetKey: "shell", clientKey: "curl" },
      metaData: {
        title: "Hyper CDC API Reference",
        description: "Hyper CDC Orchestrator control-plane API"
      }
    })
  </script>
  <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
</body>
</html>`
})
