/**
 * Clicking a sidebar link for the page you are already on is a no-op navigation, so
 * pages that keep view state in refs (an open editor, a selection) never reset.
 * The layout reports those clicks with `reselect(path)`; pages opt in with
 * `onNavReselect(path, handler)` and restore their landing view.
 */
export function useNavReselect() {
  const ticks = useState<Record<string, number>>("nav-reselect", () => ({}))

  const reselect = (path: string) => {
    ticks.value = { ...ticks.value, [path]: (ticks.value[path] ?? 0) + 1 }
  }

  const onNavReselect = (path: string, handler: () => void) => {
    watch(
      () => ticks.value[path],
      (count, previous) => {
        if (count && count !== previous) handler()
      },
    )
  }

  return { reselect, onNavReselect }
}
