<script setup lang="ts">
import * as monaco from "monaco-editor"

const props = defineProps<{
  content: string | null | undefined
  height?: string
  navigationBar?: boolean
}>()

const containerRef = ref<HTMLElement | null>(null)
let editor: monaco.editor.IStandaloneCodeEditor | null = null

// ── Theme IDs ─────────────────────────────────────────────────────────────
const DARK_THEME = "hyper-dark"
const LIGHT_THEME = "hyper-light"

function registerThemes() {
  // Dark — matches app surface palette
  monaco.editor.defineTheme(DARK_THEME, {
    base: "vs-dark",
    inherit: true,
    rules: [
      { token: "string.key.json",       foreground: "60a5fa" },
      { token: "string.value.json",      foreground: "34d399" },
      { token: "number.json",            foreground: "c084fc" },
      { token: "keyword.json",           foreground: "f97316" },
      { token: "keyword.null.json",      foreground: "94a3b8" },
      { token: "delimiter.bracket.json", foreground: "475569" },
      { token: "delimiter.colon.json",   foreground: "475569" },
      { token: "delimiter.comma.json",   foreground: "475569" },
    ],
    colors: {
      "editor.background":                  "#1a1d30",
      "editor.foreground":                  "#cbd5e1",
      "editorLineNumber.foreground":        "#434a6e",
      "editorLineNumber.activeForeground":  "#8e97b8",
      "editorCursor.foreground":            "#e29749",
      "editor.selectionBackground":         "#e2974926",
      "editor.inactiveSelectionBackground": "#e2974914",
      "editor.lineHighlightBackground":     "#3a406014",
      "editor.lineHighlightBorder":         "#00000000",
      "editorGutter.background":            "#1a1d30",
      "scrollbarSlider.background":         "#434a6e80",
      "scrollbarSlider.hoverBackground":    "#535c8699",
      "scrollbarSlider.activeBackground":   "#535c86cc",
      "minimap.background":                 "#161929",
      "minimapSlider.background":           "#434a6e60",
      "minimapSlider.hoverBackground":      "#535c8680",
      "minimapSlider.activeBackground":     "#535c86aa",
      "editorBracketMatch.background":      "#e2974920",
      "editorBracketMatch.border":          "#e29749",
    },
  })

  // Light — VS Code exact light colours
  monaco.editor.defineTheme(LIGHT_THEME, {
    base: "vs",
    inherit: true,
    rules: [
      { token: "string.key.json",       foreground: "0451a5" },
      { token: "string.value.json",      foreground: "a31515" },
      { token: "number.json",            foreground: "098658" },
      { token: "keyword.json",           foreground: "0000ff" },
      { token: "keyword.null.json",      foreground: "0000ff" },
      { token: "delimiter.bracket.json", foreground: "6c6c6c" },
      { token: "delimiter.colon.json",   foreground: "6c6c6c" },
      { token: "delimiter.comma.json",   foreground: "6c6c6c" },
    ],
    colors: {
      "editor.background":                  "#f8fafc",
      "editor.foreground":                  "#1e293b",
      "editorLineNumber.foreground":        "#94a3b8",
      "editorLineNumber.activeForeground":  "#475569",
      "editorCursor.foreground":            "#e29749",
      "editor.selectionBackground":         "#add6ff",
      "editor.inactiveSelectionBackground": "#e5ebf1",
      "editor.lineHighlightBackground":     "#f1f5f940",
      "editor.lineHighlightBorder":         "#00000000",
      "editorGutter.background":            "#f8fafc",
      "scrollbarSlider.background":         "#cbd5e180",
      "scrollbarSlider.hoverBackground":    "#94a3b899",
      "scrollbarSlider.activeBackground":   "#64748bcc",
      "minimap.background":                 "#f1f5f9",
      "minimapSlider.background":           "#cbd5e160",
      "minimapSlider.hoverBackground":      "#94a3b880",
      "minimapSlider.activeBackground":     "#64748baa",
      "editorBracketMatch.background":      "#e2974920",
      "editorBracketMatch.border":          "#e29749",
    },
  })
}

// ── Theme detection ───────────────────────────────────────────────────────
const isDark = () => document.documentElement.classList.contains("dark")
const currentTheme = () => (isDark() ? DARK_THEME : LIGHT_THEME)

let themeObserver: MutationObserver | null = null

// ── Content helpers ───────────────────────────────────────────────────────
const prettyJson = (s: string | null | undefined): string => {
  if (!s) return ""
  try {
    return JSON.stringify(JSON.parse(s), null, 2)
  } catch {
    return s ?? ""
  }
}

onMounted(() => {
  if (!containerRef.value) return

  registerThemes()

  editor = monaco.editor.create(containerRef.value, {
    value: prettyJson(props.content),
    language: "json",
    theme: currentTheme(),
    readOnly: true,
    minimap: { enabled: true, scale: 1 },
    scrollBeyondLastLine: false,
    folding: true,
    lineNumbers: "on",
    automaticLayout: true,
    fontSize: 11,
    lineHeight: 16,
    fontFamily: 'ui-monospace, "SFMono-Regular", Menlo, Monaco, Consolas, monospace',
    fontLigatures: false,
    renderWhitespace: "none",
    wordWrap: "off",
    scrollbar: {
      vertical: "auto",
      horizontal: "auto",
      verticalScrollbarSize: 5,
      horizontalScrollbarSize: 5,
    },
    overviewRulerLanes: 0,
    hideCursorInOverviewRuler: true,
    contextmenu: false,
    links: false,
    padding: { top: 6, bottom: 6 },
    glyphMargin: false,
    lineDecorationsWidth: 2,
  })

  // Watch <html class="dark"> toggling
  themeObserver = new MutationObserver(() => {
    editor?.updateOptions({ theme: currentTheme() })
  })
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["class"],
  })
})

watch(
  () => props.content,
  (val) => {
    const model = editor?.getModel()
    if (model) {
      monaco.editor.setModelLanguage(model, "json")
      model.setValue(prettyJson(val))
    }
  },
)

onBeforeUnmount(() => {
  themeObserver?.disconnect()
  themeObserver = null
  editor?.dispose()
  editor = null
})
</script>

<template>
  <div
    ref="containerRef"
    :style="{ height: height ?? '100%', width: '100%' }"
  />
</template>
