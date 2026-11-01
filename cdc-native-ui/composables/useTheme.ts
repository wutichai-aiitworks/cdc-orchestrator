import { useDark, useToggle } from "@vueuse/core"

export const useTheme = () => {
  const isDark = useDark({
    selector: "html",
    attribute: "class",
    valueDark: "dark",
    valueLight: "",
    storageKey: "ds-color-scheme",
    initialValue: "light",
  })

  const toggle = useToggle(isDark)

  return { isDark, toggle }
}
